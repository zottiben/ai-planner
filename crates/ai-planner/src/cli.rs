use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "aip",
    version,
    about = "Build plans as rows, shared by every worktree and every agent",
    long_about = "aip keeps build plans in one SQLite database instead of markdown files \
                  copied between worktrees. Plans, slices, decisions and progress are \
                  addressable, so parallel agents can update one plan without clobbering \
                  each other.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Database file. Defaults to $AI_PLANNER_DB, else ~/.ai-planner/planner.db
    #[arg(long, global = true, env = "AI_PLANNER_DB")]
    pub db: Option<PathBuf>,

    /// Run as if started in this directory
    #[arg(short = 'C', long, global = true, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Which plan to act on: slug, ticket key, id, or part of the title
    #[arg(short = 'p', long, global = true, value_name = "PLAN")]
    pub plan: Option<String>,

    /// Machine-readable output
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create the database if needed and register this repo
    Init(InitArgs),

    /// Start a new plan in this repo
    New(NewArgs),

    /// Which plan does this worktree belong to?
    Current(CurrentArgs),

    /// Where you are and what to do next
    Status(StatusArgs),

    /// List plans
    Ls(LsArgs),

    /// Search every plan by what it says
    Find(FindArgs),

    /// Print a plan as markdown
    Show(ShowArgs),

    /// Set a plan's status
    Set(SetArgs),

    /// Change a plan's header fields
    Edit(EditArgs),

    /// Write a section of the plan document
    Section(SectionArgs),

    /// Record where a plan was grounded
    Source(SourceArgs),

    /// Work with the plan's slices (the vertical slices / PRs)
    #[command(subcommand)]
    Slice(SliceCmd),

    /// Append a progress note
    Log(LogArgs),

    /// Show the progress log
    Logs(LogsArgs),

    /// Record and revisit design decisions
    #[command(subcommand)]
    Decision(DecisionCmd),

    /// Track open questions
    #[command(subcommand)]
    Question(QuestionCmd),

    /// Record a gotcha worth carrying into the next session
    #[command(subcommand)]
    Gotcha(GotchaCmd),

    /// Bring existing BUILD_PLAN.md / HANDOFF.md files into the database
    Import(ImportArgs),

    /// Write a plan back out as markdown
    Export(ExportArgs),

    /// Checkpoint this worktree before clearing context
    #[command(subcommand)]
    Handoff(HandoffCmd),

    /// Print what a fresh session needs to pick this up
    Resume(ResumeArgs),

    /// One line of session-start context, as harness hook JSON
    Hook,

    /// Check the setup and point out anything that needs attention
    Doctor,

    /// Registered repos
    Repos,

    /// Database maintenance
    #[command(subcommand)]
    Db(DbCmd),
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Override the repo's display name
    #[arg(long)]
    pub name: Option<String>,
}

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Plan title, e.g. "ACME-1234 - Reusable Date Range Picker"
    pub title: String,

    /// Short handle. Defaults to the ticket key, else a slug of the title
    #[arg(long)]
    pub slug: Option<String>,

    /// Ticket key. Parsed out of the title when it is there
    #[arg(long)]
    pub ticket: Option<String>,

    #[arg(long)]
    pub ticket_url: Option<String>,

    /// Branch the slices stack onto
    #[arg(long)]
    pub base: Option<String>,

    #[arg(long)]
    pub owner: Option<String>,

    /// The blockquote under the title
    #[arg(long)]
    pub summary: Option<String>,

    #[arg(long, default_value = "draft")]
    pub status: String,
}

#[derive(Args, Debug)]
pub struct CurrentArgs {
    /// Say which rule resolved it
    #[arg(long)]
    pub why: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// One line, for prompts and scripts
    #[arg(long)]
    pub oneline: bool,
}

#[derive(Args, Debug)]
pub struct LsArgs {
    /// Every repo, not just this one
    #[arg(long)]
    pub all: bool,

    /// Filter by status; repeatable
    #[arg(long, value_name = "STATUS")]
    pub status: Vec<String>,

    /// Agreed but unfinished work: ready, active, in_review, blocked
    #[arg(long)]
    pub incomplete: bool,

    /// Substring of the title, slug or ticket
    #[arg(value_name = "QUERY")]
    pub query: Option<String>,
}

#[derive(Args, Debug)]
pub struct FindArgs {
    /// What to look for
    pub query: Vec<String>,

