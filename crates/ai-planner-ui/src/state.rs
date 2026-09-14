//! What every handler shares: the store, and the token that guards it.

use std::sync::{Arc, Mutex};

use ai_planner_core::{Result as CoreResult, Store};

use crate::error::{Error, Result};

/// `Store` needs `&mut self` to write, so it is behind a mutex rather than cloned per
/// request. That is not a bottleneck to design around: every operation is a local
/// SQLite query measured in microseconds, and SQLite serialises writes anyway - the
/// mutex only moves the waiting from `busy_timeout` to the process that caused it.
#[derive(Clone)]
pub struct AppState {
    store: Arc<Mutex<Store>>,
    token: Arc<str>,
}

impl AppState {
    pub fn new(store: Store, token: impl Into<Arc<str>>) -> AppState {
        AppState {
            store: Arc::new(Mutex::new(store)),
            token: token.into(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// Borrow the store for a read. The closure is synchronous on purpose: holding a
    /// `std::sync::MutexGuard` across an `.await` is a deadlock waiting to happen, and
    /// a sync closure makes that impossible to write by accident.
    pub fn read<T>(&self, f: impl FnOnce(&Store) -> CoreResult<T>) -> Result<T> {
        let guard = self.store.lock().map_err(|_| poisoned())?;
        Ok(f(&guard)?)
    }

    pub fn write<T>(&self, f: impl FnOnce(&mut Store) -> CoreResult<T>) -> Result<T> {
        let mut guard = self.store.lock().map_err(|_| poisoned())?;
        Ok(f(&mut guard)?)
    }
}

/// A poisoned lock means a handler panicked mid-request. The database is still
/// consistent - `Db::write` is transactional - so this reports rather than aborts.
fn poisoned() -> Error {
    Error::Internal(ai_planner_core::Error::invalid(
        "the planner lock was poisoned by an earlier panic - restart `aip ui`",
    ))
}
