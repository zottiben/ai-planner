# ai-planner - Build Plan

> Owner: Ben Zotti. Created 2026-09-01.
> Grounded on the real build plans in `~/.awt/widget-a1b2c3/{1,2,3,4}/widget`
> and `~/src/widget`, on `~/src/file-sql` (Rust + SQLite + MCP + optional local
> model), `~/src/ai-toolbox` (the `handoff` skill and the installer), and
> `~/src/ai-worktree` (how worktrees are laid out and leased).
>
> This file is the bootstrap. Once PR3 lands it is imported into the database and
> becomes plan #1 - after that, `aip show ai-planner` is the source of truth and this
> file is deleted. Dogfood or it does not work.

---

## 1. Outcome

Build plans stop being markdown files that get copied between worktrees and start
being rows in one SQLite database that every worktree, every repo, and every agent
harness reads from and writes to.

Concretely, after this project:

- No `*_BUILD_PLAN.md` or `HANDOFF*.md` in any repo root.
- A plan is written once and is visible from all 4 worktrees instantly.
- An agent starting in worktree 3 on `feat/date-range-picker` is told, before it is
  asked anything, which plan it is on, which slice is next, and what the last session
  learned.
- Two agents building PR2 and PR3 of the same plan in parallel cannot clobber each
  other's progress.
- The database opens in TablePlus and reads like a dashboard.

---

## 2. Grounding - the evidence this is built on

### 2a. The problem, measured

| File | wt1 | wt2 | wt3 | wt4 | master |
| --- | --- | --- | --- | --- | --- |
| `CANVAS_EDITOR_BUILD_PLAN.md` | 923 | 923 | 923 | - | - |
| `ACME-1234_BUILD_PLAN.md` | - | - | 483 | 476 | - |
| `ACCOUNTS_V2_BUILD_PLAN.md` | - | 102 | - | - | - |
| `ACME-1201_BUILD_PLAN.md` | - | - | 382 | - | - |
| `ACME-1202_BUILD_PLAN.md` | - | - | - | 317 | - |
| `PLAYWRIGHT_PRODUCTION_BUILD_PLAN.md` | 625 | - | - | - | - |
| `HANDOFF*.md` | 3 files | 1 | 1 | 1 | - |
| review/findings `*.md` | 4 | 6 | 2 | 3 | 7 |

Three copies of Canvas Editor are byte-identical (identical shas), so today's
copying is pure duplication with no benefit. ACME-1234 has already diverged between
worktrees 3 and 4 - two agents wrote progress into two different copies of the same
plan. That is the failure this project exists to remove.

### 2b. The shape of a plan, as actually written

Every plan sampled follows the same spine, with only the vocabulary changing:

1. **Header** - title, ticket ref + URL, owner, created date, base branch, and the
   sources it was grounded on (ClickUp list id, Figma file/node ids, a POC repo).
2. **Scope / Outcome** - prose, plus a ticket table.
3. **Grounding** - "What already exists (verified in code, 2026-07-28)". Long,
   high-value, and the part most expensive to regenerate.
4. **Decisions** - stable numbered items: `D1..D11` (ACME-1234), `AD-1..AD-7`
   (Canvas Editor). Referenced later by number: *"Do not re-litigate D1-D11 - they are
   agreed with Ben."* One is marked superseded in place (`D4`'s mechanism changed).
5. **Delivery slices** - `PR1..PR8`, `S1..S9`, `M0..M7`, `Phase 0..8`. Each carries a
   title, a file-count estimate, scope, a **Demo** line, and often a status marker
   (`✅ DONE`, `✅ IN REVIEW`, `⛔ BLOCKED`, `- DELIVERED 2026-07-29`).
6. **Open questions / Risks** - things needing a decision from Ben.
7. **Progress log** - append-only, newest first, dated, one paragraph per session.

The handoffs add: a **RESUME HERE** block (worktree, branch, HEAD sha, PR number),
a **gates table** (`bun run typecheck` 7/7, `lint` 15/15, `test:vitest` 731 tests),
**decisions taken this session**, **gotchas**, and **how to resume**.

