use std::path::{Path, PathBuf};

use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::error::{Error, Result};
use crate::util::now;

const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "core", include_str!("migrations/001_core.sql")),
    (2, "search", include_str!("migrations/002_search.sql")),
    (3, "embedding", include_str!("migrations/003_embedding.sql")),
    (4, "nudge", include_str!("migrations/004_nudge.sql")),
];

/// One database for every repo and every worktree (D1). Override for tests, or for
/// anyone who wants a project-local file.
pub const DB_ENV: &str = "AI_PLANNER_DB";

pub fn default_db_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os(DB_ENV) {
        return PathBuf::from(explicit);
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("ai-planner").join("planner.db");
    }
    home().join(".ai-planner").join("planner.db")
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub struct Db {
    conn: Connection,
    path: PathBuf,
}

impl Db {
    /// Open an existing database. Fails rather than creating one, so a typo in
    /// `AI_PLANNER_DB` cannot silently produce a second, empty planner.
    pub fn open(path: &Path) -> Result<Db> {
        if !path.exists() {
            return Err(Error::NoDatabase(path.to_path_buf()));
        }
        Db::open_or_create(path)
    }

    pub fn open_or_create(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        configure(&conn)?;
        let mut db = Db {
            conn,
            path: path.to_path_buf(),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn open_default() -> Result<Db> {
        Db::open(&default_db_path())
    }

    #[cfg(test)]
    pub fn memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        configure(&conn)?;
        let mut db = Db {
            conn,
            path: PathBuf::from(":memory:"),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Every write goes through here. `IMMEDIATE` takes the write lock up front so a
    /// concurrent writer waits on `busy_timeout` instead of failing halfway through a
    /// transaction with SQLITE_BUSY (D4).
    pub fn write<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version    INTEGER PRIMARY KEY,
                 name       TEXT NOT NULL,
                 applied_at TEXT NOT NULL
             )",
        )?;
        let applied: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;

        for (version, name, sql) in MIGRATIONS {
            if *version <= applied {
                continue;
            }
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![version, name, now()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn pending_migrations(&self) -> Result<i64> {
        let latest = MIGRATIONS.iter().map(|(v, _, _)| *v).max().unwrap_or(0);
        Ok(latest - self.schema_version()?)
    }
}

fn configure(conn: &Connection) -> Result<()> {
    // WAL lets readers run while a writer holds the lock, which is the normal state
    // when four worktrees are busy. `busy_timeout` turns a lock collision into a short
    // wait instead of an error the agent has to handle.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent_and_create_the_views() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.db");

        let db = Db::open_or_create(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), 4);
        assert_eq!(db.pending_migrations().unwrap(), 0);
        drop(db);

        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), 4);

        let views: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name LIKE 'v_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(views, 5, "the TablePlus views must ship with the schema");
    }

    #[test]
    fn opening_a_missing_database_is_an_error_not_a_new_one() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nope.db");
        assert!(matches!(Db::open(&path), Err(Error::NoDatabase(_))));
        assert!(!path.exists());
    }

    #[test]
    fn the_log_table_refuses_updates() {
        let mut db = Db::memory().unwrap();
        db.write(|tx| {
            tx.execute(
                "INSERT INTO repo (key, name, created_at) VALUES ('k', 'n', '2026-01-01T00:00:00Z')",
                [],
            )?;
            tx.execute(
                "INSERT INTO plan (repo_id, slug, title, created_at, updated_at)
                 VALUES (1, 's', 't', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [],
            )?;
            tx.execute(
                "INSERT INTO log (plan_id, at, body) VALUES (1, '2026-01-01T00:00:00Z', 'first')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        let err = db
            .conn()
            .execute("UPDATE log SET body = 'rewritten' WHERE id = 1", []);
        assert!(err.is_err(), "log rows must not be rewritable");

        let body: String = db
            .conn()
            .query_row("SELECT body FROM log WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "first");
    }

    #[test]
    fn deleting_a_plan_still_cascades_to_its_log() {
        let mut db = Db::memory().unwrap();
        db.write(|tx| {
            tx.execute(
                "INSERT INTO repo (key, name, created_at) VALUES ('k', 'n', '2026-01-01T00:00:00Z')",
                [],
            )?;
            tx.execute(
                "INSERT INTO plan (repo_id, slug, title, created_at, updated_at)
                 VALUES (1, 's', 't', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [],
            )?;
            tx.execute(
                "INSERT INTO log (plan_id, at, body) VALUES (1, '2026-01-01T00:00:00Z', 'x')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        db.write(|tx| {
            tx.execute("DELETE FROM plan WHERE id = 1", [])?;
            Ok(())
        })
        .unwrap();

        let logs: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(logs, 0);
    }

    #[test]
    fn the_status_check_constraint_rejects_invented_statuses() {
        let mut db = Db::memory().unwrap();
        db.write(|tx| {
            tx.execute(
                "INSERT INTO repo (key, name, created_at) VALUES ('k', 'n', '2026-01-01T00:00:00Z')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        let bad = db.conn().execute(
            "INSERT INTO plan (repo_id, slug, title, status, created_at, updated_at)
             VALUES (1, 's', 't', 'nearly', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        );
        assert!(bad.is_err());
    }
}
