//! Checkpointing a worktree so a fresh context resumes exactly where it left off.
//!
//! This is `toolbox-handoff` step 3 moved into the database (D11). The other steps -
//! commit and push, certify the gates, tell the user - are unchanged; only the place
//! the resume doc lives changes, and the change is what stops it being a file that
//! gets copied between worktrees.

use std::fmt::Write as _;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::model::{LogKind, Status};
use crate::store::{NewLog, Store};
use crate::util::now;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gate {
    pub name: String,
    /// `pass`, `fail`, or whatever the runner actually said.
    pub result: String,
    pub detail: Option<String>,
}

impl Gate {
    /// `typecheck=pass`, `test=pass:731 tests`, `lint=fail`.
    pub fn parse(spec: &str) -> Gate {
        let (name, rest) = spec.split_once('=').unwrap_or((spec, "pass"));
        let (result, detail) = rest.split_once(':').unwrap_or((rest, ""));
        Gate {
            name: name.trim().to_string(),
            result: result.trim().to_string(),
            detail: Some(detail.trim().to_string()).filter(|d| !d.is_empty()),
        }
    }

    pub fn passed(&self) -> bool {
        matches!(
            self.result.to_lowercase().as_str(),
            "pass" | "passed" | "green" | "ok" | "yes"
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub id: i64,
    pub plan_id: i64,
    pub worktree_path: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub gates: Vec<Gate>,
    pub resume_md: String,
    pub next_md: String,
    pub actor: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Default)]
pub struct NewHandoff {
    pub plan_id: i64,
    pub worktree_path: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub gates: Vec<Gate>,
    pub resume_md: String,
    pub next_md: String,
}

impl Store {
    pub fn write_handoff(&mut self, new: NewHandoff) -> Result<Handoff> {
        let gates_json = serde_json::to_string(&new.gates)?;
        let me = self.actor().to_string();
        let ts = now();
        let plan_id = new.plan_id;
        let worktree = new.worktree_path.clone();

        let id = self.db_mut().write(|tx| {
            tx.execute(
                "INSERT INTO handoff
                   (plan_id, worktree_path, branch, head_sha, gates_json, resume_md, next_md, actor, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    plan_id,
                    worktree,
                    new.branch,
                    new.head_sha,
                    gates_json,
                    new.resume_md,
                    new.next_md,
                    me,
                    ts
                ],
            )?;
            Ok(tx.last_insert_rowid())
        })?;

        let failed: Vec<&str> = new
            .gates
            .iter()
            .filter(|g| !g.passed())
            .map(|g| g.name.as_str())
            .collect();
        let note = if new.gates.is_empty() {
            "handoff written (no gates recorded)".to_string()
        } else if failed.is_empty() {
            format!("handoff written, {} gates green", new.gates.len())
        } else {
            // Never let a handoff imply green over a failure.
            format!("handoff written, RED: {}", failed.join(", "))
        };
        self.append_log(NewLog {
            plan_id,
            kind: Some(LogKind::Handoff),
            body: note,
            branch: new.branch.clone(),
            worktree_path: Some(new.worktree_path.clone()),
            ..Default::default()
        })?;

        self.handoff(id)
    }

    pub fn handoff(&self, id: i64) -> Result<Handoff> {
        Ok(self.db().conn().query_row(
            &format!("{HANDOFF_SELECT} WHERE id = ?1"),
            [id],
            row_to_handoff,
        )?)
    }

    /// The most recent handoff for this plan in this worktree.
    pub fn latest_handoff(&self, plan_id: i64, worktree: &str) -> Result<Option<Handoff>> {
        Ok(self
            .db()
            .conn()
            .query_row(
                &format!(
                    "{HANDOFF_SELECT} WHERE plan_id = ?1 AND worktree_path = ?2
                     ORDER BY at DESC, id DESC LIMIT 1"
                ),
                params![plan_id, worktree],
                row_to_handoff,
            )
            .optional()?)
    }