This is the schema. It is derived, not invented.

### 2c. What identity is already free

```
$ cd ~/.awt/widget-a1b2c3/3/widget
branch:      feat/date-range-picker
common-dir:  /Users/benzotti/src/widget/.git      <- same from every worktree
toplevel:    /Users/benzotti/.awt/widget-a1b2c3/3/widget
remote:      git@github.com:acme/widget.git
```

`git worktree list` from master enumerates all 5 checkouts. So repo identity, worktree
identity and branch are all obtainable in one `git` call with no configuration and no
`awt` dependency.

Branch names already carry plan identity: `feature/acme-1234-csv-export` contains the
ticket key; `feat/date-range-picker` is named as PR1's branch inside the ACME-1234
plan. **Resolution is a lookup, not an inference, in the common case.**

### 2d. What file-sql already solved

`file-sql` is the reference implementation for everything hard here: Rust workspace,
`rusqlite` with bundled SQLite, FTS5 + `sqlite-vec`, a `Storage` trait, an `rmcp` stdio
MCP server, a bundled skill, and - the part that matters most - **a local embedding
model that is opt-in behind a cargo feature** (`model-embeddings`, `fastembed`, BGE),
with a deterministic AI-free lexical mode as the default. We copy that posture exactly.

### 2e. What the handoff skill needs

`toolbox-handoff` step 3 writes a committed `HANDOFF.md` with RESUME HERE, gotchas, and
how-to-resume. Steps 1, 2 and 4 (commit + push, certify gates, tell the user) are
unaffected by this project. **Only step 3 changes**: the same content, written to the
database and scoped to (plan, worktree), so it is not a file and cannot be copied.

---

## 3. Decisions

### D1 - One global database, not one per repo

`$XDG_DATA_HOME/ai-planner/planner.db`, defaulting to `~/.ai-planner/planner.db`.

A per-repo database would live inside a worktree and be back to square one - either
copied per worktree, or in the main checkout where the worktrees cannot agree on who
owns it. Global means four worktrees share it with zero setup, cross-repo search works,
and TablePlus needs exactly one connection.

The cost is a single file holding everything. Mitigated by D5 (raw markdown is never
discarded), `aip export` (any plan back to markdown at any time), and `aip db backup`.

Override with `AI_PLANNER_DB` for tests and for anyone who wants a project-local file.

### D2 - Repo identity is the normalised remote, with the main checkout as fallback

`git@github.com:acme/widget.git` and
`https://github.com/acme/widget` both normalise to
`github.com/acme/widget`. Every worktree of that repo resolves to the
same key without a config file.

No remote (a local-only repo) falls back to the absolute path of the directory
containing `git rev-parse --git-common-dir`, which is the main checkout and is likewise
identical from every worktree.

### D3 - A plan is structured rows, and rendering back to the plan's own markdown is a
### hard requirement

The tables are `plan`, `plan_section`, `decision`, `slice`, `question`, `gotcha`, `log`,
`handoff` - the spine from section 2b.

`aip show <plan>` must emit markdown that reads like the file it replaced. This is the
single most important property for agent adoption: an agent that wants "the plan" gets
the same document it used to `cat`, so nothing about how it reads a plan has to change.
Only writing changes, and writing becomes targeted (`aip slice done PR2`) instead of
rewriting an 923-line file.

Enforced by a round-trip test: import each of the 9 real plans, render, and assert the
rendered document contains every heading, decision key, slice key and log line of the
original.

### D4 - Append-only log, optimistic concurrency on everything else

Parallel agents are the whole point, so the concurrency model is explicit:

- SQLite in **WAL** mode, `busy_timeout = 5000`, every write in an `IMMEDIATE`
  transaction. That makes concurrent writers safe at the storage layer.
- The **`log` table is insert-only**. Progress notes are the highest-frequency write and
  the thing that got clobbered in the ACME-1234 divergence. Inserts cannot conflict.
