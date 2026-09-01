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

That does four things:

| | |
| --- | --- |
| `cargo install` | the `aip` binary |
| `install-skill.sh` | the agent skill into `~/.claude/skills` and `~/.agents/skills` |
| `install-hook.sh` | a session-start hook, merged into `~/.claude/settings.json` |
| `install-mcp.sh` | the MCP server, registered with Claude Code, Codex and Pi |

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

Reports stale claims, blocked slices with no reason recorded, and which markdown files
are imported and safe to delete.

---

## Working with agents

Three pieces, so an agent never has to be told about this:

- **Session hook** - `aip hook` prints one line of context (plan, slice, next item,
  whether a handoff is waiting) as harness hook JSON. Silent when there is nothing to
  say, and it can never fail a session.
- **MCP server** - `aip serve` over stdio. Tools: `locate`, `get_plan`, `get_resume`,
  `search_plans`, `list_plans`, `list_slices`, `get_slice`, `claim_slice`,
  `release_slice`, `set_slice_status`, `update_slice`, `add_slice`, `append_log`,
  `add_decision`, `supersede_decision`, `add_gotcha`, `open_question`,
  `list_questions`, `answer_question`, `update_section`, `create_plan`,
  `write_handoff`, `import_markdown`.
- **Skill** - tells the agent which tool to reach for, and not to write plan markdown.

Codex and Pi: call `aip hook` from your own session-start hook; it prints the same JSON.

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
