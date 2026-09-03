//! The MCP server: the same operations as the CLI, as structured tool calls.
//!
//! One process serves one working directory, but the git context is re-detected on
//! every call rather than captured at start-up - an agent switches branches mid-session,
//! and a stale branch resolves to the wrong plan, which is the one mistake that matters
//! here (D8 of the plan: never guess).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use ai_planner_core::embed::{self, Embedder};
use ai_planner_core::handoff::{Gate, NewHandoff};
use ai_planner_core::import::{ImportOptions, Outcome};
use ai_planner_core::{
    render_plan, DecisionStatus, GitContext, LogKind, NewDecision, NewLog, NewPlan, NewSlice,
    PlanFilter, Renders, SearchOptions, SectionWrite, SliceUpdate, Status, Store,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};

struct Inner {
    store: Mutex<Store>,
    /// Where the server was launched. Tool calls may override it per call.
    root: PathBuf,
}

#[derive(Clone)]
pub struct PlannerServer {
    inner: Arc<Inner>,
    tool_router: ToolRouter<Self>,
}

fn to_err(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

fn json<T: Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let body = serde_json::to_string_pretty(value).map_err(to_err)?;
    Ok(CallToolResult::success(vec![ContentBlock::text(body)]))
}

fn text(body: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        body.into(),
    )]))
}

