// Mirrors crates/ai-planner-core/src/model.rs and store/board.rs.
//
// The Rust models serialise themselves, so there is no DTO layer between these and the
// database. That makes this file the one place the two languages have to agree: change
// a field in model.rs and change it here, or the compiler on this side will not notice.

export type Status =
  | "draft"
  | "ready"
  | "active"
  | "in_review"
  | "blocked"
  | "done"
  | "deferred";

export type LogKind =
  | "progress"
  | "status"
  | "decision"
  | "gotcha"
  | "verification"
  | "blocker"
  | "handoff";

export interface StatusMeta {
  value: Status;
  label: string;
  marker: string;
  terminal: boolean;
}

export interface Meta {
  database: string;
  actor: string;
  statuses: StatusMeta[];
  version: string;
}

export interface RepoSummary {
  id: number;
  key: string;
  name: string;
  remote_url: string | null;
  main_path: string | null;
  plans: number;
  unfinished_plans: number;
  open_questions: number;
  last_activity: string | null;
}

export interface Plan {
  id: number;
  repo_id: number;
  repo_name: string;
  slug: string;
  title: string;
  status: Status;
  summary: string | null;
  ticket_key: string | null;
  ticket_url: string | null;
  base_branch: string | null;
  owner: string | null;
  source_path: string | null;
  rev: number;
  created_at: string;
  updated_at: string;
}

/** A plan with its roll-ups flattened in, as `PlanSummary` serialises it. */
export interface PlanSummary extends Plan {
  slices: number;
  done: number;
  percent: number | null;
  open_questions: number;
  last_activity: string | null;
}

export interface Slice {
  id: number;
  plan_id: number;
  ord: number;
  key: string;
  title: string;
  status: Status;
  scope_md: string;
  demo_md: string | null;
  estimate_files: number | null;
  branch: string | null;
  base_branch: string | null;
  pr_url: string | null;
  worktree_path: string | null;
  claimed_by: string | null;
  claimed_at: string | null;
  blocked_reason: string | null;
  started_at: string | null;
  completed_at: string | null;
  rev: number;
  updated_at: string;
}

export interface BoardColumn {
  status: Status;
  slices: Slice[];
}

export interface Board {
  plan: Plan;
  columns: BoardColumn[];
}

export interface LogEntry {
  id: number;
  plan_id: number;
  slice_key: string | null;
  at: string;
  actor: string | null;
  kind: LogKind;
  branch: string | null;
  worktree_path: string | null;
  body: string;
}

export interface Section {
  id: number;
  plan_id: number;
  ord: number;
  key: string;
  title: string;
  body: string;
  renders: string;
  rev: number;
}

export interface Decision {
  id: number;
  plan_id: number;
  ord: number;
  key: string;
  title: string;
  body: string;
  status: "proposed" | "agreed" | "superseded" | "rejected";
  superseded_by: string | null;
  supersede_note: string | null;
  rev: number;
  decided_at: string;
}

export interface Question {
  id: number;
  plan_id: number;
  slice_key: string | null;
  body: string;
  status: string;
  answer: string | null;
  asked_at: string;
  answered_at: string | null;
}

export interface Gotcha {
  id: number;
  plan_id: number;
  title: string;
  body: string;
  created_at: string;
}

export interface PlanSource {
  id: number;
  kind: string;
  reference: string;
  note: string | null;
}

export interface PlanBundle {
  plan: Plan;
  sources: PlanSource[];
  sections: Section[];
  decisions: Decision[];
  slices: Slice[];
  questions: Question[];
  gotchas: Gotcha[];
  log: LogEntry[];
}

export interface SliceDetail {
  slice: Slice;
  plan_id: number;
  plan_slug: string;
  plan_title: string;
  repo: string;
  log: LogEntry[];
}
