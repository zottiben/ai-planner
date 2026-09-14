//! The read half of the API.
//!
//! Every handler is a thin call into `Store`. The JSON is the model types serialising
//! themselves, so there is no DTO layer to fall out of step with the schema - change a
//! field in `model.rs` and the wire changes with it, which is the point.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use ai_planner_core::{
    Board, BoardColumn, PlanBundle, PlanFilter, PlanSummary, RepoSummary, SliceDetail, Status,
};

use crate::error::{Error, Result};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/meta", get(meta))
        .route("/repos", get(repos))
        .route("/repos/{id}/board", get(repo_board))
        .route("/plans", get(plans))
        .route("/plans/{id}", get(plan))
        .route("/plans/{id}/board", get(board))
        .route("/plans/{id}/log", get(plan_log))
        .route("/slices/{id}", get(slice))
}

/// What the client needs before it renders anything: which database it is looking at,
/// and the status vocabulary. Sending the statuses rather than hardcoding them in TS
/// means a new status in the `CHECK` constraint shows up as a column without a
/// frontend release.
#[derive(Debug, Serialize)]
struct Meta {
    database: String,
    actor: String,
    statuses: Vec<StatusMeta>,
    version: &'static str,
}

#[derive(Debug, Serialize)]
struct StatusMeta {
    value: Status,
    label: String,
    marker: &'static str,
    terminal: bool,
}

async fn meta(State(state): State<AppState>) -> Result<Json<Meta>> {
    let (database, actor) = state.read(|store| {
        Ok((
            store.path().to_string_lossy().to_string(),
            store.actor().to_string(),
        ))
    })?;
    Ok(Json(Meta {
        database,
        actor,
        statuses: Status::ALL
            .iter()
            .map(|s| StatusMeta {
                value: *s,
                label: label(*s),
                marker: s.marker(),
                terminal: s.is_terminal(),
            })
            .collect(),
        version: env!("CARGO_PKG_VERSION"),
    }))
}

/// `in_review` is a database value; "In review" is what a person reads.
fn label(status: Status) -> String {
    let raw = status.as_str().replace('_', " ");
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => raw,
    }
}

async fn repos(State(state): State<AppState>) -> Result<Json<Vec<RepoSummary>>> {
    Ok(Json(state.read(|store| store.repo_summaries())?))
}

#[derive(Debug, Default, Deserialize)]
pub struct PlanQuery {
    repo: Option<i64>,
    /// Ready, active, in review or blocked - agreed work that is not finished.
    #[serde(default)]
    incomplete: bool,
    /// Anything not terminal, drafts included.
    #[serde(default)]
    unfinished: bool,
    q: Option<String>,
}

async fn plans(
    State(state): State<AppState>,
    Query(query): Query<PlanQuery>,
) -> Result<Json<Vec<PlanSummary>>> {
    let filter = PlanFilter {
        repo_id: query.repo,
        statuses: match (query.incomplete, query.unfinished) {
            (true, _) => Status::INCOMPLETE.to_vec(),
            (_, true) => Status::UNFINISHED.to_vec(),
            _ => Vec::new(),
        },
        query: query.q.filter(|q| !q.trim().is_empty()),
    };
    Ok(Json(state.read(|store| store.plan_summaries(&filter))?))
}

/// The whole plan in one request - sections, decisions, slices, questions, gotchas and
/// the log. It is what `aip show` renders, and the rundown pane needs all of it, so
/// splitting it into six round trips would buy nothing.
async fn plan(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<PlanBundle>> {
    Ok(Json(state.read(|store| store.bundle(id))?))
}

async fn board(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Board>> {
    Ok(Json(state.read(|store| store.board(id))?))
}

async fn repo_board(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<BoardColumn>>> {
    Ok(Json(state.read(|store| store.repo_board(id))?))
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    limit: Option<i64>,
}

async fn plan_log(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<LogQuery>,
) -> Result<Json<Vec<ai_planner_core::LogEntry>>> {
    let limit = clamp_limit(query.limit)?;
    Ok(Json(state.read(|store| store.log(id, limit))?))
}

async fn slice(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<SliceDetail>> {
    Ok(Json(state.read(|store| store.slice_detail(id, Some(200)))?))
}

/// The log is append-only and grows forever, so an unbounded limit is a foot-gun a
/// long-running board would eventually find.
fn clamp_limit(limit: Option<i64>) -> Result<Option<i64>> {
    match limit {
        None => Ok(Some(200)),
        Some(n) if n <= 0 => Err(Error::bad_request("limit must be positive")),
        Some(n) => Ok(Some(n.min(2000))),
    }
}
