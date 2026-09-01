//! Finding a plan by what it says.
//!
//! Lexical by default and by design (D8): FTS5 + BM25, re-ranked by recency and by
//! whether the hit is in the repo you are standing in. No model, no download, and the
//! same query gives the same answer every time.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{Plan, Status};
use crate::store::Store;
use crate::util::now;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub plan: Plan,
    /// `plan`, `section`, `slice`, `decision`, `gotcha`, `question` or `log`.
    pub kind: String,
    /// The slice key, section title or similar - where in the plan the hit is.
    pub reference: String,
    /// The matching text, trimmed to something readable.
    pub snippet: String,
    pub score: f64,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Rank hits in this repo above the rest rather than excluding others.
    pub prefer_repo: Option<i64>,
    /// Only this repo.
    pub only_repo: Option<i64>,
    pub statuses: Vec<Status>,
    pub limit: usize,
}

impl Store {
    /// Rebuild the index from the tables it summarises. Cheap at this scale, and a
    /// rebuilt index is never stale, which a trigger-maintained one eventually is.
    pub fn reindex(&mut self) -> Result<usize> {
        let ts = now();
        self.db_mut().write(|tx| {
            tx.execute("DELETE FROM search", [])?;
            let mut rows = 0usize;

            for (sql, kind) in [
                (
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     SELECT COALESCE(p.summary,'') || ' ' || p.slug || ' ' || COALESCE(p.ticket_key,''),
                            p.id, 'plan', p.slug, p.title FROM plan p",
                    "plan",
                ),
                (
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     SELECT s.body, s.plan_id, 'section', s.title, s.title FROM plan_section s",
                    "section",
                ),
                (
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     SELECT s.scope_md || ' ' || COALESCE(s.demo_md,'') || ' ' || COALESCE(s.branch,''),
                            s.plan_id, 'slice', s.key, s.title FROM slice s",
                    "slice",
                ),
                (
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     SELECT d.body, d.plan_id, 'decision', d.key, d.title FROM decision d",
                    "decision",
                ),
                (
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     SELECT g.body, g.plan_id, 'gotcha', g.title, g.title FROM gotcha g",
                    "gotcha",
                ),
                (
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     SELECT q.body, q.plan_id, 'question', CAST(q.id AS TEXT), '' FROM question q",
                    "question",
                ),
                (
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     SELECT l.body, l.plan_id, 'log', SUBSTR(l.at, 1, 10), '' FROM log l
                     WHERE l.kind IN ('progress','verification','blocker','handoff')",
                    "log",
                ),
            ] {
                let _ = kind;
                rows += tx.execute(sql, [])?;
            }

            tx.execute(
                "INSERT INTO search_state (id, rebuilt_at, rows) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET rebuilt_at = excluded.rebuilt_at, rows = excluded.rows",
                params![ts, rows as i64],
            )?;
            Ok(rows)
        })
    }

    pub fn search(&self, query: &str, opts: &SearchOptions) -> Result<Vec<Hit>> {
        let match_query = to_match_query(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let limit = if opts.limit == 0 { 20 } else { opts.limit };

        let conn = self.db().conn();
        let mut stmt = conn.prepare(
            "SELECT s.plan_id, s.kind, s.ref,
                    snippet(search, 0, '', '', '…', 14) AS snip,
                    bm25(search, 1.0, 0.0, 0.0, 0.0, 2.0) AS rank
             FROM search s
             WHERE search MATCH ?1
             ORDER BY rank
             LIMIT 400",
        )?;

        let raw = stmt.query_map([&match_query], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
            ))
        })?;

        let mut hits: Vec<Hit> = Vec::new();
        for row in raw {
            let (plan_id, kind, reference, snippet, rank) = row?;
            let Ok(plan) = self.get_plan(plan_id) else {
                continue;
            };
            if let Some(only) = opts.only_repo {
                if plan.repo_id != only {
                    continue;
                }
            }
            if !opts.statuses.is_empty() && !opts.statuses.contains(&plan.status) {
                continue;
            }

            // bm25 is negative, better being more negative; flip it so bigger is better
            // and then apply the two signals that matter beyond the words themselves.
            let mut score = -rank;
            if Some(plan.repo_id) == opts.prefer_repo {
                score += 2.0;
            }
            if matches!(plan.status, Status::Active | Status::InReview) {
                score += 1.0;
            } else if plan.status.is_terminal() {
                score -= 1.0;
            }
            score += recency_bonus(&plan.updated_at);

            hits.push(Hit {
                plan,
                kind,
                reference,
                snippet: snippet.split_whitespace().collect::<Vec<_>>().join(" "),
                score,
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // One line per plan per kind is enough to judge a hit; more is just noise.
        let mut seen: Vec<(i64, String)> = Vec::new();
        hits.retain(|h| {
            let key = (h.plan.id, h.kind.clone());
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        });
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn search_rows(&self) -> Result<i64> {
        Ok(self
            .db()
            .conn()
            .query_row(
                "SELECT COALESCE(rows, 0) FROM search_state WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0))
    }
}

/// Turn what a person typed into an FTS5 query. Every term is a prefix match and the
/// terms are OR-ed, so a partly-remembered phrase still finds the plan; ranking sorts
/// out which hit is best rather than the query excluding everything.
fn to_match_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() > 1)
        .map(|t| format!("\"{}\"*", t.to_lowercase()))
        .collect();
    terms.join(" OR ")
}

/// A plan touched this week outranks one untouched for a month.
fn recency_bonus(updated_at: &str) -> f64 {
    let today = now();
    let (a, b) = (&updated_at[..updated_at.len().min(10)], &today[..10]);
    if a == b {
        1.5
    } else if a[..7.min(a.len())] == b[..7] {
        0.75
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_becomes_a_prefix_or_query() {
        assert_eq!(to_match_query("date range"), "\"date\"* OR \"range\"*");
        // Punctuation is dropped rather than breaking the query syntax.
        assert_eq!(
            to_match_query("canvas-editor!"),
            "\"canvas\"* OR \"editor\"*"
        );
        assert_eq!(to_match_query("a"), "");
        assert_eq!(to_match_query(""), "");
    }
}
