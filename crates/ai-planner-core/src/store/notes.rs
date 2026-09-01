use rusqlite::{params, OptionalExtension, Transaction};

use super::plans::{row_to_decision, row_to_gotcha, row_to_log, row_to_question};
use super::Store;
use crate::error::{Error, Result};
use crate::model::{Decision, DecisionStatus, Gotcha, LogEntry, LogKind, Question};
use crate::util::now;

#[derive(Debug, Clone, Default)]
pub struct NewDecision {
    pub plan_id: i64,
    pub key: Option<String>,
    pub title: String,
    pub body: String,
    pub status: Option<DecisionStatus>,
    pub ord: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct NewLog {
    pub plan_id: i64,
    pub slice_id: Option<i64>,
    pub kind: Option<LogKind>,
    pub body: String,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    /// Backdated entries, used by the importer to preserve a progress log's own dates.
    pub at: Option<String>,
}

/// The one place a log row is written. Everything that changes state calls this
/// inside its own transaction, so status history comes for free and cannot be lost.
pub(super) fn insert_log(
    tx: &Transaction<'_>,
    actor: &str,
    plan_id: i64,
    slice_id: Option<i64>,
    kind: LogKind,
    body: &str,
) -> rusqlite::Result<i64> {
    tx.execute(
        "INSERT INTO log (plan_id, slice_id, at, actor, kind, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![plan_id, slice_id, now(), actor, kind.as_str(), body],
    )?;
    Ok(tx.last_insert_rowid())
}

const LOG_SELECT: &str = "SELECT l.id, l.plan_id, s.key, l.at, l.actor, l.kind, l.branch,
            l.worktree_path, l.body
     FROM log l LEFT JOIN slice s ON s.id = l.slice_id";

impl Store {
    // -- decisions --------------------------------------------------------------

    /// Adds `D<n>` with the next free number when no key is given, matching how the
    /// source plans number their decisions.
    pub fn add_decision(&mut self, new: NewDecision) -> Result<Decision> {
        let plan_id = new.plan_id;
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::invalid("a decision needs a title"));
        }
        let status = new.status.unwrap_or(DecisionStatus::Agreed);
        let body = new.body;
        let explicit_key = new
            .key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        let me = self.actor.clone();

        let key = self.db.write(|tx| {
            let key = match explicit_key {
                Some(k) => k,
                None => {
                    let n: i64 = tx.query_row(
                        "SELECT COUNT(*) + 1 FROM decision WHERE plan_id = ?1",
                        [plan_id],
                        |r| r.get(0),
                    )?;
                    format!("D{n}")
                }
            };
            let ord = match new.ord {
                Some(o) => o,
                None => tx.query_row(
                    "SELECT COALESCE(MAX(ord), 0) + 10 FROM decision WHERE plan_id = ?1",
                    [plan_id],
                    |r| r.get::<_, i64>(0),
                )?,
            };
            tx.execute(
                "INSERT INTO decision (plan_id, ord, key, title, body, status, decided_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![plan_id, ord, key, title, body, status.as_str(), now()],
            )?;
            insert_log(tx, &me, plan_id, None, LogKind::Decision, &format!("{key} - {title}"))?;
            Ok(key)
        })?;

