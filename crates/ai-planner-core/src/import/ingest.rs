//! Turning a parsed document into rows, without losing anything (D5).

use std::path::Path;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use super::parse::{parse_plan, ParsedPlan};
use crate::error::Result;
use crate::model::{DecisionStatus, LogKind, Plan, Status};
use crate::store::{NewLog, NewPlan, Store};
use crate::util::{now, slugify, ticket_key};

#[derive(Debug, Clone, Copy, Default)]
pub struct ImportOptions {
    /// Overwrite a plan of the same slug whose content has drifted.
    pub replace: bool,
    /// Parse and report, write nothing.
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Created {
        plan: Plan,
        slices: usize,
        decisions: usize,
        log: usize,
    },
    /// Byte-identical to something already imported - the copies in other worktrees.
    AlreadyImported {
        plan: Plan,
        first_seen: String,
    },
    /// Same plan, different content - two worktrees that drifted apart.
    Conflict {
        plan: Plan,
        existing_sources: Vec<String>,
    },
    Replaced {
        plan: Plan,
    },
    HandoffAttached {
        plan: Plan,
        worktree: String,
    },
    Skipped {
        reason: String,
    },
    Planned {
        title: String,
        slug: String,
        slices: usize,
        decisions: usize,
        log: usize,
    },
}

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A handoff is a different document: it records where one worktree got to, not what
/// the project is. It attaches to a plan rather than becoming one.
pub fn looks_like_handoff(path: &Path, md: &str) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_uppercase();
    if name.starts_with("HANDOFF") {
        return true;
    }
    md.lines()
        .find(|l| l.starts_with("# "))
        .is_some_and(|l| l[2..].trim().to_uppercase().starts_with("HANDOFF"))
}

impl Store {
    pub fn import_file(
        &mut self,
        repo_id: i64,
        path: &Path,
        md: &str,
        opts: ImportOptions,
    ) -> Result<Outcome> {
        if looks_like_handoff(path, md) {
            return self.import_handoff(repo_id, path, md, opts);
        }
        self.import_plan_md(repo_id, path, md, opts)
    }

    fn import_plan_md(
        &mut self,
        repo_id: i64,
        path: &Path,
        md: &str,
        opts: ImportOptions,
    ) -> Result<Outcome> {
        let parsed = parse_plan(md);
        if parsed.title.trim().is_empty() {
            return Ok(Outcome::Skipped {
                reason: "no title heading".into(),
            });
        }
        let slug = derive_slug(&parsed);
        let sha = sha256(md.as_bytes());

        if opts.dry_run {
            return Ok(Outcome::Planned {
                title: parsed.title.clone(),
                slug,
                slices: parsed.slices.len(),
                decisions: parsed.decisions.len(),
                log: parsed.log.len(),
            });
        }

        // The same file copied into four worktrees imports once.
        if let Some((plan_id, first_seen)) = self.import_by_sha(repo_id, &sha)? {
            self.record_import(plan_id, path, &sha, md.len())?;
            return Ok(Outcome::AlreadyImported {
                plan: self.get_plan(plan_id)?,
                first_seen,
            });
        }

        if let Some(existing) = self.plan_by_slug(repo_id, &slug)? {
            if !opts.replace {
                return Ok(Outcome::Conflict {
                    existing_sources: self.import_sources(existing.id)?,
                    plan: existing,
                });
            }
            self.delete_plan_content(existing.id)?;
            self.write_parsed(existing.id, &parsed, md, path)?;
            self.record_import(existing.id, path, &sha, md.len())?;
            return Ok(Outcome::Replaced {
                plan: self.get_plan(existing.id)?,
            });
        }

        let plan = self.create_plan(NewPlan {
            repo_id,
            title: parsed.title.clone(),
            slug: Some(slug),
            status: Some(derive_status(&parsed)),
            summary: parsed.summary.clone(),
            ticket_key: parsed.ticket_key.clone(),
            ticket_url: parsed.ticket_url.clone(),
            base_branch: parsed.base_branch.clone(),
            owner: parsed.owner.clone(),
            raw_md: Some(md.to_string()),
            source_path: Some(path.to_string_lossy().to_string()),
            bare: true,
        })?;

        self.write_parsed(plan.id, &parsed, md, path)?;
        self.record_import(plan.id, path, &sha, md.len())?;

        Ok(Outcome::Created {
            plan: self.get_plan(plan.id)?,
            slices: parsed.slices.len(),
            decisions: parsed.decisions.len(),
            log: parsed.log.len(),
        })
    }

