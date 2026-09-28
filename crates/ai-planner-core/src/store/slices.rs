use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::plans::{row_to_slice, HeldSlice};
use super::Store;
use crate::error::{Error, Result};
use crate::model::{LogKind, Slice, Status};
use crate::util::now;

#[derive(Debug, Clone, Default)]
pub struct NewSlice {
    pub plan_id: i64,
    pub key: String,
    pub title: String,
    pub status: Option<Status>,
    pub scope_md: Option<String>,
    pub demo_md: Option<String>,
    pub estimate_files: Option<i64>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub ord: Option<i64>,
}

/// A patch. Every `None` field is left alone, so two agents editing different
/// attributes of one slice do not fight over the columns they are not touching.
#[derive(Debug, Clone, Default)]
pub struct SliceUpdate {
    pub title: Option<String>,
    pub scope_md: Option<String>,
    pub demo_md: Option<String>,
    pub estimate_files: Option<i64>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub pr_url: Option<String>,
    pub blocked_reason: Option<String>,
}

/// What deleting one slice would destroy, and what it would only detach. Deleting is
/// the one write that cannot be taken back, so the caller is handed the size of it
/// before and after rather than a bare row count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceRemoval {
    pub slice_id: i64,
    pub plan_id: i64,
    pub plan_slug: String,
    pub key: String,
    pub title: String,
    pub status: Status,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    /// Progress notes written against it. These are *not* destroyed - the log is
    /// append-only and its `slice_id` falls to NULL - but they stop naming the slice.
    pub detached_log_entries: i64,
    /// Questions asked against it. They stay on the plan, likewise detached.
    pub detached_questions: i64,
    /// Dependency edges pointing at it. The edge goes; the slice at the other end does not.
    pub dependents: i64,
    pub embeddings: i64,
    /// Who is holding it, and where. Live work, and the reason to hesitate.
    pub held: Option<HeldSlice>,
}

const SLICE_SELECT: &str = "SELECT id, plan_id, ord, key, title, status, scope_md, demo_md,
            estimate_files, branch, base_branch, pr_url, worktree_path, claimed_by,
            claimed_at, blocked_reason, started_at, completed_at, rev, updated_at
     FROM slice";

impl Store {
    pub fn add_slice(&mut self, new: NewSlice) -> Result<Slice> {
        let key = new.key.trim().to_string();
        if key.is_empty() {
            return Err(Error::invalid("a slice needs a key, e.g. PR1 or S2 or M4"));
        }
        let plan = self.get_plan(new.plan_id)?;
        if self.slice(new.plan_id, &key)?.is_some() {
            return Err(Error::DuplicateSlice(plan.slug, key));
        }

        let plan_id = new.plan_id;
        let title = new.title.trim().to_string();
        let status = new.status.unwrap_or(Status::Ready);
        self.db.write(|tx| {
            let ord = match new.ord {
                Some(o) => o,
                None => tx.query_row(
                    "SELECT COALESCE(MAX(ord), 0) + 10 FROM slice WHERE plan_id = ?1",
                    [plan_id],
                    |r| r.get::<_, i64>(0),
                )?,
            };
            tx.execute(
                "INSERT INTO slice
                   (plan_id, ord, key, title, status, scope_md, demo_md, estimate_files,
                    branch, base_branch, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, COALESCE(?6, ''), ?7, ?8, ?9, ?10, ?11, ?11)",
                params![
                    plan_id,
                    ord,
                    key,
                    title,
                    status,
                    new.scope_md,
                    new.demo_md,
                    new.estimate_files,
                    new.branch,
                    new.base_branch,
                    now(),
                ],
            )?;
            Ok(())
        })?;

        self.slice(plan_id, &key)?
            .ok_or_else(|| Error::NoSuchSlice(plan.slug, key))
    }

    pub fn slice(&self, plan_id: i64, key: &str) -> Result<Option<Slice>> {
        Ok(self
            .db
            .conn()
            .query_row(
                &format!("{SLICE_SELECT} WHERE plan_id = ?1 AND key = ?2 COLLATE NOCASE"),
                params![plan_id, key],
                row_to_slice,
            )
            .optional()?)
    }

    pub fn require_slice(&self, plan_id: i64, key: &str) -> Result<Slice> {
        match self.slice(plan_id, key)? {
            Some(s) => Ok(s),
            None => {
                let plan = self.get_plan(plan_id)?;
                Err(Error::NoSuchSlice(plan.slug, key.to_string()))
            }
        }
    }

