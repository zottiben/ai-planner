use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("no database at {0} - run `aip init`")]
    NoDatabase(PathBuf),

    #[error("not a git repository (looked from {0})")]
    NotAGitRepo(PathBuf),

    #[error("git is not installed or failed to run: {0}")]
    Git(String),

    #[error("no repo registered for {0} - run `aip init`")]
    UnknownRepo(String),

    #[error("no plan matching {0:?}")]
    NoSuchPlan(String),

    #[error("{0:?} matches {1} plans: {2} - pass --plan to disambiguate")]
    AmbiguousPlan(String, usize, String),

    #[error("plan {0} has no slice {1:?}")]
    NoSuchSlice(String, String),

    #[error("plan {0} already has a slice {1:?}")]
    DuplicateSlice(String, String),

    #[error("a plan {0:?} already exists in this repo")]
    DuplicatePlan(String),

    #[error("{0} was changed by another writer since you read it (rev {1}, now {2}) - re-read and retry")]
    Conflict(String, i64, i64),

    #[error("slice {0} is already claimed by {1} in {2}")]
    AlreadyClaimed(String, String, String),

    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}