macro_rules! schema_args {
    ($(#[$meta:meta])* struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
        #[schemars(crate = "rmcp::schemars")]
        struct $name { $($body)* }
    };
}

schema_args! {
    struct WhereArgs {
        /// Working directory to resolve from. Defaults to where the server was started.
        #[serde(default)]
        cwd: Option<String>,
        /// Plan slug, ticket key or id. Omit to resolve it from the worktree.
        #[serde(default)]
        plan: Option<String>,
    }
}

schema_args! {
    struct SearchArgs {
        /// What to look for, in your own words.
        query: String,
        /// Every repo, not just the one you are in. Defaults to false.
        #[serde(default)]
        all_repos: Option<bool>,
        /// Maximum hits. Defaults to 12.
        #[serde(default)]
        limit: Option<usize>,
    }
}

schema_args! {
    struct SliceArgs {
        /// Slice key, e.g. PR2, S1, M4.
        key: String,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct SliceStatusArgs {
        key: String,
        /// draft | ready | active | in_review | blocked | done | deferred
        status: String,
        /// Why, when blocking. Recorded so the next session is not left guessing.
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct SliceEditArgs {
        key: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        demo: Option<String>,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        pr_url: Option<String>,
        #[serde(default)]
        estimate_files: Option<i64>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct AddSliceArgs {
        /// Slice key, e.g. PR1, S2, M4. Unique within the plan.
        key: String,
        title: String,
        /// What the slice covers, as markdown.
        #[serde(default)]
        scope: Option<String>,
        /// How to prove it works.
        #[serde(default)]
        demo: Option<String>,
        #[serde(default)]
        estimate_files: Option<i64>,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct LogArgs {
        /// What happened. Write it for the next session, not for yourself.
        body: String,
        /// Attach it to a slice.
        #[serde(default)]
        slice: Option<String>,
        /// progress | verification | blocker. Defaults to progress.
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct DecisionArgs {
        title: String,
        /// The reasoning. Later slices must not re-litigate it, so say why.
        #[serde(default)]
        body: Option<String>,
        /// Fixed key such as D4 or AD-2. Auto-numbered when omitted.
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct SupersedeArgs {
        /// The decision being replaced, e.g. D4.
        key: String,
        /// The decision replacing it.
        by: String,
        #[serde(default)]
        note: Option<String>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct GotchaArgs {
        /// A short, findable name for it.
        title: String,
        /// The detail: what bites, and what to do instead.
        body: String,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct QuestionArgs {
        /// Something only a human can decide.
        body: String,
        #[serde(default)]
        slice: Option<String>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct AnswerArgs {
        /// The question's id, from list_questions.
        id: i64,
        answer: String,
    }
}

schema_args! {
    struct SectionArgs {
        /// Section handle, e.g. grounding, outcome, risks.
        key: String,
        /// Markdown body.
        body: String,
        /// The heading to render, e.g. "2. Grounding".
        #[serde(default)]
        title: Option<String>,
        /// body | sources | decisions | slices | questions | gotchas | log
        #[serde(default)]
        renders: Option<String>,
        /// Refuse the write if the section changed since this revision. Pass the `rev`
        /// you read; the call fails rather than overwriting another agent's edit.
        #[serde(default)]
        expect_rev: Option<i64>,
        /// Append to the section instead of replacing it.
        #[serde(default)]
        append: Option<bool>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct CreatePlanArgs {
        /// e.g. "ACME-1234 - Reusable Date Range Picker"
        title: String,
        #[serde(default)]
        slug: Option<String>,
        #[serde(default)]
        ticket_url: Option<String>,
        #[serde(default)]
        base_branch: Option<String>,
        /// The blockquote under the title.
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct ListPlansArgs {
        /// Every repo, not just the one you are in.
        #[serde(default)]
        all_repos: Option<bool>,
        /// Only unfinished plans: ready, active, in_review, blocked.
        #[serde(default)]
        incomplete: Option<bool>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct HandoffArgs {
        /// Gate results as `name=result`, e.g. ["typecheck=pass", "test=pass:731 tests"].
        /// Record what you actually ran, failures included.
        #[serde(default)]
        gates: Option<Vec<String>>,
        /// The next concrete work items.
        #[serde(default)]
        next: Option<Vec<String>>,
        /// Anything the next session needs that the plan does not already say.
        #[serde(default)]
        notes: Option<String>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct SyncArgs {
        /// Apply what git says instead of only reporting it. Defaults to false.
        #[serde(default)]
        fix: Option<bool>,
        /// Skip the GitHub lookup and use branch history alone. Defaults to false.
        #[serde(default)]
        no_gh: Option<bool>,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
    }
}

schema_args! {
    struct ImportArgs {
        /// Files or directories holding BUILD_PLAN / HANDOFF markdown.
        paths: Vec<String>,
        /// Report what would happen without writing. Defaults to true - importing is
        /// worth a look before it lands.
        #[serde(default)]
        dry_run: Option<bool>,
    }
}

#[derive(Serialize)]
struct Located {
    plan: String,
    title: String,
    status: Status,
    slice: Option<String>,
    slice_status: Option<Status>,
    next_slice: Option<String>,
    next_title: Option<String>,
    done: usize,
    total: usize,
    open_questions: usize,
    rule: ai_planner_core::Rule,
    why: String,
    branch: Option<String>,
    worktree: String,
    has_handoff: bool,
}

impl PlannerServer {
    pub fn new(store: Store, root: PathBuf) -> Self {
        PlannerServer {
            inner: Arc::new(Inner {
                store: Mutex::new(store),
                root,
            }),
            tool_router: Self::tool_router(),
        }
    }

    fn store(&self) -> Result<MutexGuard<'_, Store>, ErrorData> {
        self.inner
            .store
            .lock()
            .map_err(|e| to_err(format!("store lock poisoned: {e}")))
    }

    /// Re-detect git state per call: branches change mid-session.
    fn git(&self, cwd: Option<&String>) -> Result<GitContext, ErrorData> {
        let dir = cwd
            .map(PathBuf::from)
            .unwrap_or_else(|| self.inner.root.clone());
        GitContext::detect(&dir).map_err(to_err)
    }

    /// The plan a call acts on, plus the git context it was resolved from.
    fn target(
        &self,
        store: &Store,
        cwd: Option<&String>,
        plan: Option<&String>,
    ) -> Result<(ai_planner_core::Plan, GitContext), ErrorData> {
        let git = self.git(cwd)?;
        match store
            .resolve(Some(&git), plan.map(String::as_str))
            .map_err(to_err)?
        {
            Ok(resolution) => Ok((resolution.plan, git)),
            Err(unresolved) => {
                let hint = if unresolved.candidates.is_empty() {
                    "call create_plan first".to_string()
                } else {
                    format!(
                        "pass `plan` with one of: {}",
                        unresolved
                            .candidates
                            .iter()
                            .map(|p| p.slug.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                Err(ErrorData::invalid_params(
                    format!("{} - {hint}", unresolved.reason),
                    None,
                ))
            }
        }
    }
}

#[tool_router]
impl PlannerServer {
    #[tool(
        description = "Which build plan this worktree is on, which slice you are on, what is next, and whether a handoff is waiting. Call this first in a session - it answers 'what am I building?' without reading any files."
    )]
    async fn locate(
        &self,
        Parameters(args): Parameters<WhereArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let git = self.git(args.cwd.as_ref())?;
        let resolved = store
            .resolve(Some(&git), args.plan.as_deref())
            .map_err(to_err)?;
        let resolution = match resolved {
            Ok(r) => r,
            Err(unresolved) => {
                return json(&serde_json::json!({
                    "resolved": false,
                    "reason": unresolved.reason,
                    "candidates": unresolved.candidates.iter()
                        .map(|p| serde_json::json!({ "plan": p.slug, "title": p.title }))
                        .collect::<Vec<_>>(),
                }))
            }
        };

        let plan = resolution.plan.clone();
        let worktree = git.worktree_str();
        let slices = store.slices(plan.id).map_err(to_err)?;
        let done = slices.iter().filter(|s| s.status == Status::Done).count();
        let next = store.next_slice(plan.id, Some(&worktree)).map_err(to_err)?;
        let open_questions = store.questions(plan.id, true).map_err(to_err)?.len();
        let has_handoff = store
            .latest_handoff(plan.id, &worktree)
            .map_err(to_err)?
            .is_some();

        // Confirming a resolution is exactly when the association is worth learning.
        if let Some(repo) = store.find_repo(&git.repo_key).map_err(to_err)? {
            store
                .record_affinity(plan.id, repo.id, git.branch.as_deref(), &worktree)
                .map_err(to_err)?;
        }

        json(&Located {
            plan: plan.slug.clone(),
            title: plan.title.clone(),
            status: plan.status,
            slice: resolution.slice.as_ref().map(|s| s.key.clone()),
            slice_status: resolution.slice.as_ref().map(|s| s.status),
            next_slice: next.as_ref().map(|s| s.key.clone()),
            next_title: next.as_ref().map(|s| s.title.clone()),
            done,
            total: slices.len(),
            open_questions,
            rule: resolution.rule,
            why: resolution.rule.why().to_string(),
            branch: git.branch.clone(),
            worktree,
            has_handoff,
        })
    }

    #[tool(
        description = "The full plan as markdown - scope, grounding, decisions, slices, open questions and progress log. This is the document that used to live in BUILD_PLAN.md. Read it before making design choices so you do not re-litigate a settled decision."
    )]
    async fn get_plan(
        &self,
        Parameters(args): Parameters<WhereArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let bundle = store.bundle(plan.id).map_err(to_err)?;
        text(render_plan(&bundle))
    }

    #[tool(
        description = "What a fresh session needs to pick this work up: the last handoff and its gate results, the next slice, who holds what in which worktree, the gotchas, and the open questions. Call this after a context clear."
    )]
    async fn get_resume(
        &self,
        Parameters(args): Parameters<WhereArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store()?;
        let (plan, git) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let md = store
            .render_resume(plan.id, Some(&git.worktree_str()))
            .map_err(to_err)?;
        text(md)
    }

    #[tool(
        description = "Search every plan by what it says - titles, sections, slices, decisions, gotchas, questions and progress notes. Use this to find a plan you half remember, or to check whether something has already been decided or hit before."
    )]
    async fn search_plans(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        if store.search_rows().map_err(to_err)? == 0 {
            store.reindex().map_err(to_err)?;
        }
        let repo_id = self
            .git(None)
            .ok()
            .and_then(|g| store.find_repo(&g.repo_key).ok().flatten())
            .map(|r| r.id);

        let embedder: Option<Box<dyn Embedder>> = match store.embedding_state().map_err(to_err)? {
            Some((model, _)) if embed::available() => embed::build(&model, None).ok(),
            _ => None,
        };

        let hits = store
            .search_with(
                &args.query,
                &SearchOptions {
                    prefer_repo: repo_id,
                    only_repo: if args.all_repos.unwrap_or(false) {
                        None
                    } else {
                        repo_id
                    },
                    statuses: Vec::new(),
                    limit: args.limit.unwrap_or(12),
                    lexical_only: false,
                },
                embedder.as_deref(),
            )
            .map_err(to_err)?;
        json(&hits)
    }

    #[tool(description = "List the plans in this repo, or across every repo.")]
    async fn list_plans(
        &self,
        Parameters(args): Parameters<ListPlansArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store()?;
        let repo_id = self
            .git(args.cwd.as_ref())
            .ok()
            .and_then(|g| store.find_repo(&g.repo_key).ok().flatten())
            .map(|r| r.id);
        let plans = store
            .list_plans(&PlanFilter {
                repo_id: if args.all_repos.unwrap_or(false) {
                    None
                } else {
                    repo_id
                },
                statuses: if args.incomplete.unwrap_or(false) {
                    Status::INCOMPLETE.to_vec()
                } else {
                    Vec::new()
                },
                query: None,
            })
            .map_err(to_err)?;
        json(&plans)
    }

    #[tool(
        description = "The plan's slices with their statuses, branches, PR links, and which worktree holds each one. Check this before starting work so you do not duplicate a slice another agent has claimed."
    )]
    async fn list_slices(
        &self,
        Parameters(args): Parameters<WhereArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        json(&store.slices(plan.id).map_err(to_err)?)
    }

    #[tool(description = "One slice in full: its scope, demo, status, branch and history.")]
    async fn get_slice(
        &self,
        Parameters(args): Parameters<SliceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let slice = store.require_slice(plan.id, &args.key).map_err(to_err)?;
        let history = store.slice_log(slice.id, Some(20)).map_err(to_err)?;
        json(&serde_json::json!({ "slice": slice, "history": history }))
    }

    #[tool(
        description = "Take a slice for this worktree before you start building it. Fails if another agent already holds it, which is the point - two agents must not build the same slice."
    )]
    async fn claim_slice(
        &self,
        Parameters(args): Parameters<SliceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, git) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let slice = store.require_slice(plan.id, &args.key).map_err(to_err)?;
        let claimed = store
            .claim_slice(&slice, &git.worktree_str(), git.branch.as_deref())
            .map_err(to_err)?;
        if let Some(repo) = store.find_repo(&git.repo_key).map_err(to_err)? {
            store
                .record_affinity(plan.id, repo.id, git.branch.as_deref(), &git.worktree_str())
                .map_err(to_err)?;
        }
        json(&claimed)
    }

    #[tool(description = "Give a claimed slice back so another worktree can take it.")]
    async fn release_slice(
        &self,
        Parameters(args): Parameters<SliceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let slice = store.require_slice(plan.id, &args.key).map_err(to_err)?;
        json(&store.release_slice(&slice).map_err(to_err)?)
    }

    #[tool(
        description = "Move a slice to a status: ready, active, in_review, blocked, done or deferred. Always give a reason when blocking. The change is logged, so status history is kept."
    )]
    async fn set_slice_status(
        &self,
        Parameters(args): Parameters<SliceStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let slice = store.require_slice(plan.id, &args.key).map_err(to_err)?;
        let status = Status::parse(&args.status).map_err(to_err)?;
        json(
            &store
                .set_slice_status(&slice, status, args.reason.as_deref())
                .map_err(to_err)?,
        )
    }

    #[tool(description = "Change a slice's title, scope, demo, branch, PR link or file estimate.")]
    async fn update_slice(
        &self,
        Parameters(args): Parameters<SliceEditArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let slice = store.require_slice(plan.id, &args.key).map_err(to_err)?;
        json(
            &store
                .update_slice(
                    &slice,
                    SliceUpdate {
                        title: args.title,
                        scope_md: args.scope,
                        demo_md: args.demo,
                        estimate_files: args.estimate_files,
                        branch: args.branch,
                        base_branch: None,
                        pr_url: args.pr_url,
                        blocked_reason: None,
                    },
                )
                .map_err(to_err)?,
        )
    }

    #[tool(description = "Add a vertical slice to the plan - one slice is one PR.")]
    async fn add_slice(
        &self,
        Parameters(args): Parameters<AddSliceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        json(
            &store
                .add_slice(NewSlice {
                    plan_id: plan.id,
                    key: args.key,
                    title: args.title,
                    status: None,
                    scope_md: args.scope,
                    demo_md: args.demo,
                    estimate_files: args.estimate_files,
                    branch: args.branch,
                    base_branch: plan.base_branch.clone(),
                    ord: None,
                })
                .map_err(to_err)?,
        )
    }

    #[tool(
        description = "Append a progress note. The log is append-only, so this never conflicts with what another agent is writing. Record as you go rather than at the end."
    )]
    async fn append_log(
        &self,
        Parameters(args): Parameters<LogArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, git) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let slice_id = match &args.slice {
            Some(key) => Some(store.require_slice(plan.id, key).map_err(to_err)?.id),
            None => None,
        };
        let id = store
            .append_log(NewLog {
                plan_id: plan.id,
                slice_id,
                kind: Some(
                    args.kind
                        .as_deref()
                        .map(LogKind::parse)
                        .transpose()
                        .map_err(to_err)?
                        .unwrap_or(LogKind::Progress),
                ),
                body: args.body,
                branch: git.branch.clone(),
                worktree_path: Some(git.worktree_str()),
                at: None,
            })
            .map_err(to_err)?;
        json(&serde_json::json!({ "id": id, "plan": plan.slug }))
    }

    #[tool(
        description = "Record a design decision with its reasoning, so later slices do not re-litigate it. Auto-numbered D1, D2, ... unless you pass a key."
    )]
    async fn add_decision(
        &self,
        Parameters(args): Parameters<DecisionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        json(
            &store
                .add_decision(NewDecision {
                    plan_id: plan.id,
                    key: args.key,
                    title: args.title,
                    body: args.body.unwrap_or_default(),
                    status: Some(DecisionStatus::Agreed),
                    ord: None,
                })
                .map_err(to_err)?,
        )
    }

    #[tool(
        description = "Mark a decision superseded when its reasoning stops holding. The original text is kept and annotated, never edited away."
    )]
    async fn supersede_decision(
        &self,
        Parameters(args): Parameters<SupersedeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        json(
            &store
                .supersede_decision(plan.id, &args.key, &args.by, args.note.as_deref())
                .map_err(to_err)?,
        )
    }

    #[tool(
        description = "Record something the code alone does not reveal - an API quirk, a verification trick, an environment fact - so the next session does not rediscover it. A durable project rule belongs in AGENTS.md instead."
    )]
    async fn add_gotcha(
        &self,
        Parameters(args): Parameters<GotchaArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let id = store
            .add_gotcha(plan.id, &args.title, &args.body)
            .map_err(to_err)?;
        json(&serde_json::json!({ "id": id, "plan": plan.slug }))
    }

    #[tool(
        description = "Raise something only a human can decide. It stays visible in the plan's status until answered - prefer this over guessing and building the wrong thing."
    )]
    async fn open_question(
        &self,
        Parameters(args): Parameters<QuestionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let slice_id = match &args.slice {
            Some(key) => Some(store.require_slice(plan.id, key).map_err(to_err)?.id),
            None => None,
        };
        let id = store
            .add_question(plan.id, slice_id, &args.body)
            .map_err(to_err)?;
        json(&serde_json::json!({ "id": id, "plan": plan.slug }))
    }

    #[tool(description = "The plan's questions, open ones first.")]
    async fn list_questions(
        &self,
        Parameters(args): Parameters<WhereArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        json(&store.questions(plan.id, false).map_err(to_err)?)
    }

    #[tool(description = "Answer an open question, once a human has decided.")]
    async fn answer_question(
        &self,
        Parameters(args): Parameters<AnswerArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        store
            .answer_question(args.id, &args.answer)
            .map_err(to_err)?;
        json(&serde_json::json!({ "id": args.id, "status": "answered" }))
    }

    #[tool(
        description = "Write a narrative section of the plan document - outcome, grounding, risks. Pass expect_rev with the rev you read to be refused rather than overwrite a concurrent edit."
    )]
    async fn update_section(
        &self,
        Parameters(args): Parameters<SectionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, _) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let body = if args.append.unwrap_or(false) {
            let existing = store
                .section(plan.id, &args.key)
                .map_err(to_err)?
                .map(|s| s.body)
                .unwrap_or_default();
            if existing.trim().is_empty() {
                args.body.clone()
            } else {
                format!("{}\n\n{}", existing.trim_end(), args.body.trim())
            }
        } else {
            args.body.clone()
        };
        let renders = args
            .renders
            .as_deref()
            .map(Renders::parse)
            .transpose()
            .map_err(to_err)?;
        json(
            &store
                .set_section(
                    plan.id,
                    SectionWrite {
                        key: &args.key,
                        title: args.title.as_deref(),
                        body: &body,
                        renders,
                        ord: None,
                        expect_rev: args.expect_rev,
                    },
                )
                .map_err(to_err)?,
        )
    }

    #[tool(description = "Start a new plan in this repo.")]
    async fn create_plan(
        &self,
        Parameters(args): Parameters<CreatePlanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let git = self.git(args.cwd.as_ref())?;
        let repo = store.ensure_repo(&git).map_err(to_err)?;
        let created = store
            .create_plan(NewPlan {
                repo_id: repo.id,
                title: args.title,
                slug: args.slug,
                status: Some(Status::Draft),
                summary: args.summary,
                ticket_key: None,
                ticket_url: args.ticket_url,
                base_branch: args.base_branch.or_else(|| git.branch.clone()),
                owner: None,
                raw_md: None,
                source_path: None,
                bare: false,
            })
            .map_err(to_err)?;
        store
            .record_affinity(
                created.id,
                repo.id,
                git.branch.as_deref(),
                &git.worktree_str(),
            )
            .map_err(to_err)?;
        json(&created)
    }

    #[tool(
        description = "Checkpoint this worktree before the context is cleared: the gates you ran with their real results, what to do first, and anything the plan does not already say. Record a failed gate as failed - never imply green over a failure."
    )]
    async fn write_handoff(
        &self,
        Parameters(args): Parameters<HandoffArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, git) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let worktree = git.worktree_str();
        let next = match args.next {
            Some(items) if !items.is_empty() => items
                .iter()
                .map(|i| format!("- {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => match store.next_slice(plan.id, Some(&worktree)).map_err(to_err)? {
                Some(s) => format!("{} - {}", s.key, s.title),
                None => String::new(),
            },
        };
        json(
            &store
                .write_handoff(NewHandoff {
                    plan_id: plan.id,
                    worktree_path: worktree,
                    branch: git.branch.clone(),
                    head_sha: git.head_sha.clone(),
                    gates: args
                        .gates
                        .unwrap_or_default()
                        .iter()
                        .map(|g| Gate::parse(g))
                        .collect(),
                    resume_md: args.notes.unwrap_or_default(),
                    next_md: next,
                })
                .map_err(to_err)?,
        )
    }

    #[tool(
        description = "Reconcile the plan against what git and GitHub already show: branches that have landed, PRs that are open or merged, claims on branches that no longer exist. Call this when you finish a slice or open a PR, and whenever a hook tells you the plan is out of step. Reports by default; pass fix=true to apply."
    )]
    async fn sync_plan(
        &self,
        Parameters(args): Parameters<SyncArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let (plan, git) = self.target(&store, args.cwd.as_ref(), args.plan.as_ref())?;
        let use_gh = !args.no_gh.unwrap_or(false) && ai_planner_core::git::gh_available();
        let findings = store.drift(plan.id, &git, use_gh).map_err(to_err)?;

        let applied = if args.fix.unwrap_or(false) && !findings.is_empty() {
            Some(store.apply_drift(plan.id, &findings).map_err(to_err)?)
        } else {
            None
        };
        json(&serde_json::json!({
            "plan": plan.slug,
            "used_gh": use_gh,
            "in_sync": findings.is_empty(),
            "findings": findings.iter().map(|f| serde_json::json!({
                "slice": f.slice(),
                "problem": f.describe(),
                "remedy": f.remedy(),
            })).collect::<Vec<_>>(),
            "applied": applied.map(|r| r.applied),
        }))
    }

    #[tool(
        description = "Bring existing BUILD_PLAN.md / HANDOFF.md files into the database. Defaults to a dry run: read the report, then call again with dry_run false. Never deletes the source files."
    )]
    async fn import_markdown(
        &self,
        Parameters(args): Parameters<ImportArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut store = self.store()?;
        let dry_run = args.dry_run.unwrap_or(true);
        let mut report = Vec::new();

        for raw in &args.paths {
            let path = PathBuf::from(raw);
            let files = if path.is_dir() {
                collect(&path, 0)
            } else {
                vec![path.clone()]
            };
            for file in files {
                let md = match std::fs::read_to_string(&file) {
                    Ok(md) => md,
                    Err(e) => {
                        report.push(serde_json::json!({
                            "file": file.to_string_lossy(), "outcome": "unreadable",
                            "detail": e.to_string(),
                        }));
                        continue;
                    }
                };
                let dir = file.parent().unwrap_or(std::path::Path::new("."));
                let Ok(git) = GitContext::detect(dir) else {
                    report.push(serde_json::json!({
                        "file": file.to_string_lossy(), "outcome": "skipped",
                        "detail": "not inside a git repository",
                    }));
                    continue;
                };
                let repo = store.ensure_repo(&git).map_err(to_err)?;
                let outcome = store
                    .import_file(
                        repo.id,
                        &file,
                        &md,
                        ImportOptions {
                            replace: false,
                            dry_run,
                        },
                    )
                    .map_err(to_err)?;
                report.push(describe(&file, &outcome));
            }
        }
        json(&serde_json::json!({ "dry_run": dry_run, "files": report }))
    }
}

