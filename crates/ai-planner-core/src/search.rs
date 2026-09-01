//! Finding a plan by what it says.
//!
//! Lexical by default (D8): FTS5 + BM25, re-ranked by recency and by whether the hit is
//! in the repo you are standing in. When a local model is installed and `aip embed` has
//! been run, the two rankings are fused so a query can match meaning as well as words.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::embed::{self, Embedder};
use crate::error::Result;
use crate::model::{Plan, Status};
use crate::store::Store;
use crate::util::{now, sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub plan: Plan,
    /// `plan`, `section`, `slice`, `decision`, `gotcha`, `question` or `log`.
    pub kind: String,
    /// The slice key, section title, or the date of a log note - where in the plan
    /// the hit is, in the terms the plan itself uses.
    pub reference: String,
    pub snippet: String,
    pub score: f64,
    /// `lexical`, `semantic`, or `both`.
    pub matched: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Rank hits in this repo above the rest rather than excluding others.
    pub prefer_repo: Option<i64>,
    /// Only this repo.
    pub only_repo: Option<i64>,
    pub statuses: Vec<Status>,
    pub limit: usize,
    /// Skip the semantic leg even when embeddings exist.
    pub lexical_only: bool,
}

/// One addressable piece of a plan. Both indexes are built from these, so the lexical
/// and semantic legs can never disagree about what text belongs to what.
#[derive(Debug, Clone)]
pub struct IndexUnit {
    pub plan_id: i64,
    pub kind: &'static str,
    pub reference: String,
    pub title: String,
    pub body: String,
}

impl IndexUnit {
    /// What the model sees. Titles are repeated into the text because a heading is the
    /// most informative sentence a section has.
    pub fn embed_text(&self) -> String {
        let body: String = self
            .body
            .split_whitespace()
            .take(220)
            .collect::<Vec<_>>()
            .join(" ");
        if self.title.trim().is_empty() {
            body
        } else {
            format!("{}. {body}", self.title.trim())
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct EmbedStats {
    pub embedded: usize,
    pub unchanged: usize,
    pub removed: usize,
}

impl Store {
    /// Everything worth searching, in one pass.
    pub fn index_units(&self) -> Result<Vec<IndexUnit>> {
        let conn = self.db().conn();
        let mut units = Vec::new();

        let mut push = |sql: &str, kind: &'static str| -> Result<()> {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |r| {
                Ok(IndexUnit {
                    plan_id: r.get(0)?,
                    kind,
                    reference: r.get(1)?,
                    title: r.get(2)?,
                    body: r.get(3)?,
                })
            })?;
            for row in rows {
                units.push(row?);
            }
            Ok(())
        };

        push(
            "SELECT id, slug, title,
                    COALESCE(summary,'') || ' ' || slug || ' ' || COALESCE(ticket_key,'')
             FROM plan",
            "plan",
        )?;
        push(
            "SELECT plan_id, title, title, body FROM plan_section WHERE TRIM(body) <> ''",
            "section",
        )?;
        push(
            "SELECT plan_id, key, title,
                    scope_md || ' ' || COALESCE(demo_md,'') || ' ' || COALESCE(branch,'')
             FROM slice",
            "slice",
        )?;
        push("SELECT plan_id, key, title, body FROM decision", "decision")?;
        push("SELECT plan_id, title, title, body FROM gotcha", "gotcha")?;
        push(
            "SELECT plan_id, CAST(id AS TEXT), '', body FROM question",
            "question",
        )?;
        // Keyed by id, not by date: several notes land on one day, and a colliding
        // key makes them overwrite each other's vector on every rebuild.
        push(
            "SELECT plan_id, CAST(id AS TEXT), SUBSTR(at, 1, 10), body FROM log
             WHERE kind IN ('progress','verification','blocker','handoff')",
            "log",
        )?;

        Ok(units)
    }

