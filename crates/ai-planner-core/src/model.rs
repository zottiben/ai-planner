use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// One vocabulary for plans and slices alike. `incomplete` from the ask is not a
/// stored value: it is the filter `ready|active|in_review|blocked`, because an agent
/// needs to know whether to resume something or to begin it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Draft,
    Ready,
    Active,
    InReview,
    Blocked,
    Done,
    Deferred,
}

impl Status {
    pub const ALL: [Status; 7] = [
        Status::Draft,
        Status::Ready,
        Status::Active,
        Status::InReview,
        Status::Blocked,
        Status::Done,
        Status::Deferred,
    ];

    /// The statuses meant by "incomplete": agreed work that is not finished and not
    /// consciously dropped. This is the `--incomplete` filter, and it deliberately
    /// excludes drafts - a plan still being written is not outstanding work.
    pub const INCOMPLETE: [Status; 4] = [
        Status::Ready,
        Status::Active,
        Status::InReview,
        Status::Blocked,
    ];

    /// Anything not finished or dropped, drafts included. This is what resolution
    /// considers a live candidate: a plan you started writing five seconds ago is
    /// unquestionably the plan you are on.
    pub const UNFINISHED: [Status; 5] = [
        Status::Draft,
        Status::Ready,
        Status::Active,
        Status::InReview,
        Status::Blocked,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Ready => "ready",
            Status::Active => "active",
            Status::InReview => "in_review",
            Status::Blocked => "blocked",
            Status::Done => "done",
            Status::Deferred => "deferred",
        }
    }

    /// Accepts the stored form plus the shorthands people and agents actually type.
    pub fn parse(s: &str) -> Result<Status> {
        let norm = s.trim().to_lowercase().replace(['-', ' '], "_");
        Ok(match norm.as_str() {
            "draft" => Status::Draft,
            "ready" | "todo" | "planned" | "open" => Status::Ready,
            "active" | "in_progress" | "wip" | "started" | "doing" => Status::Active,
            "in_review" | "review" | "pr" => Status::InReview,
            "blocked" => Status::Blocked,
            "done" | "complete" | "completed" | "delivered" | "shipped" => Status::Done,
            "deferred" | "dropped" | "later" | "optional" | "wontdo" => Status::Deferred,
            _ => {
                let all = Status::ALL
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(Error::invalid(format!(
                    "unknown status {s:?} - use one of: {all}"
                )));
            }
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Done | Status::Deferred)
    }

    /// A short marker for CLI tables, matching how the source plans already mark up
    /// their slices.
    pub fn marker(self) -> &'static str {
        match self {
            Status::Draft => "·",
            Status::Ready => "○",
            Status::Active => "▶",
            Status::InReview => "◐",
            Status::Blocked => "⛔",
            Status::Done => "✔",
            Status::Deferred => "—",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl rusqlite::ToSql for Status {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::from(self.as_str()))
    }
}

impl rusqlite::types::FromSql for Status {
    fn column_result(v: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        let s = v.as_str()?;
        Status::parse(s).map_err(|_| rusqlite::types::FromSqlError::InvalidType)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Proposed,
    Agreed,
    Superseded,
    Rejected,
}

impl DecisionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DecisionStatus::Proposed => "proposed",
            DecisionStatus::Agreed => "agreed",
            DecisionStatus::Superseded => "superseded",
            DecisionStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().as_str() {
            "proposed" => DecisionStatus::Proposed,
            "agreed" | "accepted" => DecisionStatus::Agreed,
            "superseded" => DecisionStatus::Superseded,
            "rejected" => DecisionStatus::Rejected,
            _ => return Err(Error::invalid(format!("unknown decision status {s:?}"))),
        })
    }
}

impl std::fmt::Display for DecisionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a section emits after its body. Anything other than `Body` splices in a
/// generated block, which is how a rendered plan keeps the shape of the document it
/// was imported from (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Renders {
    Body,
    Sources,
    Decisions,
    Slices,
    Questions,
    Gotchas,
    Log,
}