- Mutable text (`plan_section.body`, `slice.scope_md`, `decision.body`) carries a
  `rev` integer. Updates are `WHERE id = ? AND rev = ?`; zero rows affected is a
  conflict, and the CLI/MCP returns the current value so the caller can merge rather
  than overwrite.
- Slice status transitions are single-column updates, so two agents finishing two
  different slices never touch the same row.

Contention is low by construction because **one slice is one PR is one worktree**.

### D5 - Nothing imported is ever discarded

`plan.raw_md` keeps the verbatim source of an imported file forever, and
`plan_import` records the source path, sha256 and import time. If the parser
mis-splits a section, the original is one query away. This is what makes it safe to
delete the markdown files afterwards.

### D6 - Claims, not locks

`aip slice claim PR2` atomically sets `claimed_by` + `worktree_path` +
`claimed_at` only if the slice is unclaimed. It does not prevent writes - it prevents
two agents *starting the same slice*, which is the actual waste. A stale claim (its
worktree gone, or older than the configured horizon) is reported by `aip doctor` and
released with `aip slice release PR2`.

This mirrors `awt`'s lease model deliberately: same mental model, and a claim records
the worktree so `awt status` and `aip status` tell a consistent story.

### D7 - Resolution is a deterministic cascade; the model is the last resort

`aip current` answers "which plan am I on?" in this order, stopping at the first hit:

1. `--plan <slug>` or `$AI_PLANNER_PLAN`.
2. A slice whose `branch` equals the current branch. (Exact hit today for
   `feat/date-range-picker`.)
3. The newest claim or handoff for this worktree path.
4. `plan_affinity` - the learned (repo, branch, worktree) -> plan association.
5. A ticket key parsed from the branch matching a plan slug or ticket ref.
   (`feature/acme-1234-csv-export` -> `acme-1234`.)
6. Exactly one active plan in this repo -> that one.
7. Otherwise: ranked search, return candidates, do not guess.

Steps 2-6 cover every plan in the sample without any AI. The point of the cascade is
that the answer is *correct and explainable*, and `aip current --why` prints which rule
fired.

### D8 - The model is opt-in, exactly as in file-sql

Default search is **lexical**: FTS5 over titles, sections, slices and logs, ranked with
BM25 plus recency and repo/branch affinity. No model download, no network, deterministic.

`--features model-embeddings` plus `search.mode = "model"` enables local BGE embeddings
via `fastembed` and `sqlite-vec` for "find the plan about the thing with the two-pane
date panel". Same crate, same flags, same defaults as file-sql, so there is one thing
to understand rather than two.

### D9 - `plan_affinity` is the learning, and it is a counter, not a model

Every confirmed resolution increments a row keyed by (repo, branch, worktree, plan).
It is exact, instant, inspectable in TablePlus, and it gets better with use. A model
cannot beat a lookup table for this question, and a wrong answer here is expensive
because the agent writes progress into the wrong plan.

### D10 - Seamlessness is three things, and all three ship

An agent must not have to be told about this package each session:

1. **A session-start hook** runs `aip status --hook` and injects "you are on plan
   ACME-1234, slice PR2 (claimed here), 3 of 8 done, next: ..." into the session
   context. This is the load-bearing one - the pointer arrives unprompted, which is
   what `HANDOFF.md`-in-the-repo-root was really buying.
2. **An MCP server** (`aip serve`, `rmcp`, stdio) so the agent can act, not just read.
3. **A user-level skill** telling the agent which tool to reach for and when.

Installed once at user scope, so every repo and every worktree inherits them. The hook
is what makes it seamless; the skill alone is not enough, because a skill only fires
when the agent already suspects it needs one.

### D11 - The handoff lives here, and `toolbox-handoff` gets a one-step edit

Handoff state is `(plan, worktree_path)` scoped: branch, HEAD sha, gates JSON, resume
markdown, next items. `aip handoff write` records it; `aip resume` prints it.