    pub fn slice_by_id(&self, id: i64) -> Result<Slice> {
        self.db
            .conn()
            .query_row(&format!("{SLICE_SELECT} WHERE id = ?1"), [id], row_to_slice)
            .optional()?
            .ok_or_else(|| Error::NoSuchSlice("?".into(), id.to_string()))
    }

    pub fn slices(&self, plan_id: i64) -> Result<Vec<Slice>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(&format!(
            "{SLICE_SELECT} WHERE plan_id = ?1 ORDER BY ord, id"
        ))?;
        let rows = stmt.query_map([plan_id], row_to_slice)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Slices anywhere in the database whose recorded branch matches. This is rule 2
    /// of the resolution cascade and the reason it usually needs nothing cleverer (D7).
    pub fn slices_on_branch(&self, repo_id: i64, branch: &str) -> Result<Vec<Slice>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(&format!(
            "{SLICE_SELECT} WHERE branch = ?2
             AND plan_id IN (SELECT id FROM plan WHERE repo_id = ?1)
             ORDER BY updated_at DESC"
        ))?;
        let rows = stmt.query_map(params![repo_id, branch], row_to_slice)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Slices claimed in a given worktree, newest claim first.
    pub fn slices_claimed_in(&self, worktree: &str) -> Result<Vec<Slice>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(&format!(
            "{SLICE_SELECT} WHERE worktree_path = ?1 AND claimed_by IS NOT NULL
             ORDER BY claimed_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([worktree], row_to_slice)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_slice_status(
        &mut self,
        slice: &Slice,
        status: Status,
        reason: Option<&str>,
    ) -> Result<Slice> {
        let id = slice.id;
        let plan_id = slice.plan_id;
        let key = slice.key.clone();
        let from = slice.status;
        let reason = reason.map(str::to_string);
        let note = match &reason {
            Some(r) => format!("{key} {from} -> {status}: {r}"),
            None => format!("{key} {from} -> {status}"),
        };
        let me = self.actor.clone();

        self.db.write(|tx| {
            tx.execute(
                "UPDATE slice SET
                     status         = ?2,
                     blocked_reason = CASE WHEN ?2 = 'blocked' THEN COALESCE(?3, blocked_reason) ELSE NULL END,
                     started_at     = CASE WHEN started_at IS NULL AND ?2 IN ('active','in_review')
                                           THEN ?4 ELSE started_at END,
                     completed_at   = CASE WHEN ?2 = 'done' THEN ?4
                                           WHEN ?2 = 'deferred' THEN COALESCE(completed_at, ?4)
                                           ELSE NULL END,
                     -- Finishing a slice gives the worktree back. Who built it stays
                     -- readable in worktree_path and in the log; holding the claim
                     -- would just make a finished slice look like work in progress.
                     claimed_by     = CASE WHEN ?2 IN ('done','deferred') THEN NULL
                                           ELSE claimed_by END,
                     claimed_at     = CASE WHEN ?2 IN ('done','deferred') THEN NULL
                                           ELSE claimed_at END,
                     updated_at     = ?4,
                     rev            = rev + 1
                 WHERE id = ?1",
                params![id, status, reason, now()],
            )?;
            // Work starting on any slice means the plan itself is under way.
            tx.execute(
                "UPDATE plan SET
                     status = CASE WHEN status IN ('draft','ready') AND ?3 IN ('active','in_review')
                                   THEN 'active' ELSE status END,
                     updated_at = ?2
                 WHERE id = ?1",
                params![plan_id, now(), status],
            )?;
            super::notes::insert_log(tx, &me, plan_id, Some(id), LogKind::Status, &note)?;
            Ok(())
        })?;

        self.slice_by_id(id)
    }

    pub fn update_slice(&mut self, slice: &Slice, patch: SliceUpdate) -> Result<Slice> {
        let id = slice.id;
        self.db.write(|tx| {
            tx.execute(
                "UPDATE slice SET
                     title          = COALESCE(?2, title),
                     scope_md       = COALESCE(?3, scope_md),
                     demo_md        = COALESCE(?4, demo_md),
                     estimate_files = COALESCE(?5, estimate_files),
                     branch         = COALESCE(?6, branch),
                     base_branch    = COALESCE(?7, base_branch),
                     pr_url         = COALESCE(?8, pr_url),
                     blocked_reason = COALESCE(?9, blocked_reason),
                     updated_at     = ?10,
                     rev            = rev + 1
                 WHERE id = ?1",
                params![
                    id,
                    patch.title,
                    patch.scope_md,
                    patch.demo_md,
                    patch.estimate_files,
                    patch.branch,
                    patch.base_branch,
                    patch.pr_url,
                    patch.blocked_reason,
                    now(),
                ],
            )?;
            Ok(())
        })?;
        self.slice_by_id(id)
    }

    /// What deleting this slice would take with it. Counted separately from the delete
    /// so a caller can show it first, and returned by the delete so it can report what
    /// actually went.
    pub fn slice_removal(&self, slice: &Slice) -> Result<SliceRemoval> {
        let conn = self.db.conn();
        let counts: [i64; 4] = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM log       WHERE slice_id = ?1),
                    (SELECT COUNT(*) FROM question  WHERE slice_id = ?1),
                    (SELECT COUNT(*) FROM slice_dep WHERE depends_on_id = ?1),
                    (SELECT COUNT(*) FROM embedding
                      WHERE plan_id = ?2 AND kind = 'slice' AND ref = ?3)",
            params![slice.id, slice.plan_id, slice.key],
            |r| {
                let mut out = [0i64; 4];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = r.get(i)?;
                }
                Ok(out)
            },
        )?;
        let plan_slug: String = conn.query_row(
            "SELECT slug FROM plan WHERE id = ?1",
            [slice.plan_id],
            |r| r.get(0),
        )?;