    fn write_parsed(
        &mut self,
        plan_id: i64,
        parsed: &ParsedPlan,
        md: &str,
        path: &Path,
    ) -> Result<()> {
        let ts = now();
        let me = self.actor().to_string();
        let raw = md.to_string();
        let source = path.to_string_lossy().to_string();

        self.db_mut().write(|tx| {
            tx.execute(
                "UPDATE plan SET raw_md = ?2, source_path = ?3 WHERE id = ?1",
                params![plan_id, raw, source],
            )?;

            for (i, s) in parsed.sections.iter().enumerate() {
                tx.execute(
                    "INSERT INTO plan_section (plan_id, ord, key, title, body, renders, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![
                        plan_id,
                        (i as i64 + 1) * 10,
                        s.key,
                        s.title,
                        s.body,
                        s.renders.as_str(),
                        ts
                    ],
                )?;
            }

            for (i, d) in parsed.decisions.iter().enumerate() {
                tx.execute(
                    "INSERT INTO decision (plan_id, ord, key, title, body, status, decided_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![
                        plan_id,
                        (i as i64 + 1) * 10,
                        d.key,
                        d.title,
                        d.body,
                        DecisionStatus::Agreed.as_str(),
                        ts
                    ],
                )?;
            }

            for (i, s) in parsed.slices.iter().enumerate() {
                tx.execute(
                    "INSERT INTO slice
                       (plan_id, ord, key, title, status, scope_md, demo_md, estimate_files,
                        completed_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    params![
                        plan_id,
                        (i as i64 + 1) * 10,
                        s.key,
                        s.title,
                        s.status,
                        s.scope,
                        s.demo,
                        s.estimate_files,
                        if s.status == Status::Done { Some(&ts) } else { None },
                        ts
                    ],
                )?;
            }

            for q in &parsed.questions {
                tx.execute(
                    "INSERT INTO question (plan_id, body, asked_at) VALUES (?1, ?2, ?3)",
                    params![plan_id, q, ts],
                )?;
            }

            for (title, body) in &parsed.gotchas {
                tx.execute(
                    "INSERT INTO gotcha (plan_id, title, body, created_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(plan_id, title) DO NOTHING",
                    params![plan_id, title, body, ts],
                )?;
            }

            // Oldest first, so the id order matches the order things happened.
            for entry in parsed.log.iter().rev() {
                tx.execute(
                    "INSERT INTO log (plan_id, at, actor, kind, body) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        plan_id,
                        entry.at.clone().unwrap_or_else(|| ts.clone()),
                        me,
                        LogKind::Progress.as_str(),
                        entry.body
                    ],
                )?;
            }
            Ok(())
        })
    }

    fn import_handoff(
        &mut self,
        repo_id: i64,
        path: &Path,
        md: &str,
        opts: ImportOptions,
    ) -> Result<Outcome> {
        let title = md
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l[2..].trim().to_string())
            .unwrap_or_default();
        let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("");

        let Some(plan) = self.match_handoff_plan(repo_id, &title, name, md)? else {
            return Ok(Outcome::Skipped {
                reason: format!("{name}: no plan to attach it to - pass --as <plan>"),
            });
        };

        // The file's own location is authoritative. Handoffs routinely mention other
        // worktrees, so reading a path out of the text picks the wrong one.
        let worktree = crate::git::GitContext::detect(path.parent().unwrap_or(Path::new(".")))
            .map(|g| g.worktree_str())
            .unwrap_or_else(|_| {
                path.parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            });

        if opts.dry_run {
            return Ok(Outcome::HandoffAttached { plan, worktree });
        }

        let branch = field_after(md, "branch");
        let head_sha = field_after(md, "head");
        let ts = now();
        let me = self.actor().to_string();
        let plan_id = plan.id;
        let resume = md.to_string();
        let wt = worktree.clone();
        self.db_mut().write(|tx| {
            tx.execute(
                "INSERT INTO handoff (plan_id, worktree_path, branch, head_sha, resume_md, actor, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![plan_id, wt, branch, head_sha, resume, me, ts],
            )?;
            Ok(())
        })?;

        for (title, body) in super::handoff_gotchas(md) {
            self.add_gotcha(plan_id, &title, &body)?;
        }
        self.append_log(NewLog {
            plan_id,
            kind: Some(LogKind::Handoff),
            body: format!("imported handoff from {}", path.display()),
            worktree_path: Some(worktree.clone()),
            ..Default::default()
        })?;

        Ok(Outcome::HandoffAttached { plan, worktree })
    }

    /// Which plan a handoff belongs to, in order of how reliable the signal is:
    /// a ticket key, then the subject in its title, then a build-plan filename quoted
    /// in its body. A bare `HANDOFF.md` usually only has the third.
    fn match_handoff_plan(
        &self,
        repo_id: i64,
        title: &str,
        name: &str,
        md: &str,
    ) -> Result<Option<Plan>> {
        if let Some(key) = ticket_key(title).or_else(|| ticket_key(name)) {
            if let Ok(plan) = self.find_plan(&key, Some(repo_id)) {
                return Ok(Some(plan));
            }
        }

        if let Some(subject) = handoff_subject(title) {
            if let Ok(plan) = self.find_plan(&subject, Some(repo_id)) {
                return Ok(Some(plan));
            }
        }

        for file in build_plan_files_named_in(md) {
            if let Some(plan) = self.plan_imported_from(repo_id, &file)? {
                return Ok(Some(plan));
            }
        }
        Ok(None)
    }

    /// The plan imported from a file with this name, wherever the copy lived.
    fn plan_imported_from(&self, repo_id: i64, filename: &str) -> Result<Option<Plan>> {
        let like = format!("%/{filename}");
        Ok(self
            .db()
            .conn()
            .query_row(
                &format!(
                    "{} JOIN plan_import i ON i.plan_id = p.id
                     WHERE p.repo_id = ?1 AND i.source_path LIKE ?2 LIMIT 1",
                    crate::store::PLAN_SELECT
                ),
                params![repo_id, like],
                crate::store::row_to_plan,
            )
            .optional()?)
    }

    /// Attach a handoff to a plan the caller named, for the ones nothing identifies.
    pub fn attach_handoff(
        &mut self,
        plan_id: i64,
        path: &Path,
        md: &str,
        worktree: &str,
    ) -> Result<()> {
        let branch = field_after(md, "branch");
        let head_sha = field_after(md, "head");
        let (resume, wt, me, ts) = (
            md.to_string(),
            worktree.to_string(),
            self.actor().to_string(),
            now(),
        );
        self.db_mut().write(|tx| {
            tx.execute(
                "INSERT INTO handoff (plan_id, worktree_path, branch, head_sha, resume_md, actor, at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![plan_id, wt, branch, head_sha, resume, me, ts],
            )?;
            Ok(())
        })?;
        for (title, body) in super::handoff_gotchas(md) {
            self.add_gotcha(plan_id, &title, &body)?;
        }
        self.append_log(NewLog {
            plan_id,
            kind: Some(LogKind::Handoff),
            body: format!("imported handoff from {}", path.display()),
            worktree_path: Some(worktree.to_string()),
            ..Default::default()
        })?;
        Ok(())
    }

    pub fn plan_by_slug(&self, repo_id: i64, slug: &str) -> Result<Option<Plan>> {
        Ok(self
            .db()
            .conn()
            .query_row(
                &format!(
                    "{} WHERE p.repo_id = ?1 AND p.slug = ?2",
                    crate::store::PLAN_SELECT
                ),
                params![repo_id, slug],
                crate::store::row_to_plan,
            )
            .optional()?)
    }

    fn import_by_sha(&self, repo_id: i64, sha: &str) -> Result<Option<(i64, String)>> {
        Ok(self
            .db()
            .conn()
            .query_row(
                "SELECT i.plan_id, i.source_path FROM plan_import i
                 JOIN plan p ON p.id = i.plan_id
                 WHERE i.sha256 = ?1 AND p.repo_id = ?2
                 ORDER BY i.imported_at LIMIT 1",
                params![sha, repo_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn import_sources(&self, plan_id: i64) -> Result<Vec<String>> {
        let conn = self.db().conn();
        let mut stmt = conn.prepare(
            "SELECT source_path FROM plan_import WHERE plan_id = ?1 ORDER BY imported_at",
        )?;
        let rows = stmt.query_map([plan_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn record_import(&mut self, plan_id: i64, path: &Path, sha: &str, bytes: usize) -> Result<()> {
        let path = path.to_string_lossy().to_string();
        let sha = sha.to_string();
        self.db_mut().write(|tx| {
            tx.execute(
                "INSERT INTO plan_import (plan_id, source_path, sha256, bytes, imported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![plan_id, path, sha, bytes as i64, now()],
            )?;
            Ok(())
        })
    }

    /// Clear a plan's derived rows before a `--replace`. The plan row, its history and
    /// its imports survive, so a replace never loses the audit trail.
    fn delete_plan_content(&mut self, plan_id: i64) -> Result<()> {
        self.db_mut().write(|tx| {
            for table in ["plan_section", "decision", "slice", "question"] {
                tx.execute(
                    &format!("DELETE FROM {table} WHERE plan_id = ?1"),
                    [plan_id],
                )?;
            }
            Ok(())
        })
    }
}

fn derive_slug(parsed: &ParsedPlan) -> String {
    if let Some(key) = &parsed.ticket_key {
        return slugify(key);
    }
    // "Canvas Editor - Build Plan" is the Canvas Editor plan.
    let title = parsed
        .title
        .split(" - ")
        .next()
        .unwrap_or(&parsed.title)
        .trim();
    let title = title
        .trim_end_matches("Build Plan")
        .trim_end_matches("build plan")
        .trim();
    slugify(if title.is_empty() {
        &parsed.title
    } else {
        title
    })
}

fn derive_status(parsed: &ParsedPlan) -> Status {
    if parsed.slices.is_empty() {
        return Status::Draft;
    }
    if parsed
        .slices
        .iter()
        .any(|s| matches!(s.status, Status::Active | Status::InReview))
    {
        return Status::Active;
    }
    if parsed.slices.iter().all(|s| s.status.is_terminal()) {
        return Status::Done;
    }
    // Something finished but more is outstanding: the living-plan state.
    if parsed.slices.iter().any(|s| s.status == Status::Done) {
        Status::Active
    } else {
        Status::Ready
    }
}

/// `HANDOFF - Canvas Editor (ACME-1151..39909)` is about Canvas Editor.
fn handoff_subject(title: &str) -> Option<String> {
    let t = title.trim();
    let rest = t
        .strip_prefix("HANDOFF")
        .or_else(|| t.strip_prefix("Handoff"))
        .or_else(|| t.strip_prefix("handoff"))?
        .trim_start_matches([':', '-', '–', '—', ' '])
        .trim();
    // Drop a trailing parenthetical and the word "project", which are decoration.
    let rest = match rest.find('(') {
        Some(i) => rest[..i].trim(),
        None => rest,
    };
    let rest = rest.trim_end_matches(" project").trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// A bare `HANDOFF.md` usually quotes the plan it belongs to by filename.
fn build_plan_files_named_in(md: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in md.split(|c: char| c.is_whitespace() || "`\"'()[],".contains(c)) {
        let upper = token.to_uppercase();
        if upper.ends_with(".MD") && (upper.contains("BUILD_PLAN") || upper.contains("BUILD-PLAN"))
        {
            let name = token.trim().to_string();
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

fn field_after(md: &str, label: &str) -> Option<String> {
    let needle = format!("{label}:");
    for line in md.lines().take(80) {
        let t = line.trim().trim_start_matches(['-', '*', '#']).trim();
        if t.to_lowercase().starts_with(&needle) {
            let value = t[needle.len()..].trim().trim_matches('`');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}