fn describe(path: &std::path::Path, outcome: &Outcome) -> serde_json::Value {
    let file = path.to_string_lossy().to_string();
    match outcome {
        Outcome::Created { plan, slices, .. } => serde_json::json!({
            "file": file, "outcome": "created", "plan": plan.slug, "slices": slices,
        }),
        Outcome::AlreadyImported { plan, first_seen } => serde_json::json!({
            "file": file, "outcome": "duplicate", "plan": plan.slug, "same_as": first_seen,
        }),
        Outcome::Conflict {
            plan,
            existing_sources,
        } => serde_json::json!({
            "file": file, "outcome": "conflict", "plan": plan.slug,
            "existing_sources": existing_sources,
            "detail": "two copies of this plan have drifted apart - compare them before replacing",
        }),
        Outcome::Replaced { plan } => serde_json::json!({
            "file": file, "outcome": "replaced", "plan": plan.slug,
        }),
        Outcome::HandoffAttached { plan, worktree } => serde_json::json!({
            "file": file, "outcome": "handoff", "plan": plan.slug, "worktree": worktree,
        }),
        Outcome::Skipped { reason } => serde_json::json!({
            "file": file, "outcome": "skipped", "detail": reason,
        }),
        Outcome::Planned {
            slug,
            title,
            slices,
            ..
        } => serde_json::json!({
            "file": file, "outcome": "would_create", "plan": slug, "title": title,
            "slices": slices,
        }),
    }
}