    /// Search every repo, not just this one
    #[arg(long)]
    pub all: bool,

    /// Filter by status; repeatable
    #[arg(long, value_name = "STATUS")]
    pub status: Vec<String>,

    /// Only unfinished plans
    #[arg(long)]
    pub incomplete: bool,

    #[arg(long, short = 'n', default_value = "12")]
    pub limit: usize,

    /// Rebuild the index first
    #[arg(long)]
    pub reindex: bool,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Which plan; defaults to the resolved one
    pub plan: Option<String>,

    /// Only this section
    #[arg(long)]
    pub section: Option<String>,

    /// The markdown this plan was imported from, byte for byte
    #[arg(long)]
    pub raw: bool,
}

#[derive(Args, Debug)]
pub struct SetArgs {
    /// draft | ready | active | in_review | blocked | done | deferred
    pub status: String,

    /// Which plan; defaults to the resolved one
    pub plan: Option<String>,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub summary: Option<String>,
    #[arg(long)]
    pub ticket: Option<String>,
    #[arg(long)]
    pub ticket_url: Option<String>,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long)]
    pub owner: Option<String>,
}

#[derive(Args, Debug)]
pub struct SectionArgs {
    /// Section handle, e.g. grounding
    pub key: String,

    /// Markdown body; omit to read stdin
    pub body: Option<String>,

    #[arg(long)]
    pub title: Option<String>,

    /// Read the body from a file
    #[arg(long, conflicts_with = "body")]
    pub file: Option<PathBuf>,

    /// body | sources | decisions | slices | questions | gotchas | log
    #[arg(long)]
    pub renders: Option<String>,

    /// Position in the document
    #[arg(long)]
    pub ord: Option<i64>,

    /// Refuse the write if the section changed since this revision
    #[arg(long)]
    pub expect_rev: Option<i64>,

    /// Add to the end of the section instead of replacing it
    #[arg(long)]
    pub append: bool,
}

#[derive(Args, Debug)]
pub struct SourceArgs {
    /// clickup | figma | code | design | repo | doc
    pub kind: String,
    /// The id, url or path
    pub reference: String,
    #[arg(long)]
    pub note: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum SliceCmd {
    /// Add a slice
    Add(SliceAddArgs),
    /// List the plan's slices
    Ls,
    /// Show one slice in full
    Show { key: String },
    /// Move a slice to a status
    Set(SliceSetArgs),
    /// Change a slice's fields
    Edit(SliceEditArgs),
    /// Take a slice for this worktree
    Claim { key: String },
    /// Give a claimed slice back
    Release { key: String },
    /// Slices claimed in worktrees that no longer exist
    Stale,
}

#[derive(Args, Debug)]
pub struct SliceAddArgs {
    /// Slice handle, e.g. PR1, S2, M4
    pub key: String,
    pub title: String,

    #[arg(long)]
    pub scope: Option<String>,
    #[arg(long, conflicts_with = "scope")]
    pub scope_file: Option<PathBuf>,
    /// How to prove it works
    #[arg(long)]
    pub demo: Option<String>,
    /// Rough file count
    #[arg(long)]
    pub files: Option<i64>,
    #[arg(long)]
    pub branch: Option<String>,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long, default_value = "ready")]
    pub status: String,
    #[arg(long)]
    pub ord: Option<i64>,
}

#[derive(Args, Debug)]
pub struct SliceSetArgs {
    pub key: String,
    /// draft | ready | active | in_review | blocked | done | deferred
    pub status: String,
    /// Why, when blocking
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct SliceEditArgs {
    pub key: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub scope: Option<String>,
    #[arg(long, conflicts_with = "scope")]
    pub scope_file: Option<PathBuf>,
    #[arg(long)]
    pub demo: Option<String>,
    #[arg(long)]
    pub files: Option<i64>,
    #[arg(long)]
    pub branch: Option<String>,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long)]
    pub pr: Option<String>,
}

#[derive(Args, Debug)]
pub struct LogArgs {
    /// What happened
    pub body: Vec<String>,
    /// Attach to a slice
    #[arg(long, value_name = "KEY")]
    pub slice: Option<String>,
    /// progress | status | decision | gotcha | verification | blocker | handoff
    #[arg(long, default_value = "progress")]
    pub kind: String,
}