        self.decision(plan_id, &key)?
            .ok_or_else(|| Error::invalid(format!("decision {key} vanished")))
    }

    pub fn decision(&self, plan_id: i64, key: &str) -> Result<Option<Decision>> {
        Ok(self
            .db
            .conn()
            .query_row(
                "SELECT id, plan_id, ord, key, title, body, status, superseded_by, supersede_note,
                        rev, decided_at
                 FROM decision WHERE plan_id = ?1 AND key = ?2 COLLATE NOCASE",
                params![plan_id, key],
                row_to_decision,
            )
            .optional()?)
    }

    pub fn decisions(&self, plan_id: i64) -> Result<Vec<Decision>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT id, plan_id, ord, key, title, body, status, superseded_by, supersede_note,
                    rev, decided_at
             FROM decision WHERE plan_id = ?1 ORDER BY ord, id",
        )?;
        let rows = stmt.query_map([plan_id], row_to_decision)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Mark a decision superseded in place. The source plans do exactly this - D4's
    /// mechanism changed mid-build and the old text stayed readable - so the history
    /// is preserved rather than edited away.
    pub fn supersede_decision(
        &mut self,
        plan_id: i64,
        key: &str,
        by: &str,
        note: Option<&str>,
    ) -> Result<Decision> {
        let existing = self
            .decision(plan_id, key)?
            .ok_or_else(|| Error::invalid(format!("no decision {key}")))?;
        let (key, by, note) = (
            existing.key.clone(),
            by.to_string(),
            note.map(str::to_string),
        );
        let key_for_log = key.clone();
        let me = self.actor.clone();
        self.db.write(|tx| {
            tx.execute(
                "UPDATE decision SET status = 'superseded', superseded_by = ?3,
                     supersede_note = ?4, updated_at = ?5, rev = rev + 1
                 WHERE plan_id = ?1 AND key = ?2",
                params![plan_id, key, by, note, now()],
            )?;
            insert_log(
                tx,
                &me,
                plan_id,
                None,
                LogKind::Decision,
                &format!("{key_for_log} superseded by {by}"),
            )?;
            Ok(())
        })?;
        self.decision(plan_id, &key)?
            .ok_or_else(|| Error::invalid(format!("decision {key} vanished")))
    }

    // -- questions --------------------------------------------------------------

    pub fn add_question(&mut self, plan_id: i64, slice_id: Option<i64>, body: &str) -> Result<i64> {
        let body = body.trim().to_string();
        if body.is_empty() {
            return Err(Error::invalid("a question needs a body"));
        }
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO question (plan_id, slice_id, body, asked_at) VALUES (?1, ?2, ?3, ?4)",
                params![plan_id, slice_id, body, now()],
            )?;
            Ok(tx.last_insert_rowid())
        })
    }

    pub fn answer_question(&mut self, id: i64, answer: &str) -> Result<()> {
        let answer = answer.to_string();
        let n = self.db.write(|tx| {
            Ok(tx.execute(
                "UPDATE question SET status = 'answered', answer = ?2, answered_at = ?3
                 WHERE id = ?1 AND status = 'open'",
                params![id, answer, now()],
            )?)
        })?;
        if n == 0 {
            return Err(Error::invalid(format!("no open question {id}")));
        }
        Ok(())
    }

    pub fn questions(&self, plan_id: i64, open_only: bool) -> Result<Vec<Question>> {
        let conn = self.db.conn();
        let mut sql = String::from(
            "SELECT q.id, q.plan_id, s.key, q.body, q.status, q.answer, q.asked_at, q.answered_at
             FROM question q LEFT JOIN slice s ON s.id = q.slice_id
             WHERE q.plan_id = ?1",
        );
        if open_only {
            sql.push_str(" AND q.status = 'open'");
        }
        sql.push_str(" ORDER BY q.id");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([plan_id], row_to_question)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // -- gotchas ----------------------------------------------------------------

    pub fn add_gotcha(&mut self, plan_id: i64, title: &str, body: &str) -> Result<i64> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(Error::invalid("a gotcha needs a title"));
        }
        let body = body.to_string();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO gotcha (plan_id, title, body, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(plan_id, title) DO UPDATE SET body = excluded.body",
                params![plan_id, title, body, now()],
            )?;
            let id: i64 = tx.query_row(
                "SELECT id FROM gotcha WHERE plan_id = ?1 AND title = ?2",
                params![plan_id, title],
                |r| r.get(0),
            )?;
            Ok(id)
        })
    }

    pub fn gotchas(&self, plan_id: i64) -> Result<Vec<Gotcha>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT id, plan_id, title, body, created_at FROM gotcha WHERE plan_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([plan_id], row_to_gotcha)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // -- log --------------------------------------------------------------------

    pub fn append_log(&mut self, new: NewLog) -> Result<i64> {
        let body = new.body.trim().to_string();
        if body.is_empty() {
            return Err(Error::invalid("a log entry needs a body"));
        }
        let kind = new.kind.unwrap_or(LogKind::Progress);
        let at = new.at.unwrap_or_else(now);
        let me = self.actor.clone();
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO log (plan_id, slice_id, at, actor, kind, branch, worktree_path, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    new.plan_id,
                    new.slice_id,
                    at,
                    me,
                    kind.as_str(),
                    new.branch,
                    new.worktree_path,
                    body
                ],
            )?;
            tx.execute(
                "UPDATE plan SET updated_at = ?2 WHERE id = ?1",
                params![new.plan_id, now()],
            )?;
            Ok(tx.last_insert_rowid())
        })
    }

    /// Newest first, matching how the source plans write their progress logs.
    pub fn log(&self, plan_id: i64, limit: Option<i64>) -> Result<Vec<LogEntry>> {
        let conn = self.db.conn();
        let mut sql = format!("{LOG_SELECT} WHERE l.plan_id = ?1 ORDER BY l.at DESC, l.id DESC");
        if let Some(n) = limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([plan_id], row_to_log)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn slice_log(&self, slice_id: i64, limit: Option<i64>) -> Result<Vec<LogEntry>> {
        let conn = self.db.conn();
        let mut sql = format!("{LOG_SELECT} WHERE l.slice_id = ?1 ORDER BY l.at DESC, l.id DESC");
        if let Some(n) = limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([slice_id], row_to_log)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