        Ok(SliceRemoval {
            slice_id: slice.id,
            plan_id: slice.plan_id,
            plan_slug,
            key: slice.key.clone(),
            title: slice.title.clone(),
            status: slice.status,
            branch: slice.branch.clone(),
            pr_url: slice.pr_url.clone(),
            detached_log_entries: counts[0],
            detached_questions: counts[1],
            dependents: counts[2],
            embeddings: counts[3],
            held: slice.claimed_by.as_ref().map(|by| HeldSlice {
                key: slice.key.clone(),
                claimed_by: by.clone(),
                worktree_path: slice.worktree_path.clone().unwrap_or_default(),
            }),
        })
    }

    /// Delete a slice, and everything derived from it.
    ///
    /// Irreversible, so it refuses while somebody is holding it unless `force` says
    /// otherwise: that is live work, and the agent deleting is rarely the one doing it.
    /// `worktree` is the caller's own path - a claim it holds itself is its own to
    /// throw away. The log notes written against the slice survive it, detached: they
    /// are the record of what happened, and the slice row is only the label on it.
    pub fn delete_slice(
        &mut self,
        slice: &Slice,
        force: bool,
        worktree: Option<&str>,
    ) -> Result<SliceRemoval> {
        let removal = self.slice_removal(slice)?;
        let id = slice.id;
        let plan_id = slice.plan_id;
        let key = slice.key.clone();
        let title = slice.title.clone();
        let me = self.actor.clone();
        let worktree = worktree.map(str::to_string);

        let gone = self.db.write(|tx| {
            // The guard is the `WHERE` clause rather than the read above, so a claim
            // that lands between the two loses the race instead of being deleted by
            // it (D6). A held slice with no worktree to compare against is never ours.
            let gone = tx.execute(
                "DELETE FROM slice
                 WHERE id = ?1 AND (?2 OR claimed_by IS NULL OR worktree_path = ?3)",
                params![id, force, worktree],
            )?;
            if gone == 0 {
                return Ok(0);
            }
            // Neither index follows the slice down: `search` is FTS5 and has no foreign
            // keys at all, and `embedding` hangs off the plan. Left alone, both would
            // keep answering `aip find` for a slice that no longer exists.
            tx.execute(
                "DELETE FROM search WHERE plan_id = ?1 AND kind = 'slice' AND ref = ?2",
                params![plan_id, key],
            )?;
            tx.execute(
                "DELETE FROM embedding WHERE plan_id = ?1 AND kind = 'slice' AND ref = ?2",
                params![plan_id, key],
            )?;
            tx.execute(
                "UPDATE search_state SET rows = (SELECT COUNT(*) FROM search) WHERE id = 1",
                [],
            )?;
            tx.execute(
                "UPDATE embedding_state SET rows = (SELECT COUNT(*) FROM embedding) WHERE id = 1",
                [],
            )?;
            tx.execute(
                "UPDATE plan SET updated_at = ?2 WHERE id = ?1",
                params![plan_id, now()],
            )?;
            // Against the plan rather than the slice, which is on its way out. A slice
            // that vanishes with no trace is how a plan starts lying about its history.
            super::notes::insert_log(
                tx,
                &me,
                plan_id,
                None,
                LogKind::Status,
                &format!("{key} deleted - {title}"),
            )?;
            Ok(gone)
        })?;

        if gone == 0 {
            // Either somebody claimed it under us, or it was already gone. Re-read to
            // say which, rather than guessing from the copy the caller handed in.
            let fresh = self.slice_by_id(id)?;
            return Err(Error::SliceIsHeld(
                fresh.key,
                fresh.claimed_by.unwrap_or_default(),
                fresh.worktree_path.unwrap_or_default(),
            ));
        }
        Ok(removal)
    }

    /// Take a slice for this worktree. The guard is the `WHERE` clause, not a read
    /// followed by a write, so two agents racing produce exactly one winner (D6).
    /// Re-claiming the same slice from the same worktree is a no-op, which keeps the
    /// command safe to re-run; the same agent in a *different* worktree is a genuine
    /// clash and is refused.
    pub fn claim_slice(
        &mut self,
        slice: &Slice,
        worktree: &str,
        branch: Option<&str>,
    ) -> Result<Slice> {
        let id = slice.id;
        let plan_id = slice.plan_id;
        let key = slice.key.clone();
        let worktree = worktree.to_string();
        let branch = branch.map(str::to_string);
        let me = self.actor.clone();
        let me_for_log = me.clone();
        let wt_for_log = worktree.clone();

        let claimed = self.db.write(|tx| {
            let n = tx.execute(
                "UPDATE slice SET
                     claimed_by    = ?2,
                     claimed_at    = ?3,
                     worktree_path = ?4,
                     -- Fill the branch in, never overwrite it: the plan may already
                     -- say which branch this slice is built on, and claiming from
                     -- somewhere else must not silently rewrite that. Use
                     -- `slice edit --branch` to change it on purpose.
                     branch        = COALESCE(branch, ?5),
                     status        = CASE WHEN status IN ('draft','ready') THEN 'active' ELSE status END,
                     started_at    = COALESCE(started_at, ?3),
                     updated_at    = ?3,
                     rev           = rev + 1
                 WHERE id = ?1
                   AND (claimed_by IS NULL OR (claimed_by = ?2 AND worktree_path = ?4))",
                params![id, me, now(), worktree, branch],
            )?;
            if n == 1 {
                tx.execute(
                    "UPDATE plan SET
                         status = CASE WHEN status IN ('draft','ready') THEN 'active' ELSE status END,
                         updated_at = ?2
                     WHERE id = ?1",
                    params![plan_id, now()],
                )?;
                super::notes::insert_log(
                    tx,
                    &me_for_log,
                    plan_id,
                    Some(id),
                    LogKind::Status,
                    &format!("{key} claimed by {me_for_log} in {wt_for_log}"),
                )?;
            }
            Ok(n == 1)
        })?;

        let fresh = self.slice_by_id(id)?;
        if !claimed {
            return Err(Error::AlreadyClaimed(
                fresh.key.clone(),
                fresh.claimed_by.clone().unwrap_or_default(),
                fresh.worktree_path.clone().unwrap_or_default(),
            ));
        }
        Ok(fresh)
    }

    pub fn release_slice(&mut self, slice: &Slice) -> Result<Slice> {
        let id = slice.id;
        let plan_id = slice.plan_id;
        let key = slice.key.clone();
        let me = self.actor.clone();
        self.db.write(|tx| {
            tx.execute(
                "UPDATE slice SET claimed_by = NULL, claimed_at = NULL, updated_at = ?2, rev = rev + 1
                 WHERE id = ?1",
                params![id, now()],
            )?;
            super::notes::insert_log(
                tx,
                &me,
                plan_id,
                Some(id),
                LogKind::Status,
                &format!("{key} released"),
            )?;
            Ok(())
        })?;
        self.slice_by_id(id)
    }

    /// Slices claimed in a worktree that no longer exists on disk. Reported by
    /// `aip doctor` rather than cleaned up automatically, because the work may be
    /// real even when the checkout is gone.
    pub fn stale_claims(&self) -> Result<Vec<Slice>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(&format!(
            "{SLICE_SELECT} WHERE claimed_by IS NOT NULL AND status NOT IN ('done','deferred')"
        ))?;
        let rows = stmt.query_map([], row_to_slice)?;
        let all: Vec<Slice> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(all
            .into_iter()
            .filter(|s| {
                s.worktree_path
                    .as_ref()
                    .is_some_and(|p| !std::path::Path::new(p).exists())
            })
            .collect())
    }
}
