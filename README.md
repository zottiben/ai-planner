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

```sh
curl -fsSL https://zottiben.github.io/ai-planner/install.sh | sh
```

No Rust toolchain needed: that downloads the latest release for your platform. macOS
and Linux, on x86_64 and arm64.

Or from a clone, which builds it:

```sh
git clone https://github.com/zottiben/ai-planner && cd ai-planner
./install/install.sh --from-source
```

Either way it does three things:

| | |
| --- | --- |
| installs | the `aip` binary - and `ai-planner.app` into `/Applications` on macOS |
| `aip setup` | the skill (`~/.claude/skills`, `~/.agents/skills`), the always-on rules block in your global charter, and the three harness hooks merged into `~/.claude/settings.json` |
| `install-mcp.sh` | the MCP server, registered with Claude Code, Codex and Pi |

The skill, the hook script and the board's frontend are all compiled into the binary,
so `aip setup` needs no clone and no network, `aip ui` needs no dev server, and none of
them can fall out of step with the version you are running.

Add `--with-model` for semantic search (see [step 6](#6-optional-search-by-meaning)).
That one implies `--from-source`, because the feature is compiled in. Each script runs
standalone and takes `--project` to install into the current repo instead of user-wide.

`brew install gum` is optional and makes `aip show` render the plan instead of printing
raw markdown at you.

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

On a terminal, `aip show` hands the document to
[gum](https://github.com/charmbracelet/gum) - headings, tables and code spans styled -
and `aip show -P` scrolls it in a pager with `/` to search. Behind a pipe, a redirect,
`--json` or `NO_COLOR` it is the plain markdown again, byte for byte, because that is
what an agent reads. `--plain` forces that anywhere, `AI_PLANNER_MARKDOWN=always`
forces the other way, and no gum installed simply means plain markdown.

### 5. See it as a board

```sh
aip ui             # opens a browser at a local board
```

A sidebar of every repo in the database and the plans under it; a column per status
with a card per slice; a ticket behind each card with its scope, demo, branch, PR, who
holds the claim and its own progress log; and a Plan tab with the decisions, gotchas
and open questions laid out to read.

It is a write surface, not a report. Dragging a card is `aip slice set`, and the board
claims, releases, links a PR and writes progress notes through the same `Store` the
CLI uses - so the claim guard and the append-only log apply exactly as they do in a
terminal.

The frontend is compiled into the binary, so there is no dev server and no `npm
install`. It binds `127.0.0.1` only, on a port the OS picks, with a token minted per
run - loopback is not an origin boundary, and this API can move any slice in any repo
on the machine.

It follows the database while you watch: an agent writing from another worktree moves
the card without a refresh.

```sh
aip ui --port 7777 --no-open    # a stable URL, and leave the browser alone
```

### 5b. Or as a desktop app

```sh
cd crates/ai-planner-desktop && npx @tauri-apps/cli@2 build
```

Produces a `.dmg` on macOS, `.msi`/`.exe` on Windows and `.AppImage`/`.deb` on Linux -
about 13 MB installed, because it uses the platform's webview rather than shipping a
copy of Chromium. It is the same app `aip ui` serves, in a window with an icon.

`.github/workflows/release.yml` builds all three on a version tag, alongside standalone
CLI archives and `checksums.txt`. macOS is a universal binary; Linux also ships native
x86_64 and arm64 builds.

> The macOS app is **ad-hoc signed** by default. The curl installer does not set the
> quarantine attribute, so the app it puts in `/Applications` launches normally. A
> `.dmg` downloaded in a browser is quarantined: right-click the app and choose Open,
> or run `xattr -dr com.apple.quarantine /Applications/ai-planner.app`. The workflow
> automatically upgrades to Developer ID signing and notarisation if the same secrets
> used by Skelly are configured. Windows remains unsigned and can show SmartScreen.

### 5c. Or as rows

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
  `write_handoff`, `import_markdown`, `sync_plan`, `delete_plan`.
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

## Deleting a plan

```sh
aip delete acme-1234 --dry-run   # what would go
aip delete acme-1234             # type the slug to confirm
aip delete acme-1234 --yes       # no prompt, for scripts and agents
```

The one write that cannot be taken back: the slices, decisions, notes, gotchas,
questions and handoffs go with the plan, and it leaves the search index with them. It
is for a plan raised by mistake, or for clearing the ground so a task can be run again
from nothing - a finished plan is `aip set done`, not a deleted one.

Four things stand in the way of the wrong plan going:

- **It never resolves from the worktree.** Every other command works out which plan you
  mean from where you are standing; this one has to be named.
- **A terminal asks you to type the slug**; anything else has to have said `--yes`.
  Over MCP, `delete_plan` takes a `confirm` that must equal the resolved slug, so a
  loose reference cannot carry the wrong plan off.
- **A slice another worktree holds refuses the delete** until `--force`. That is live
  work belonging to somebody else.
- **`--dry-run`** prints exactly what would be destroyed and stops.

`aip export <plan> -o <file>` keeps a copy of the document first, and `aip db backup`
copies the whole database.

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
| Markdown rendering | `$AI_PLANNER_MARKDOWN` = `auto` (default), `always`, `never`; theme from gum's own `$GUM_FORMAT_THEME` |

One database for every repo - that is what lets four worktrees share a plan with no
setup. `aip db backup` takes a consistent copy while agents are writing.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --features model-embeddings
```

The board's frontend lives in `ui/` and is built into `ui/dist`, which is **committed**:
it gets compiled into the binary, so `cargo install` has to work on a machine with a
Rust toolchain and nothing else. `cargo build` rebuilds it when node is present and a
source is newer, and never fails the build if node is missing - `aip doctor` reports
which kind of binary you have.

```sh
cd ui && npm ci
npm test              # the markdown renderer and the formatters
npm run typecheck

# Iterating on the frontend: hot reload against a real board
aip ui --port 7777 --no-open
npm run dev
```

The desktop shell is out of the default workspace members, so `cargo build`, `cargo
test` and `cargo clippy` at the root skip Tauri entirely. Build it by name:

```sh
cargo build -p ai-planner-desktop
```