`toolbox-handoff` steps 1, 2 and 4 are unchanged. Step 3 changes from "write
`HANDOFF.md`" to "`aip handoff write`", and the resume line changes from
*"read HANDOFF.md and continue"* to *"`aip resume` and continue"* - which the
session-start hook then makes automatic. The edit to `~/src/ai-toolbox/skills/handoff/SKILL.md`
ships in PR6 of this plan, gated on the CLI being installed (the skill falls back to
`HANDOFF.md` if `aip` is absent, so ai-toolbox stays standalone).

### D12 - Rust, matching file-sql

Same toolchain, same crates (`rusqlite` bundled, `sqlite-vec`, FTS5, `rmcp`,
`fastembed`, `clap`), same install story (`cargo install`, one static binary, no
runtime). The optional-local-model requirement is already solved there and is copied
rather than re-derived. Go was the alternative (matching `awt`) but would mean
re-solving embeddings and MCP from scratch.

Binary name: **`aip`**. Three letters, consistent with `awt`.

### D13 - The database is a first-class UI

Because TablePlus is a stated requirement, the schema is designed to be *browsed*:

- Timestamps are ISO-8601 `TEXT`, not epoch integers, so rows are readable.
- Status columns are lowercase strings with `CHECK` constraints, not enum ints.
- Read-only views ship with the schema: `v_plans` (repo, slug, title, status, slices
  done/total, % complete, last activity), `v_slices`, `v_log`, `v_open_questions`,
  `v_worktrees`. Opening the file and looking at `v_plans` should answer "what is going
  on" with no query written.

---

## 4. Statuses

One vocabulary, used at both plan and slice level, covering the four Ben asked for plus
the states the real plans already mark up.

| Status | Meaning | Seen in the wild as |
| --- | --- | --- |
| `draft` | being written, not ready to build | (plans mid-authoring) |
| `ready` | agreed and buildable, unclaimed | plain slice entries |
| `active` | in progress | "IN PROGRESS", "current work" |
| `in_review` | PR open, awaiting review | "✅ IN REVIEW", "PR #412 open" |
| `blocked` | cannot proceed, reason recorded | "⛔ BLOCKED" |
| `done` | complete | "✅ DONE", "DELIVERED 2026-07-29" |
| `deferred` | consciously not doing now | "PR8 (optional)", "deferred, not built" |

`incomplete` from the ask is expressed as `active` (started) or `ready` (not started) -
kept distinct because an agent needs to know whether to resume or to begin. `aip ls
--incomplete` is a filter over `ready|active|in_review|blocked` so the word still works
on the CLI.

Every status change writes a `log` row, so status history is free.

---

## 5. Delivery slices

Seven slices. Each is independently useful and independently shippable.

### PR1 - Core: schema, storage, plan/slice CRUD, TablePlus views

`crates/ai-planner-core` + `crates/ai-planner` (bin `aip`).

- Schema + migrations (D3, D13), WAL/busy_timeout/IMMEDIATE (D4), `rev` optimistic
  concurrency, the `v_*` views.
- Git context detection: repo key (D2), worktree, branch.
- `aip init | new | ls | show | set | slice add|ls|set | log | decision add | question
  add | gotcha add | db path|open`.
- `aip show` renders the plan's markdown (D3).
- Tests: migrations, concurrent writers, rev conflicts, repo-key normalisation.

**Demo:** `aip new "Date Range Picker" --ticket ACME-1234`, add three slices, mark one
done, `aip show` prints a plan that looks like the file, `aip db open` shows it in
TablePlus.

### PR2 - Context: resolution, claims, status, affinity

- The D7 cascade with `--why`, `plan_affinity` (D9), slice claims (D6).
- `aip current | status | resume`, `aip slice claim|release`.
- Worktree awareness: `aip status` in worktree 3 reports worktree 3.
- Tests: each cascade rule in isolation, claim races, stale-claim detection.

**Demo:** from `~/.awt/.../3/widget` on `feat/date-range-picker`, `aip current`
names the plan and `--why` says which rule fired; two shells racing `slice claim PR2`
produce exactly one winner.

### PR3 - Import and export: the real files, losslessly

