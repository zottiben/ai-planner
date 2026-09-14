//! One error shape for the whole API.
//!
//! The refusals `ai-planner-core` produces are the interesting ones - a claim held by
//! another worktree, a `rev` that moved under an edit - and they carry the facts a
//! person needs. Flattening them into "500 internal error" would throw exactly the
//! information the board exists to show, so each one keeps its own status and its own
//! machine-readable `code`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use ai_planner_core::Error as CoreError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    BadRequest(String),

    /// Somebody else got there first. The body names them.
    #[error("{message}")]
    Claimed {
        message: String,
        slice: String,
        holder: String,
        worktree: String,
    },

    /// The row moved under a read-modify-write.
    #[error("{0}")]
    Conflict(String),

    #[error("missing or wrong token")]
    Unauthorized,

    #[error(transparent)]
    Internal(CoreError),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Error::BadRequest(msg.into())
    }

    fn status(&self) -> StatusCode {
        match self {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::Claimed { .. } | Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Internal(_) | Error::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// A stable string the client switches on, so the UI never parses a message.
    fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::BadRequest(_) => "bad_request",
            Error::Claimed { .. } => "already_claimed",
            Error::Conflict(_) => "conflict",
            Error::Unauthorized => "unauthorized",
            Error::Internal(_) | Error::Io(_) => "internal",
        }
    }
}

/// Core's errors already distinguish "you asked for something that is not there" from
/// "the database broke", so the mapping is by variant rather than by string matching.
impl From<CoreError> for Error {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::NoSuchPlan(_) | CoreError::NoSuchSlice(_, _) | CoreError::UnknownRepo(_) => {
                Error::NotFound(err.to_string())
            }
            CoreError::Invalid(_)
            | CoreError::AmbiguousPlan(_, _, _)
            | CoreError::DuplicatePlan(_)
            | CoreError::DuplicateSlice(_, _) => Error::BadRequest(err.to_string()),
            CoreError::Conflict(_, _, _) | CoreError::PlanIsHeld(_, _, _) => {
                Error::Conflict(err.to_string())
            }
            CoreError::AlreadyClaimed(ref slice, ref holder, ref worktree) => Error::Claimed {
                message: err.to_string(),
                slice: slice.clone(),
                holder: holder.clone(),
                worktree: worktree.clone(),
            },
            other => Error::Internal(other),
        }
    }
}

#[derive(Debug, Serialize)]
struct Body {
    error: String,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    slice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    holder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree: Option<String>,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (slice, holder, worktree) = match &self {
            Error::Claimed {
                slice,
                holder,
                worktree,
                ..
            } => (
                Some(slice.clone()),
                Some(holder.clone()),
                Some(worktree.clone()),
            ),
            _ => (None, None, None),
        };
        let body = Body {
            error: self.to_string(),
            code: self.code(),
            slice,
            holder,
            worktree,
        };
        (self.status(), Json(body)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;
