//! The write half of the API.
//!
//! Every mutation here goes through `Store`, which means the board inherits the rules
//! rather than re-deciding them (D1): the `IMMEDIATE` transaction, the SQL claim guard
//! that matches on actor *and* worktree, the `rev` check on mutable text, and a log row
//! appended inside the same transaction as the change it describes. There is no SQL in
//! this file, and there should never be.

use axum::extract::{Path, State};
use axum::routing::{patch, post};
use axum::{Json, Router};
use serde::Deserialize;

use ai_planner_core::{LogKind, NewLog, Slice, SliceUpdate, Status};

use crate::error::{Error, Result};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/slices/{id}", patch(edit_slice))
        .route("/slices/{id}/status", post(set_status))
        .route("/slices/{id}/claim", post(claim))
        .route("/slices/{id}/release", post(release))
        .route("/plans/{id}/log", post(add_log))
        .route("/plans/{id}/status", post(set_plan_status))
        .route("/questions/{id}/answer", post(answer))
}

#[derive(Debug, Deserialize)]
pub struct StatusBody {
    status: String,
    /// Required when moving to `blocked`. `set_slice_status` wants a reason, and a
    /// board that quietly blocks something with no reason is worse than the CLI.
    #[serde(default)]
    reason: Option<String>,
}

async fn set_status(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<StatusBody>,
) -> Result<Json<Slice>> {
    let status = Status::parse(&body.status).map_err(Error::from)?;
    let reason = body.reason.filter(|r| !r.trim().is_empty());
    if status == Status::Blocked && reason.is_none() {
        return Err(Error::bad_request("blocking a slice needs a reason"));
    }

    state
        .write(|store| {
            let slice = store.slice_by_id(id)?;
            store.set_slice_status(&slice, status, reason.as_deref())
        })
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub struct ClaimBody {
    /// Where the work will happen. The board runs on the machine the worktrees are on,
    /// so this is normally the plan's repo path - but it is the caller's to state,
    /// because claiming for the wrong worktree is what the guard exists to catch.
    worktree: Option<String>,
    branch: Option<String>,
}

async fn claim(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<ClaimBody>,
) -> Result<Json<Slice>> {
    state
        .write(|store| {
            let slice = store.slice_by_id(id)?;
            // Falling back to the repo's main checkout rather than to an empty string:
            // a claim with no worktree cannot be told apart from another, and rule 4's
            // guard matches on the pair.
            let worktree = match body.worktree.as_deref().filter(|w| !w.trim().is_empty()) {
                Some(w) => w.to_string(),
                None => {
                    let plan = store.get_plan(slice.plan_id)?;
                    store
                        .repo_by_id(plan.repo_id)?
                        .and_then(|repo| repo.main_path)
                        .ok_or_else(|| {
                            ai_planner_core::Error::invalid(
                                "this repo has no recorded checkout - claim from a worktree, \
                                 or pass one",
                            )
                        })?
                }
            };
            store.claim_slice(&slice, &worktree, body.branch.as_deref())
        })
        .map(Json)
}

async fn release(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Slice>> {
    state
        .write(|store| {
            let slice = store.slice_by_id(id)?;
            store.release_slice(&slice)
        })
        .map(Json)
}

/// A patch: every absent field is left alone, so two people editing different
/// attributes of one slice do not fight over the columns they are not touching.
#[derive(Debug, Default, Deserialize)]
pub struct SlicePatch {
    title: Option<String>,
    scope_md: Option<String>,
    demo_md: Option<String>,
    estimate_files: Option<i64>,
    branch: Option<String>,
    base_branch: Option<String>,
    pr_url: Option<String>,
}

async fn edit_slice(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<SlicePatch>,
) -> Result<Json<Slice>> {
    state
        .write(|store| {
            let slice = store.slice_by_id(id)?;
            store.update_slice(
                &slice,
                SliceUpdate {
                    title: body.title,
                    scope_md: body.scope_md,
                    demo_md: body.demo_md,
                    estimate_files: body.estimate_files,
                    branch: body.branch,
                    base_branch: body.base_branch,
                    pr_url: body.pr_url,
                    blocked_reason: None,
                },
            )
        })
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub struct LogBody {
    body: String,
    slice: Option<String>,
    kind: Option<String>,
}

async fn add_log(
    State(state): State<AppState>,
    Path(plan_id): Path<i64>,
    Json(body): Json<LogBody>,
) -> Result<Json<serde_json::Value>> {
    if body.body.trim().is_empty() {
        return Err(Error::bad_request("a note needs a body"));
    }
    let kind = match body.kind.as_deref() {
        Some(k) => LogKind::parse(k).map_err(Error::from)?,
        None => LogKind::Progress,
    };

    let id = state.write(|store| {
        let slice_id = match body.slice.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(key) => Some(store.require_slice(plan_id, key)?.id),
            None => None,
        };
        store.append_log(NewLog {
            plan_id,
            slice_id,
            kind: Some(kind),
            body: body.body.trim().to_string(),
            ..Default::default()
        })
    })?;

    Ok(Json(serde_json::json!({ "id": id })))
}

async fn set_plan_status(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<StatusBody>,
) -> Result<Json<ai_planner_core::Plan>> {
    let status = Status::parse(&body.status).map_err(Error::from)?;
    state
        .write(|store| {
            let plan = store.get_plan(id)?;
            store.set_plan_status(&plan, status)
        })
        .map(Json)
}

#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    answer: String,
}

async fn answer(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<serde_json::Value>> {
    if body.answer.trim().is_empty() {
        return Err(Error::bad_request("an answer needs a body"));
    }
    state.write(|store| store.answer_question(id, body.answer.trim()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
