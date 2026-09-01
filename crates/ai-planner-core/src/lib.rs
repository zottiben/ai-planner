//! Build plans as rows, not as markdown files copied between worktrees.
//!
//! The whole surface is `Store`, which owns one SQLite database shared by every repo
//! and every worktree on the machine.

pub mod db;
pub mod error;
pub mod git;
pub mod handoff;
pub mod import;
pub mod model;
pub mod render;
pub mod resolve;
pub mod search;
pub mod store;
pub mod util;

pub use db::{default_db_path, Db, DB_ENV};
pub use error::{Error, Result};
pub use git::GitContext;
pub use handoff::{Gate, Handoff, NewHandoff};
pub use model::{
    Decision, DecisionStatus, Gotcha, LogEntry, LogKind, Plan, PlanBundle, PlanSource, Question,
    Renders, Repo, Section, Slice, Status,
};
pub use render::render_plan;
pub use resolve::{Resolution, Resolved, Rule, Unresolved};
pub use search::{Hit, SearchOptions};
pub use store::{
    default_actor, NewDecision, NewLog, NewPlan, NewSlice, PlanFilter, PlanUpdate, SectionWrite,
    SliceUpdate, Store,
};