#[derive(Args, Debug)]
pub struct LogsArgs {
    #[arg(long, short = 'n', default_value = "20")]
    pub limit: i64,
    /// Only entries attached to this slice
    #[arg(long, value_name = "KEY")]
    pub slice: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum DecisionCmd {
    /// Record a decision
    Add(DecisionAddArgs),
    /// List the plan's decisions
    Ls,
    /// Mark a decision superseded, keeping the original readable
    Supersede(DecisionSupersedeArgs),
}

#[derive(Args, Debug)]
pub struct DecisionAddArgs {
    pub title: String,
    /// The reasoning; omit to read stdin
    pub body: Option<String>,
    /// Fixed key such as D4 or AD-2; auto-numbered otherwise
    #[arg(long)]
    pub key: Option<String>,
    #[arg(long, conflicts_with = "body")]
    pub file: Option<PathBuf>,
    #[arg(long, default_value = "agreed")]
    pub status: String,
}

#[derive(Args, Debug)]
pub struct DecisionSupersedeArgs {
    pub key: String,
    /// The decision that replaces it
    #[arg(long)]
    pub by: String,
    #[arg(long)]
    pub note: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum QuestionCmd {
    /// Raise a question that needs an answer
    Add(QuestionAddArgs),
    /// List questions
    Ls {
        /// Answered ones too
        #[arg(long)]
        all: bool,
    },
    /// Answer a question
    Answer { id: i64, answer: String },
}

#[derive(Args, Debug)]
pub struct QuestionAddArgs {
    pub body: String,
    #[arg(long, value_name = "KEY")]
    pub slice: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum GotchaCmd {
    /// Record a gotcha
    Add(GotchaAddArgs),
    /// List gotchas
    Ls,
}

#[derive(Args, Debug)]
pub struct GotchaAddArgs {
    pub title: String,
    /// The detail; omit to read stdin
    pub body: Option<String>,
    #[arg(long, conflicts_with = "body")]
    pub file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum HandoffCmd {
    /// Record where this worktree got to
    Write(HandoffWriteArgs),
    /// The last handoff from this worktree
    Show,
    /// The latest handoff from every worktree on this plan
    Ls,
}

#[derive(Args, Debug)]
pub struct HandoffWriteArgs {
    /// A gate result: `typecheck=pass`, `test=pass:731 tests`, `lint=fail`. Repeatable
    #[arg(long, value_name = "NAME=RESULT")]
    pub gate: Vec<String>,

    /// The next concrete work item; repeatable. Defaults to the plan's next slice
    #[arg(long, value_name = "ITEM")]
    pub next: Vec<String>,

    /// Anything the next context needs that the plan does not already say
    #[arg(long)]
    pub notes: Option<String>,

    #[arg(long, conflicts_with = "notes")]
    pub notes_file: Option<PathBuf>,

    /// Record the handoff even with uncommitted changes
    #[arg(long)]
    pub allow_dirty: bool,
}

#[derive(Args, Debug)]
pub struct ResumeArgs {
    /// Also claim the next slice for this worktree
    #[arg(long)]
    pub claim: bool,
}

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// Files or directories to import
    pub paths: Vec<PathBuf>,

    /// Search a directory tree for BUILD_PLAN / HANDOFF markdown; repeatable
    #[arg(long, value_name = "DIR")]
    pub scan: Vec<PathBuf>,

    /// Overwrite a plan whose stored copy has drifted from this file
    #[arg(long)]
    pub replace: bool,

    /// Parse and report without writing
    #[arg(long)]
    pub dry_run: bool,

    /// Attach every imported handoff to this plan, for the ones nothing identifies
    #[arg(long = "as", value_name = "PLAN")]
    pub attach_to: Option<String>,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Which plan; defaults to the resolved one
    pub plan: Option<String>,

    /// Write to a file instead of stdout
    #[arg(long, short = 'o')]
    pub out: Option<PathBuf>,

    /// The markdown it was imported from, rather than the rendered plan
    #[arg(long)]
    pub raw: bool,

    /// Overwrite an existing file
    #[arg(long)]
    pub force: bool,
}

#[derive(Subcommand, Debug)]
pub enum DbCmd {
    /// Print the database path
    Path,
    /// Open the database in the default app (TablePlus on macOS)
    Open,
    /// Copy the database to a timestamped file
    Backup {
        #[arg(long, short = 'o')]
        out: Option<PathBuf>,
    },
    /// Schema version and row counts
    Status,
}