    /// Every worktree's latest handoff for a plan - what the other three are doing.
    pub fn handoffs_for(&self, plan_id: i64) -> Result<Vec<Handoff>> {
        let conn = self.db().conn();
        let mut stmt = conn.prepare(&format!(
            "{HANDOFF_SELECT} WHERE id IN (
                 SELECT MAX(id) FROM handoff WHERE plan_id = ?1 GROUP BY worktree_path
             ) ORDER BY at DESC"
        ))?;
        let rows = stmt.query_map([plan_id], row_to_handoff)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The document a fresh context should read first. Everything `HANDOFF.md` used to
    /// carry, assembled from live state rather than from whatever was last typed.
    pub fn render_resume(&self, plan_id: i64, worktree: Option<&str>) -> Result<String> {
        let plan = self.get_plan(plan_id)?;
        let slices = self.slices(plan_id)?;
        let done = slices.iter().filter(|s| s.status == Status::Done).count();
        let handoff = match worktree {
            Some(wt) => self.latest_handoff(plan_id, wt)?,
            None => None,
        };

        let mut out = String::new();
        let _ = writeln!(out, "# Resume: {}\n", plan.title);
        let _ = writeln!(
            out,
            "**Plan:** `{}` · **Status:** {} · **{done}/{} slices done**",
            plan.slug,
            plan.status,
            slices.len()
        );
        if let Some(h) = &handoff {
            let head = h.head_sha.as_deref().unwrap_or("?");
            let branch = h.branch.as_deref().unwrap_or("?");
            let _ = writeln!(
                out,
                "**Last handoff:** {} by {} · `{branch}` @ `{head}`",
                h.at.split('T').next().unwrap_or(&h.at),
                h.actor.as_deref().unwrap_or("?")
            );
            if !h.gates.is_empty() {
                let red: Vec<&str> = h
                    .gates
                    .iter()
                    .filter(|g| !g.passed())
                    .map(|g| g.name.as_str())
                    .collect();
                if red.is_empty() {
                    let _ = writeln!(
                        out,
                        "**Gates:** all {} green at that point - re-run them before trusting it",
                        h.gates.len()
                    );
                } else {
                    let _ = writeln!(out, "**Gates: RED** - {}", red.join(", "));
                }
            }
        }
        out.push('\n');

        if let Some(h) = &handoff {
            if !h.next_md.trim().is_empty() {
                let _ = writeln!(out, "## Do this first\n\n{}\n", h.next_md.trim());
            }
        }

        let next = self.next_slice(plan_id, worktree)?;
        if let Some(n) = &next {
            let _ = writeln!(out, "## Next slice\n");
            let _ = writeln!(out, "**{} - {}** ({})\n", n.key, n.title, n.status);
            if !n.scope_md.trim().is_empty() {
                let _ = writeln!(out, "{}\n", n.scope_md.trim());
            }
            if let Some(demo) = n.demo_md.as_deref().filter(|d| !d.trim().is_empty()) {
                let _ = writeln!(out, "**Demo:** {demo}\n");
            }
        }

        if !slices.is_empty() {
            let _ = writeln!(out, "## Slices\n");
            let _ = writeln!(out, "| Slice | Status | Branch | Where |");
            let _ = writeln!(out, "| --- | --- | --- | --- |");
            for s in &slices {
                let where_ = match (&s.claimed_by, &s.worktree_path) {
                    (Some(by), Some(wt)) if Some(wt.as_str()) == worktree => {
                        format!("{by}, here")
                    }
                    (Some(by), Some(wt)) => format!("{by} in `{wt}`"),
                    _ => String::new(),
                };
                let _ = writeln!(
                    out,
                    "| {} {} | {} | {} | {where_} |",
                    s.key,
                    s.title,
                    s.status,
                    s.branch.as_deref().unwrap_or("")
                );
            }
            out.push('\n');
        }

        let gotchas = self.gotchas(plan_id)?;
        if !gotchas.is_empty() {
            let _ = writeln!(out, "## Gotchas\n");
            for g in gotchas {
                let _ = writeln!(out, "### {}\n", g.title);
                if !g.body.trim().is_empty() {
                    let _ = writeln!(out, "{}\n", g.body.trim());
                }
            }
        }

        let questions = self.questions(plan_id, true)?;
        if !questions.is_empty() {
            let _ = writeln!(out, "## Open questions\n");
            for q in questions {
                let _ = writeln!(out, "- {}", q.body);
            }
            out.push('\n');
        }

        let log = self.log(plan_id, Some(8))?;
        if !log.is_empty() {
            let _ = writeln!(out, "## Recent\n");
            for e in log {
                let date = e.at.split('T').next().unwrap_or(&e.at);
                let slice = e
                    .slice_key
                    .as_deref()
                    .map(|k| format!("**{k}** "))
                    .unwrap_or_default();
                let _ = writeln!(out, "- {date} - {slice}{}", e.body.replace('\n', " "));
            }
            out.push('\n');
        }

        if let Some(h) = &handoff {
            if !h.resume_md.trim().is_empty() {
                let _ = writeln!(
                    out,
                    "## Notes from the last session\n\n{}\n",
                    h.resume_md.trim()
                );
            }
        }

        let _ = writeln!(
            out,
            "---\n\nRead the full plan with `aip show {}`. Record progress as you go with \
             `aip log`, and move slices with `aip slice set <key> <status>`.",
            plan.slug
        );
        Ok(out)
    }
}

const HANDOFF_SELECT: &str = "SELECT id, plan_id, worktree_path, branch, head_sha, gates_json,
            resume_md, next_md, actor, at
     FROM handoff";

fn row_to_handoff(row: &rusqlite::Row<'_>) -> rusqlite::Result<Handoff> {
    let gates_json: Option<String> = row.get(5)?;
    Ok(Handoff {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        worktree_path: row.get(2)?,
        branch: row.get(3)?,
        head_sha: row.get(4)?,
        gates: gates_json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default(),
        resume_md: row.get(6)?,
        next_md: row.get(7)?,
        actor: row.get(8)?,
        at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_specs_parse_the_way_they_are_typed() {
        let g = Gate::parse("typecheck=pass");
        assert_eq!(g.name, "typecheck");
        assert!(g.passed());
        assert_eq!(g.detail, None);

        let g = Gate::parse("test=pass:731 tests in @acme/ui");
        assert!(g.passed());
        assert_eq!(g.detail.as_deref(), Some("731 tests in @acme/ui"));

        let g = Gate::parse("lint=fail");
        assert!(!g.passed());

        // A bare name is an assertion that it passed.
        assert!(Gate::parse("build").passed());
    }
}