- Markdown importer for the observed dialects (`## N.` sections, `### PR1|S1|M4|Phase 0`
  slices, `### D1|AD-1` decisions, `## Progress log` bullets, `✅|⛔` status markers).
- Handoff importer (RESUME HERE, gates table, gotchas).
- Dedupe by sha256; divergent copies of one plan (ACME-1234) are reported as a conflict
  with a diff rather than silently merged.
- `aip export` back to markdown; `plan.raw_md` retained (D5).
- Round-trip test over all 9 real plans (D3).

**Demo:** `aip import --scan ~/.awt/widget-a1b2c3 ~/src/widget` ingests every
plan and handoff, reports the 3 identical Canvas Editor copies as one plan and the
ACME-1234 pair as a conflict; `aip ls` lists 6 distinct plans.

### PR4 - Search

- FTS5 over plan/section/slice/log with BM25 + recency + affinity ranking (D8).
- `aip find "<query>"`, `--repo`/`--all`, `--status`, JSON output.
- Optional `model-embeddings` feature: `fastembed` BGE + `sqlite-vec` hybrid rank.

**Demo:** `aip find "two-pane date panel"` returns ACME-1234 first with the matching
line; `aip find "canvas editor pdf"` returns the Canvas Editor plan's S5.

### PR5 - MCP server and skill

- `aip serve` (`rmcp`, stdio): `resolve_plan`, `search_plans`, `get_plan`, `get_slice`,
  `list_slices`, `claim_slice`, `set_slice_status`, `append_log`, `add_decision`,
  `add_gotcha`, `open_question`, `answer_question`, `update_section`, `write_handoff`,
  `get_handoff`, `create_plan`.
- Skill `ai-planner` into `.claude/skills`, `.agents/skills`, and user scope.
- Registration for Claude Code, Codex (`~/.codex/config.toml`), Pi (`.pi/mcp.json`).

**Demo:** in Claude Code, "what am I building?" answers from the DB with no file read;
marking a slice done from the agent is visible in TablePlus immediately.

### PR6 - Handoff and the session-start hook

- `aip handoff write|show`, `aip resume`, `aip status --hook` emitting harness context
  JSON (D10).
- Hook installers for Claude Code / Codex / Pi, user scope.
- The one-step edit to `~/src/ai-toolbox/skills/handoff/SKILL.md` (D11), with an
  `aip`-absent fallback so ai-toolbox stays standalone.

**Demo:** `/handoff` in worktree 3 records to the DB; a fresh session in that worktree
opens already knowing the plan, the slice and the next item, with no prompt.

### PR7 - Install, docs, dogfood

- `install.sh` (+ `install-skill.sh`, `install-mcp.sh`) in the file-sql shape, GH Pages.
- `aip doctor` (DB reachable, migrations current, stale claims, orphaned worktrees,
  un-imported `*_BUILD_PLAN.md` still on disk).
- `aip db backup`, README, `AGENTS.md`.
- Import this file as plan #1 and delete it.

**Demo:** `curl ... | sh` on a clean machine, then `aip doctor` green; `aip show
ai-planner` renders this document from the database.

---

## 6. Open questions

- **Deleting the source files.** PR3 imports; it does not delete. Recommendation: keep
  the markdown until PR7's `doctor` confirms every file is imported and rendered, then
  remove in one commit. Confirm before anything is deleted.
- **The other `*.md` in those roots** (`PR-201-code-review.md`,
  `ACME-980-FINDINGS.md`, `PR-208_REVIEW_RESPONSES.md` - 22 files) are the same
  pollution but a different document type. Not in scope; worth a `kind` column on
  `plan` so they can be folded in later without a migration.
- **Sync across machines.** Out of scope. The single file makes `git`-in-a-private-repo
  or Syncthing trivial later; nothing here blocks it.
- **`awt` integration.** `aip` reads git directly and never shells out to `awt`, so it
  works in plain worktrees too. A future `awt get` hook could pre-claim a slice, but
  coupling the two tools is not needed for any requirement here.
