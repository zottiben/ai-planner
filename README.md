# ai-planner

Build plans as rows in one SQLite database, not markdown files copied between
worktrees. `aip` gives every worktree, every repo and every agent harness the same
view of the same plan, and lets parallel agents update it without clobbering each
other.

A plan kept as `BUILD_PLAN.md` gets copied into each worktree so several PRs can be
built at once. Then the copies drift, two agents write progress into two versions of
one plan, and one of them wins. This removes the copies.

---

## Get started

### 1. Install

Needs a Rust toolchain ([rustup.rs](https://rustup.rs)).

```sh
git clone https://github.com/zottiben/ai-planner && cd ai-planner
./install/install.sh
```

That does three things:

| | |
| --- | --- |
| `cargo install` | the `aip` binary |
| `aip setup` | the skill (`~/.claude/skills`, `~/.agents/skills`), the always-on rules block in your global charter, and the three harness hooks merged into `~/.claude/settings.json` |
| `install-mcp.sh` | the MCP server, registered with Claude Code, Codex and Pi |

The skill and the hook script are compiled into the binary, so `aip setup` needs no
clone and no network - and they can never fall out of step with the version you are
running.

Add `--with-model` for semantic search (see [step 6](#6-optional-search-by-meaning)).
Each script runs standalone and takes `--project` to install into the current repo
instead of user-wide.

Behind a TLS-intercepting proxy, `export CARGO_NET_GIT_FETCH_WITH_CLI=true` first.

### 2. Register your repo

Once per repo, from any worktree - all of them share the one database.

```sh
cd ~/src/widget
aip init
```

### 3. Bring your existing plans in

```sh
aip import --scan ~/.awt/widget-a1b2c3 --scan ~/src/widget --dry-run
aip import --scan ~/.awt/widget-a1b2c3 --scan ~/src/widget
```

It finds every `*BUILD_PLAN*.md` and `HANDOFF*.md` and reads the dialects these are
actually written in - numbered sections, slices keyed `PR1` / `S1` / `M4` / `Phase 0` /
`Slice 0` at either heading level, decisions keyed `D1` / `AD-1`, status markers
(`✅ DONE`, `⛔ BLOCKED`, `✅ IN REVIEW`, `- DELIVERED 2026-07-29`), `**Demo:**` lines
and dated progress-log bullets.

- The same file in four worktrees imports **once**; every path it was found at is kept.
- Copies that have **drifted apart** are reported as a conflict, never merged silently.
  Compare them, then `--replace` to pick a winner.
- `HANDOFF*.md` attaches to its plan instead of becoming one, and its gotchas become rows.
- **Nothing is deleted, ever.** The original markdown is kept verbatim in the database
  too, so you can delete the files yourself whenever you are satisfied. `aip doctor`
  lists which are safe to remove.

### 4. Use it

```sh
aip status         # where you are: plan, slice, next item, open questions, recent notes
aip show           # the whole plan as markdown - the document the file used to be
aip ls             # plans in this repo        (--all for every repo)
aip find "herd symlink"

aip slice ls
aip slice claim PR2                  # take it for this worktree before you start
aip slice set PR2 in_review
aip log "PR2 gates green on abc1234." --slice PR2
aip sync                             # what git says that the plan does not (--fix applies)
aip decision add "One headless core, two shells" "The core carries all the logic."
aip gotcha add "The Herd symlink is shared" "Repoint it, then put it back."
```

### 5. Browse it

```sh
aip db open        # hands the file to TablePlus
```

Five views ship with the schema: `v_plans`, `v_slices`, `v_log`, `v_open_questions`,
`v_worktrees`. Open `v_plans` and you have a dashboard with no query written.

### 6. Optional: search by meaning

Off by default. Lexical search answers most questions, and this pulls in an ONNX
runtime plus a ~130 MB model.

```sh
./install/install.sh --with-model     # or add --features model-embeddings to cargo install
aip embed                             # downloads the model once, then indexes
```

`aip find` then fuses meaning with words, so a query need not share vocabulary with the
plan. Everything stays on the machine - no API keys, no inference calls. `aip embed
--clear` reverts to lexical; `--model-dir <dir>` loads a pre-downloaded model on an
offline machine.

### 7. Check it

```sh
aip doctor
```

Reports stale claims, blocked slices with no reason recorded, a missing rules block or
an out-of-date skill, and which markdown files are imported and safe to delete.

### 8. Keeping it current

```sh
aip update --check     # is there anything newer?
aip update             # rebuild, then refresh the skill, rules and hooks
```

`aip update` reads back **how** you installed it - the source and the feature list -
from cargo's own records, so a rebuild cannot silently drop `--features
model-embeddings` and leave semantic search broken with no error. It also backs the
database up first, since a newer binary may add migrations, and then re-runs `aip
setup` so the skill, the rules block and the hooks match the new binary. That second
half is the part that is easy to forget by hand and produces the strangest symptoms
when it is skipped.

Installed from a local clone? `git pull` there first - `aip update` rebuilds whatever
the clone currently contains, and will tell you so.

---

## Keeping the plan and the work in step

The failure mode this has to survive is an agent forgetting the plan exists between
tasks. Four mechanisms, because no single one is enough:

| | What it catches |
| --- | --- |
| `aip rules install` | The agent not knowing the tool exists. Appends a marked block to `~/.claude/CLAUDE.md` and `~/.agents/AGENTS.md`, so the rules are always in context rather than waiting to be discovered like a skill. Idempotent; `--force` refreshes it, `uninstall` removes it. |
| `UserPromptSubmit` hook | Forgetting *between tasks*. Injects one line - plan, slice, any drift - on every turn. A new task arrives as a new prompt, and `SessionStart` is long out of context by then. |
| `Stop` hook | A turn ending with the plan stale. Fires only when something is demonstrably wrong, and deduplicates per state so it cannot become noise. |
| `aip sync` | Everything the agent forgot anyway. Reconciles from git and `gh`: branches that have landed, PRs open or merged, claims on dead branches. `--fix` applies it. |

The last one is the important one: the mechanical facts are observable, so the database
reconciles itself rather than depending on anyone's memory. The hooks and the rules only
have to cover the judgement calls - progress notes, decisions, gotchas - which nothing
but the agent knows.

`PreCompact` and `SessionEnd` are deliberately unused: neither can inject context, so a
hook there could only block compaction, which is worse than saying nothing.

## Working with agents

- **Hooks** - `aip hook --event <session-start|user-prompt-submit|stop>` prints harness
  hook JSON. Silent when there is nothing to say, and it can never fail a session.
- **MCP server** - `aip serve` over stdio. Tools: `locate`, `get_plan`, `get_resume`,
  `search_plans`, `list_plans`, `list_slices`, `get_slice`, `claim_slice`,
  `release_slice`, `set_slice_status`, `update_slice`, `add_slice`, `append_log`,
  `add_decision`, `supersede_decision`, `add_gotcha`, `open_question`,
  `list_questions`, `answer_question`, `update_section`, `create_plan`,
  `write_handoff`, `import_markdown`, `sync_plan`.
- **Skill** - tells the agent which tool to reach for, and not to write plan markdown.

Codex and Pi: call `aip hook --event …` from your own hooks; it prints the same JSON.

## Handoffs

```sh
aip handoff write --gate typecheck=pass --gate "test=pass:731 tests"
aip resume         # in the next session
```

This replaces step 3 of the `toolbox-handoff` skill - the same content, scoped to
(plan, worktree), so it is not a file and cannot be copied. Gates keep their real
results: a failure is reported as red, never folded into a green checkpoint.

## How it finds your plan

In this order, stopping at the first hit:

1. `--plan`, or `$AI_PLANNER_PLAN`
2. a slice recording the current branch
3. a slice claimed in this worktree
4. the last handoff written from this worktree
5. a learned association from a previous resolution here
6. a ticket key in the branch name (`feature/acme-1234-csv-export` -> `ACME-1234`)
7. the repo's only unfinished plan

`aip current --why` says which rule fired. When none does it lists the candidates
rather than guessing; naming one teaches the association for next time.

## Parallel agents

- WAL mode; every write is an `IMMEDIATE` transaction.
- The progress log is **append-only**, enforced by a trigger - concurrent notes cannot
  conflict.
- Sections and slices carry a `rev`; `--expect-rev` refuses a stale write instead of
  overwriting it.
- `aip slice claim` is guarded in the `UPDATE`'s `WHERE`, scoped to (actor, worktree),
  so two agents racing produce exactly one winner.

## Statuses

`draft` · `ready` · `active` · `in_review` · `blocked` · `done` · `deferred`

`--incomplete` filters `ready|active|in_review|blocked`: an agent needs to know whether
to resume something or to begin it. Every status change writes a log row.

## Where things live

| | |
| --- | --- |
| Database | `$AI_PLANNER_DB`, else `$XDG_DATA_HOME/ai-planner/planner.db`, else `~/.ai-planner/planner.db` |
| Model cache | `$AI_PLANNER_MODEL_CACHE`, else `~/.cache/ai-planner/fastembed` |
| Actor in the log | `$AI_PLANNER_ACTOR`, else `$USER` |

One database for every repo - that is what lets four worktrees share a plan with no
setup. `aip db backup` takes a consistent copy while agents are writing.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --features model-embeddings
```
