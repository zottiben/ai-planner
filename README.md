# ai-planner

Build plans as rows in one SQLite database, not markdown files copied between
worktrees. `aip` gives every worktree, every repo and every agent harness the same
view of the same plan, and lets parallel agents update it without clobbering each
other.

It exists because a plan kept as `BUILD_PLAN.md` gets copied into each worktree so
several PRs can be built at once, and then the copies drift: two agents write progress
into two versions of one plan and one of them wins.

## Install

```sh
curl -fsSL https://zottiben.github.io/ai-planner/install.sh | sh
```

That installs the `aip` binary with `cargo install` (needs a Rust toolchain -
https://rustup.rs), installs the agent skill at user scope, and wires the session-start
hook so every new agent session is told which plan its worktree is on.

Then, in each repo:

```sh
aip init          # register the repo (run it once, from any worktree)
```

## Use it

```sh
aip status                       # where you are: plan, slice, next item, questions
aip show                         # the whole plan as markdown - the file it replaced
aip ls                           # plans in this repo   (--all for every repo)
aip find "two-pane date panel"   # search everything every plan says

aip slice ls
aip slice claim PR2              # take it for this worktree before you start
aip slice set PR2 done
aip log "PR2 gates green on abc1234." --slice PR2

aip handoff write --gate typecheck=pass --gate "test=pass:731 tests"
aip resume                       # what a fresh session needs to pick this up

aip db open                      # browse it in TablePlus
aip doctor                       # check the setup
```

Every command takes `--json`.

## Bringing existing plans in

```sh
aip import --scan ~/.awt/my-repo-hash --scan ~/src/my-repo
```

The importer reads the dialects these documents are actually written in: numbered
sections, slices keyed `PR1` / `S1` / `M4` / `Phase 0` / `Slice 0` at either heading
level, decisions keyed `D1` / `AD-1`, status markers (`✅ DONE`, `⛔ BLOCKED`,
`✅ IN REVIEW`, `- DELIVERED 2026-07-29`), `**Demo:**` lines and dated progress-log
bullets. It keeps the original file verbatim, so nothing is lost and the markdown can
be deleted afterwards.

- The same file copied into four worktrees imports **once**, and every path it was
  found at is recorded.
- Copies that have **drifted apart** are reported as a conflict, never merged silently.
  `--replace` picks a winner.
- `HANDOFF*.md` files attach to their plan rather than becoming plans, and their
  gotchas become rows.

`aip doctor` then lists which files are safely importable and which are already in and
can be deleted.

## How it finds your plan

`aip` works out which plan a worktree is on, in this order, stopping at the first hit:

1. `--plan`, or `$AI_PLANNER_PLAN`.
2. A slice that records the current branch.
3. A slice claimed in this worktree.
4. The last handoff written from this worktree.
5. A learned association from a previous resolution here.
6. A ticket key in the branch name (`feature/acme-1234-csv-export` -> `ACME-1234`).
7. The repo's only unfinished plan.

`aip current --why` says which rule fired. When none does, it lists the candidates
rather than guessing - naming one teaches the association for next time.

## Parallel agents

- SQLite in WAL mode; every write is an `IMMEDIATE` transaction.
- The progress log is **append-only**, enforced by a trigger. Concurrent notes cannot
  conflict, which is the failure this project exists to remove.
- Section and slice bodies carry a `rev`. `--expect-rev` refuses a write whose base has
  moved instead of overwriting it.
- `aip slice claim` is guarded in the `UPDATE`'s `WHERE`, scoped to (actor, worktree),
  so two agents racing produce exactly one winner.

## Agent harnesses

Three things ship so an agent does not have to be told about this each session:

| Piece | What it does |
| --- | --- |
| **Session hook** | `aip hook` prints one line of context - plan, slice, next item, whether a handoff is waiting - as harness hook JSON. Silent when there is nothing to say. |
| **Skill** | `skill/SKILL.md`, installed to `~/.claude/skills` and `~/.agents/skills`, tells the agent which command to reach for. |
| **`--json`** | Every command, for when the agent needs to branch on a result. |

Codex and Pi: call `aip hook` from your own session-start hook; it prints the same JSON.

## Handoffs

`aip handoff write` replaces step 3 of the `toolbox-handoff` skill - the same content,
scoped to (plan, worktree), so it is not a file and cannot be copied. Gates are
recorded with their real results; a failed gate is reported as failed rather than
rolled into a green checkpoint. A fresh session runs `aip resume`.

## Where the database lives

`$AI_PLANNER_DB`, else `$XDG_DATA_HOME/ai-planner/planner.db`, else
`~/.ai-planner/planner.db`. One file for every repo, which is what lets four worktrees
share a plan with no setup.

Timestamps are ISO-8601 text and statuses are words, so the file reads properly in
TablePlus. Five views ship with the schema - `v_plans`, `v_slices`, `v_log`,
`v_open_questions`, `v_worktrees` - so opening it answers "what is going on" with no
query written.

```sh
aip db open      # hand the file to TablePlus
aip db backup    # VACUUM INTO a timestamped copy, safe while agents are writing
aip db status    # schema version and row counts
```

## Statuses

`draft` · `ready` · `active` · `in_review` · `blocked` · `done` · `deferred`

"Incomplete" is the filter `--incomplete` over `ready|active|in_review|blocked`: an
agent needs to know whether to resume something or to begin it. Every status change
writes a log row, so history is free.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -p ai-planner -- status
```
