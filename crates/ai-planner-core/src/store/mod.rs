//! The write and read API over the planner database.
//!
//! Concurrency rules live here rather than in callers (D4): every mutation runs in an
//! `IMMEDIATE` transaction, mutable text is guarded by a `rev`, and progress is only
//! ever appended.

mod notes;
mod plans;
mod slices;

pub use notes::{NewDecision, NewLog};
pub use plans::{NewPlan, PlanFilter, PlanUpdate, SectionWrite};
pub use slices::{NewSlice, SliceUpdate};

use std::path::Path;

use rusqlite::OptionalExtension;

use crate::db::{default_db_path, Db};
use crate::error::Result;
use crate::git::GitContext;
use crate::model::Repo;
use crate::util::now;

pub struct Store {
    db: Db,
    actor: String,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        Ok(Store {
            db: Db::open(path)?,
            actor: default_actor(),
        })
    }

    pub fn open_default() -> Result<Store> {
        Store::open(&default_db_path())
    }

    /// Create the database if it is not there yet. Only `aip init` should call this;
    /// everything else uses `open` so a bad path fails loudly instead of quietly
    /// starting a second planner.
    pub fn init(path: &Path) -> Result<Store> {
        Ok(Store {
            db: Db::open_or_create(path)?,
            actor: default_actor(),
        })
    }

    /// Who this store attributes its writes to. Held per store rather than read from
    /// the environment at each write, so two stores in one process (tests, and the
    /// MCP server serving two clients) cannot see each other's identity.
    pub fn actor(&self) -> &str {
        &self.actor
    }

    pub fn set_actor(&mut self, actor: impl Into<String>) {
        let actor = actor.into();
        if !actor.trim().is_empty() {
            self.actor = actor;
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn path(&self) -> &Path {
        self.db.path()
    }

    // -- repos ------------------------------------------------------------------

    /// Register the repo this context points at, or refresh what we know about it.
    /// Called from every worktree of the repo and converges on one row (D2).
    pub fn ensure_repo(&mut self, ctx: &GitContext) -> Result<Repo> {
        let key = ctx.repo_key.clone();
        let name = ctx.repo_name.clone();
        let remote = ctx.remote_url.clone();
        let main = ctx.main_path.to_string_lossy().to_string();

        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO repo (key, name, remote_url, main_path, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(key) DO UPDATE SET
                     name       = excluded.name,
                     remote_url = COALESCE(excluded.remote_url, repo.remote_url),
                     main_path  = COALESCE(excluded.main_path, repo.main_path)",
                rusqlite::params![key, name, remote, main, now()],
            )?;
            Ok(())
        })?;

        self.find_repo(&ctx.repo_key)?
            .ok_or_else(|| crate::error::Error::UnknownRepo(ctx.repo_key.clone()))
    }

    pub fn find_repo(&self, key: &str) -> Result<Option<Repo>> {
        Ok(self
            .db
            .conn()
            .query_row(
                "SELECT id, key, name, remote_url, main_path, created_at FROM repo WHERE key = ?1",
                [key],
                row_to_repo,
            )
            .optional()?)
    }

    pub fn repos(&self) -> Result<Vec<Repo>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT id, key, name, remote_url, main_path, created_at FROM repo ORDER BY name",
        )?;
        let rows = stmt.query_map([], row_to_repo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn row_to_repo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: row.get(0)?,
        key: row.get(1)?,
        name: row.get(2)?,
        remote_url: row.get(3)?,
        main_path: row.get(4)?,
        created_at: row.get(5)?,
    })
}

/// Who is writing, by default. Agents set `AI_PLANNER_ACTOR` (the MCP server sets it
/// from the client name); humans fall back to `$USER`.
pub fn default_actor() -> String {
    std::env::var("AI_PLANNER_ACTOR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("USER").ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