fn collect(dir: &std::path::Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if depth > 4 {
        return out;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "node_modules" | "vendor" | "target" | "dist" | "build" | "storage"
                )
            {
                continue;
            }
            out.extend(collect(&path, depth + 1));
            continue;
        }
        let upper = name.to_uppercase();
        if upper.ends_with(".MD")
            && (upper.contains("BUILD_PLAN")
                || upper.contains("BUILD-PLAN")
                || upper.starts_with("HANDOFF"))
        {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PlannerServer {
    fn get_info(&self) -> ServerInfo {
        let mut server_info = Implementation::from_build_env();
        server_info.name = "ai-planner".into();
        server_info.version = env!("CARGO_PKG_VERSION").into();

        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = server_info;
        info.instructions = Some(
            "Build plans for this machine live in one SQLite database, not in \
             BUILD_PLAN.md or HANDOFF.md files. Do not create or edit plan markdown - \
             use these tools.\n\n\
             Start with `locate` to find out which plan this worktree is on and what is \
             next; `get_resume` after a context clear. `get_plan` returns the whole plan \
             as markdown. Claim a slice with `claim_slice` before building it so a second \
             agent in another worktree does not duplicate the work, then `set_slice_status` \
             and `append_log` as you go. Record reasoning with `add_decision`, hard-won \
             facts with `add_gotcha`, and anything only a human can decide with \
             `open_question`. Before the context is cleared, call `write_handoff` with the \
             gates you actually ran."
                .into(),
        );
        info
    }
}

/// Serve over stdio until the client disconnects.
pub async fn run(store: Store, root: PathBuf) -> anyhow::Result<()> {
    let service = PlannerServer::new(store, root).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