impl Renders {
    pub fn as_str(self) -> &'static str {
        match self {
            Renders::Body => "body",
            Renders::Sources => "sources",
            Renders::Decisions => "decisions",
            Renders::Slices => "slices",
            Renders::Questions => "questions",
            Renders::Gotchas => "gotchas",
            Renders::Log => "log",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().as_str() {
            "body" | "" => Renders::Body,
            "sources" => Renders::Sources,
            "decisions" => Renders::Decisions,
            "slices" | "prs" | "milestones" => Renders::Slices,
            "questions" => Renders::Questions,
            "gotchas" => Renders::Gotchas,
            "log" | "progress" => Renders::Log,
            _ => return Err(Error::invalid(format!("unknown section kind {s:?}"))),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    Progress,
    Status,
    Decision,
    Gotcha,
    Verification,
    Blocker,
    Handoff,
}

impl LogKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LogKind::Progress => "progress",
            LogKind::Status => "status",
            LogKind::Decision => "decision",
            LogKind::Gotcha => "gotcha",
            LogKind::Verification => "verification",
            LogKind::Blocker => "blocker",
            LogKind::Handoff => "handoff",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().as_str() {
            "progress" | "" => LogKind::Progress,
            "status" => LogKind::Status,
            "decision" => LogKind::Decision,
            "gotcha" => LogKind::Gotcha,
            "verification" | "verified" => LogKind::Verification,
            "blocker" | "blocked" => LogKind::Blocker,
            "handoff" => LogKind::Handoff,
            _ => return Err(Error::invalid(format!("unknown log kind {s:?}"))),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub remote_url: Option<String>,
    pub main_path: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: i64,
    pub repo_id: i64,
    pub repo_name: String,
    pub slug: String,
    pub title: String,
    pub status: Status,
    pub summary: Option<String>,
    pub ticket_key: Option<String>,
    pub ticket_url: Option<String>,
    pub base_branch: Option<String>,
    pub owner: Option<String>,
    pub source_path: Option<String>,
    pub rev: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub id: i64,
    pub plan_id: i64,
    pub ord: i64,
    pub key: String,
    pub title: String,
    pub body: String,
    pub renders: Renders,
    pub rev: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: i64,
    pub plan_id: i64,
    pub ord: i64,
    pub key: String,
    pub title: String,
    pub body: String,
    pub status: DecisionStatus,
    pub superseded_by: Option<String>,
    pub supersede_note: Option<String>,
    pub rev: i64,
    pub decided_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slice {
    pub id: i64,
    pub plan_id: i64,
    pub ord: i64,
    pub key: String,
    pub title: String,
    pub status: Status,
    pub scope_md: String,
    pub demo_md: Option<String>,
    pub estimate_files: Option<i64>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub pr_url: Option<String>,
    pub worktree_path: Option<String>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<String>,
    pub blocked_reason: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub rev: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: i64,
    pub plan_id: i64,
    pub slice_key: Option<String>,
    pub body: String,
    pub status: String,
    pub answer: Option<String>,
    pub asked_at: String,
    pub answered_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gotcha {
    pub id: i64,
    pub plan_id: i64,
    pub title: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: i64,
    pub plan_id: i64,
    pub slice_key: Option<String>,
    pub at: String,
    pub actor: Option<String>,
    pub kind: LogKind,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSource {
    pub id: i64,
    pub kind: String,
    pub reference: String,
    pub note: Option<String>,
}

/// Everything needed to render a plan document in one shot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanBundle {
    pub plan: Plan,
    pub sources: Vec<PlanSource>,
    pub sections: Vec<Section>,
    pub decisions: Vec<Decision>,
    pub slices: Vec<Slice>,
    pub questions: Vec<Question>,
    pub gotchas: Vec<Gotcha>,
    pub log: Vec<LogEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_accepts_what_agents_actually_type() {
        assert_eq!(Status::parse("DONE").unwrap(), Status::Done);
        assert_eq!(Status::parse("in progress").unwrap(), Status::Active);
        assert_eq!(Status::parse("in-review").unwrap(), Status::InReview);
        assert_eq!(Status::parse("delivered").unwrap(), Status::Done);
        assert_eq!(Status::parse("optional").unwrap(), Status::Deferred);
        assert!(Status::parse("nearly").is_err());
    }

    #[test]
    fn incomplete_covers_started_and_not_started_but_not_terminal() {
        for s in Status::INCOMPLETE {
            assert!(!s.is_terminal(), "{s} should not be terminal");
        }
        assert!(!Status::INCOMPLETE.contains(&Status::Draft));
        assert!(!Status::INCOMPLETE.contains(&Status::Done));
        assert!(!Status::INCOMPLETE.contains(&Status::Deferred));
    }
}