    /// Rebuild the lexical index. Cheap at this scale, and a rebuilt index is never
    /// stale, which a trigger-maintained one eventually is.
    pub fn reindex(&mut self) -> Result<usize> {
        let units = self.index_units()?;
        let ts = now();
        self.db_mut().write(|tx| {
            tx.execute("DELETE FROM search", [])?;
            for u in &units {
                tx.execute(
                    "INSERT INTO search (body, plan_id, kind, ref, title)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![u.body, u.plan_id, u.kind, u.reference, u.title],
                )?;
            }
            tx.execute(
                "INSERT INTO search_state (id, rebuilt_at, rows) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET rebuilt_at = excluded.rebuilt_at,
                                               rows = excluded.rows",
                params![ts, units.len() as i64],
            )?;
            Ok(())
        })?;
        Ok(units.len())
    }

    /// Build or refresh the semantic index. Text that has not changed is not re-embedded,
    /// so a second run over an unchanged database costs one query and no model time.
    pub fn embed_all(&mut self, embedder: &dyn Embedder) -> Result<EmbedStats> {
        let units = self.index_units()?;
        let model = embedder.id().to_string();

        let existing: HashMap<(i64, String, String), String> = {
            let conn = self.db().conn();
            let mut stmt =
                conn.prepare("SELECT plan_id, kind, ref, sha FROM embedding WHERE model = ?1")?;
            let rows = stmt.query_map([&model], |r| {
                Ok((
                    (
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ),
                    r.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<HashMap<_, _>>>()?
        };

        let mut stale: Vec<(&IndexUnit, String, String)> = Vec::new();
        let mut unchanged = 0usize;
        let mut live: Vec<(i64, String, String)> = Vec::new();
        for unit in &units {
            let text = unit.embed_text();
            if text.trim().is_empty() {
                continue;
            }
            let sha = sha256(text.as_bytes());
            let key = (unit.plan_id, unit.kind.to_string(), unit.reference.clone());
            live.push(key.clone());
            match existing.get(&key) {
                Some(seen) if seen == &sha => unchanged += 1,
                _ => stale.push((unit, text, sha)),
            }
        }

        let mut embedded = 0usize;
        // Batched so the model is called a few times rather than thousands.
        for chunk in stale.chunks(64) {
            let texts: Vec<&str> = chunk.iter().map(|(_, t, _)| t.as_str()).collect();
            let vectors = embedder.embed(&texts)?;
            let dims = embedder.dims() as i64;
            let ts = now();
            let model = model.clone();
            self.db_mut().write(|tx| {
                for ((unit, text, sha), vector) in chunk.iter().zip(vectors.iter()) {
                    let snippet: String = text
                        .split_whitespace()
                        .take(28)
                        .collect::<Vec<_>>()
                        .join(" ");
                    tx.execute(
                        "INSERT INTO embedding
                           (plan_id, kind, ref, sha, model, dims, vector, title, snippet, built_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                         ON CONFLICT(plan_id, kind, ref, model) DO UPDATE SET
                             sha = excluded.sha, dims = excluded.dims, vector = excluded.vector,
                             title = excluded.title, snippet = excluded.snippet,
                             built_at = excluded.built_at",
                        params![
                            unit.plan_id,
                            unit.kind,
                            unit.reference,
                            sha,
                            model,
                            dims,
                            embed::to_blob(vector),
                            unit.title,
                            snippet,
                            ts
                        ],
                    )?;
                }
                Ok(())
            })?;
            embedded += chunk.len();
        }

        // Drop vectors for text that no longer exists, or the index answers with
        // sections that were deleted.
        let mut removed = 0usize;
        let orphans: Vec<i64> = {
            let conn = self.db().conn();
            let mut stmt =
                conn.prepare("SELECT id, plan_id, kind, ref FROM embedding WHERE model = ?1")?;
            let rows = stmt.query_map([&model], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    (
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ),
                ))
            })?;
            rows.filter_map(|row| row.ok())
                .filter(|(_, key)| !live.contains(key))
                .map(|(id, _)| id)
                .collect()
        };
        if !orphans.is_empty() {
            self.db_mut().write(|tx| {
                for id in &orphans {
                    tx.execute("DELETE FROM embedding WHERE id = ?1", [id])?;
                    removed += 1;
                }
                Ok(())
            })?;
        }

        let (dims, total, ts) = (embedder.dims() as i64, live.len() as i64, now());
        let model_for_state = model.clone();
        self.db_mut().write(|tx| {
            tx.execute(
                "INSERT INTO embedding_state (id, model, dims, rows, built_at)
                 VALUES (1, ?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET model = excluded.model, dims = excluded.dims,
                                               rows = excluded.rows, built_at = excluded.built_at",
                params![model_for_state, dims, total, ts],
            )?;
            Ok(())
        })?;

        Ok(EmbedStats {
            embedded,
            unchanged,
            removed,
        })
    }

    pub fn clear_embeddings(&mut self) -> Result<usize> {
        self.db_mut().write(|tx| {
            let n = tx.execute("DELETE FROM embedding", [])?;
            tx.execute("DELETE FROM embedding_state", [])?;
            Ok(n)
        })
    }

    /// `(model, rows)` when a semantic index exists.
    pub fn embedding_state(&self) -> Result<Option<(String, i64)>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .db()
            .conn()
            .query_row(
                "SELECT model, rows FROM embedding_state WHERE id = 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
            .filter(|(_, rows)| *rows > 0))
    }

    pub fn search(&self, query: &str, opts: &SearchOptions) -> Result<Vec<Hit>> {
        self.search_with(query, opts, None)
    }

    /// The semantic leg runs only when an embedder is supplied *and* the vectors in the
    /// database came from the same model - two vector spaces must never be compared.
    pub fn search_with(
        &self,
        query: &str,
        opts: &SearchOptions,
        embedder: Option<&dyn Embedder>,
    ) -> Result<Vec<Hit>> {
        let limit = if opts.limit == 0 { 20 } else { opts.limit };
        let mut ranked: HashMap<(i64, String, String), Candidate> = HashMap::new();

        for (rank, row) in self.lexical(query)?.into_iter().enumerate() {
            let key = (row.plan_id, row.kind.clone(), row.reference.clone());
            let entry = ranked.entry(key).or_insert_with(|| Candidate::new(row));
            entry.lexical_rank = Some(rank);
        }

        if !opts.lexical_only {
            if let Some(embedder) = embedder {
                for (rank, row) in self.semantic(query, embedder)?.into_iter().enumerate() {
                    let key = (row.plan_id, row.kind.clone(), row.reference.clone());
                    let entry = ranked.entry(key).or_insert_with(|| Candidate::new(row));
                    entry.semantic_rank = Some(rank);
                }
            }
        }

        let mut hits: Vec<Hit> = Vec::new();
        for candidate in ranked.into_values() {
            let Ok(plan) = self.get_plan(candidate.row.plan_id) else {
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

            // Reciprocal rank fusion: the two legs produce incomparable scores, so
            // combine their positions instead of pretending the numbers are on one
            // scale. k = 60 is the usual constant and is not sensitive here.
            let mut score = 0.0f64;
            if let Some(r) = candidate.lexical_rank {
                score += 1.0 / (60.0 + r as f64);
            }
            if let Some(r) = candidate.semantic_rank {
                score += 1.0 / (60.0 + r as f64);
            }
            score *= 100.0;

            if Some(plan.repo_id) == opts.prefer_repo {
                score += 0.5;
            }
            if matches!(plan.status, Status::Active | Status::InReview) {
                score += 0.25;
            } else if plan.status.is_terminal() {
                score -= 0.25;
            }
            score += recency_bonus(&plan.updated_at);

            // Prefer the human label over an internal id.
            let reference = match candidate.row.kind.as_str() {
                "log" | "question" if !candidate.row.label.trim().is_empty() => {
                    candidate.row.label.clone()
                }
                _ => candidate.row.reference.clone(),
            };
            hits.push(Hit {
                plan,
                kind: candidate.row.kind,
                reference,
                snippet: candidate.row.snippet,
                score,
                matched: match (candidate.lexical_rank, candidate.semantic_rank) {
                    (Some(_), Some(_)) => "both",
                    (Some(_), None) => "lexical",
                    _ => "semantic",
                },
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.plan.slug.cmp(&b.plan.slug))
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

    fn lexical(&self, query: &str) -> Result<Vec<Row>> {
        let match_query = to_match_query(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.db().conn();
        let mut stmt = conn.prepare(
            "SELECT plan_id, kind, ref, title, snippet(search, 0, '', '', '…', 14)
             FROM search WHERE search MATCH ?1
             ORDER BY bm25(search, 1.0, 0.0, 0.0, 0.0, 2.0)
             LIMIT 200",
        )?;
        let rows = stmt.query_map([&match_query], |r| {
            Ok(Row {
                plan_id: r.get(0)?,
                kind: r.get(1)?,
                reference: r.get(2)?,
                label: r.get(3)?,
                snippet: tidy(&r.get::<_, String>(4)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn semantic(&self, query: &str, embedder: &dyn Embedder) -> Result<Vec<Row>> {
        let Some((model, _)) = self.embedding_state()? else {
            return Ok(Vec::new());
        };
        if model != embedder.id() {
            // A different model built this index; its vectors are not comparable.
            return Ok(Vec::new());
        }
        let query_vector = embedder.embed_one(query)?;

        let conn = self.db().conn();
        let mut stmt = conn.prepare(
            "SELECT plan_id, kind, ref, title, snippet, vector FROM embedding WHERE model = ?1",
        )?;
        let rows = stmt.query_map([&model], |r| {
            Ok((
                Row {
                    plan_id: r.get(0)?,
                    kind: r.get(1)?,
                    reference: r.get(2)?,
                    label: r.get(3)?,
                    snippet: tidy(&r.get::<_, String>(4)?),
                },
                r.get::<_, Vec<u8>>(5)?,
            ))
        })?;

        // A brute-force scan: thousands of rows, single-digit milliseconds.
        let mut scored: Vec<(f32, Row)> = Vec::new();
        for row in rows {
            let (row, blob) = row?;
            let score = embed::cosine(&query_vector, &embed::from_blob(&blob));
            // No absolute cut-off: what counts as a "similar" cosine differs by model,
            // and fusion uses rank position, not the raw number. Only true zeros are
            // dropped, and the tail is cut by rank below.
            if score > 0.0 {
                scored.push((score, row));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(200);
        Ok(scored.into_iter().map(|(_, row)| row).collect())
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

#[derive(Debug, Clone)]
struct Row {
    plan_id: i64,
    kind: String,
    reference: String,
    /// What to show for `reference` when the reference itself is an id.
    label: String,
    snippet: String,
}

struct Candidate {
    row: Row,
    lexical_rank: Option<usize>,
    semantic_rank: Option<usize>,
}

impl Candidate {
    fn new(row: Row) -> Candidate {
        Candidate {
            row,
            lexical_rank: None,
            semantic_rank: None,
        }
    }
}

fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
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
        0.4
    } else if a[..7.min(a.len())] == b[..7] {
        0.2
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

    fn fixture() -> (tempfile::TempDir, Store) {
        use crate::store::{NewLog, NewPlan, NewSlice};
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(&dir.path().join("p.db")).unwrap();
        let repo_id: i64 = store
            .db()
            .conn()
            .query_row(
                "INSERT INTO repo (key, name, created_at)
                 VALUES ('github.com/acme/widget', 'widget', '2026-01-01T00:00:00Z') RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let picker = store
            .create_plan(NewPlan {
                repo_id,
                title: "ACME-1234 - Reusable Date Range Picker".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .add_slice(NewSlice {
                plan_id: picker.id,
                key: "PR1".into(),
                title: "Shared core".into(),
                scope_md: Some("A two-month calendar with a preset rail.".into()),
                ..Default::default()
            })
            .unwrap();

        let canvas = store
            .create_plan(NewPlan {
                repo_id,
                title: "Canvas Editor".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .add_slice(NewSlice {
                plan_id: canvas.id,
                key: "S5".into(),
                title: "PDF renderer".into(),
                scope_md: Some("Gotenberg prints the diagram onto an A4 page.".into()),
                ..Default::default()
            })
            .unwrap();
        // Two notes on one day: their keys must not collide.
        for body in ["first note of the day", "second note of the day"] {
            store
                .append_log(NewLog {
                    plan_id: canvas.id,
                    body: body.into(),
                    at: Some("2026-07-20T00:00:00Z".into()),
                    ..Default::default()
                })
                .unwrap();
        }
        store.reindex().unwrap();
        (dir, store)
    }

    #[test]
    fn lexical_search_finds_a_plan_by_a_word_it_used() {
        let (_dir, store) = fixture();
        let hits = store
            .search("gotenberg", &SearchOptions::default())
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].plan.slug, "canvas-editor");
        assert_eq!(hits[0].kind, "slice");
        assert_eq!(hits[0].reference, "S5");
        assert_eq!(hits[0].matched, "lexical");
    }

    #[test]
    fn two_notes_on_one_day_are_indexed_separately() {
        let (_dir, mut store) = fixture();
        let e = crate::embed::tests::HashEmbedder { dims: 128 };

        let first = store.embed_all(&e).unwrap();
        assert!(first.embedded > 0);
        // Colliding keys would make each rebuild overwrite and re-embed for ever.
        let second = store.embed_all(&e).unwrap();
        assert_eq!(second.embedded, 0, "a second pass must re-embed nothing");
        assert_eq!(second.removed, 0);

        let logs: i64 = store
            .db()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM embedding WHERE kind = 'log'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(logs, 2);
    }

    #[test]
    fn the_semantic_leg_only_runs_against_vectors_from_its_own_model() {
        let (_dir, mut store) = fixture();
        let e = crate::embed::tests::HashEmbedder { dims: 128 };
        store.embed_all(&e).unwrap();

        let hits = store
            .search_with("gotenberg", &SearchOptions::default(), Some(&e))
            .unwrap();
        assert!(hits.iter().any(|h| h.matched != "lexical"));

        // A different model's vectors are in a different space; comparing them would
        // silently return nonsense, so the semantic leg stands down.
        struct Other(crate::embed::tests::HashEmbedder);
        impl crate::embed::Embedder for Other {
            fn dims(&self) -> usize {
                self.0.dims
            }
            fn id(&self) -> &str {
                "test:other"
            }
            fn embed(&self, texts: &[&str]) -> crate::Result<Vec<Vec<f32>>> {
                self.0.embed(texts)
            }
        }
        let other = Other(crate::embed::tests::HashEmbedder { dims: 128 });
        let hits = store
            .search_with("gotenberg", &SearchOptions::default(), Some(&other))
            .unwrap();
        assert!(hits.iter().all(|h| h.matched == "lexical"));
    }

    #[test]
    fn a_deleted_plan_takes_its_vectors_with_it() {
        let (_dir, mut store) = fixture();
        let e = crate::embed::tests::HashEmbedder { dims: 128 };
        store.embed_all(&e).unwrap();

        let plan = store.find_plan("canvas-editor", None).unwrap();
        store
            .db_mut()
            .write(|tx| {
                tx.execute("DELETE FROM plan WHERE id = ?1", [plan.id])?;
                Ok(())
            })
            .unwrap();

        // The foreign key does this, not the indexer - so a dropped plan can never
        // leave vectors behind that would answer a later search.
        let left: i64 = store
            .db()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM embedding WHERE plan_id = ?1",
                [plan.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn a_deleted_slice_leaves_no_vector_answering_for_it() {
        let (_dir, mut store) = fixture();
        let e = crate::embed::tests::HashEmbedder { dims: 128 };
        store.embed_all(&e).unwrap();

        let plan = store.find_plan("canvas-editor", None).unwrap();
        store
            .db_mut()
            .write(|tx| {
                tx.execute("DELETE FROM slice WHERE plan_id = ?1", [plan.id])?;
                Ok(())
            })
            .unwrap();
        store.reindex().unwrap();
        let stats = store.embed_all(&e).unwrap();
        assert_eq!(stats.removed, 1, "the orphaned vector must be dropped");

        let hits = store
            .search_with("gotenberg", &SearchOptions::default(), Some(&e))
            .unwrap();
        assert!(hits.iter().all(|h| h.kind != "slice"));
    }

    #[test]
    fn a_unit_leads_with_its_title_because_a_heading_is_the_best_sentence() {
        let unit = IndexUnit {
            plan_id: 1,
            kind: "slice",
            reference: "PR1".into(),
            title: "Shared core".into(),
            body: "The core and the first variant.".into(),
        };
        assert_eq!(
            unit.embed_text(),
            "Shared core. The core and the first variant."
        );
    }
}
