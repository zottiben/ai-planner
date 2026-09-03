---
name: ai-planner
description: Read and update the build plan for this repo/worktree using the ai-planner MCP tools or the `aip` CLI, instead of BUILD_PLAN.md or HANDOFF.md files. Use when asked what to build next, when finishing a slice or PR, when recording a decision, gotcha or progress note, when handing off before clearing context, and when resuming a session.
---

# ai-planner: the build plan lives in the database

Build plans for this machine are rows in one SQLite database, not markdown files.
There is no `BUILD_PLAN.md` or `HANDOFF.md` to read or write, and it works identically
from every worktree.

**Do not create plan or handoff markdown files.** If you find one, offer to import it
rather than editing it.

## Two ways in - use whichever you have

If the **`ai-planner` MCP server** is connected, prefer its tools: `locate`,
`get_plan`, `get_resume`, `search_plans`, `list_slices`, `get_slice`, `claim_slice`,
`set_slice_status`, `append_log`, `add_decision`, `add_gotcha`, `open_question`,
`write_handoff`, `import_markdown`. They take structured arguments and return JSON.

Otherwise use the **`aip` CLI**, which has the same surface and takes `--json` on
every command. The sections below give the CLI form; the MCP tool of the same name
does the same thing.

## Start of session

You are usually told your plan by the session-start hook. If not, or to see more:

```sh
aip status          # where you are: plan, slice, next item, open questions, recent log
aip resume          # the full "pick this up" doc - read this after /clear or a handoff
aip show            # the whole plan as markdown, exactly like the file it replaced
```

`aip` works out which plan you are on from the worktree and branch. `aip current --why`
says which rule it used. If it cannot tell, it lists the candidates rather than
guessing - pick one with `-p <plan>`, and it remembers for next time.

## Keeping it true

The plan drifting out of step with the work is the failure that matters. Two habits
prevent it:

- **Change the slice status as part of finishing the work**, not afterwards. A PR that
  is open while the plan says `ready` makes the plan lie to the next session.
- **`aip sync`** reconciles the git-observable facts - a branch that has landed, a PR
  that opened or merged, a claim on a branch that no longer exists. `--fix` applies
  them. Run it when you finish a slice or open a PR, and whenever a hook tells you the
  plan is out of step.

```sh
aip sync            # what git and gh see that the plan does not
aip sync --fix      # apply it
```

If a hook says the plan is out of step, fix it then - do not carry it to the next task.

## While building

```sh
aip slice ls                            # the slices, their statuses, who holds what
aip slice show PR2                      # one slice in full, with its history
aip slice claim PR2                     # take it for this worktree before you start
aip slice set PR2 in_review --reason …  # ready | active | in_review | blocked | done | deferred
aip slice edit PR2 --pr <url> --branch <name>
aip log "PR2 gates green on abc1234; browser-verified." --slice PR2
```

Claim before you build. A claim is scoped to (you, this worktree), so a second agent
in another worktree is told the slice is taken instead of duplicating the work.

Record as you go rather than at the end - `aip log` is append-only and cannot conflict
with what another agent is writing.

## When you learn or decide something

```sh
aip decision add "One headless core, two shells" "The core carries all the logic…"
aip decision supersede D4 --by D12 --note "DateCalendar has no controlled month"
aip gotcha add "The Herd symlink is shared" "Repoint it, then put it back."
aip question add "range on Summary Panel - past or both?" --slice PR1
```

- **decision** - a choice with reasoning that later slices must not re-litigate.
- **gotcha** - something the code alone does not reveal, that the next session needs.
  A durable *project* rule belongs in `AGENTS.md` instead.
- **question** - needs a human answer; it surfaces in `aip status` until answered.

## Before clearing context

```sh
aip handoff write --gate typecheck=pass --gate "test=pass:731 tests" \
  --next "PR2 - button variant" --notes "…"
```

Record the gates you actually ran, with their real results. A failed gate is recorded
as failed - never write a handoff that implies green over a failure. The next session
starts with `aip resume`.

## Writing or extending a plan

```sh
aip new "ACME-1234 - Reusable Date Range Picker" --ticket-url <url> --base master
aip section grounding --file grounding.md --title "2. Grounding"
aip slice add PR1 "Shared core" --files 40 --demo "Pick Last quarter on Summary Panel"
aip source clickup 900000000000 --note "Canvas Editor list"
```

A plan is a document: keep the same shape as the ones before it - outcome, grounding
verified in code, numbered decisions, vertically-sliced PRs each with a demo, open
questions, progress log. `aip show` renders it back in that shape.

Long bodies: pass `--file`, or pipe on stdin. Section writes take `--expect-rev` to
refuse an overwrite if another agent changed it since you read it.

## Finding a plan

```sh
aip ls                          # plans in this repo
aip ls --all                    # every repo
aip ls --incomplete             # ready, active, in_review, blocked
aip find "two-pane date panel"  # search everything every plan says
```

`aip find` searches titles, sections, slices, decisions, gotchas, questions and
progress notes. Use it before deciding something - it is how you find out whether a
question has already been answered or a trap already hit. If a local model has been
installed (`aip embed`) it also matches on meaning, so a query need not share words
with the plan; `--lexical` restricts it to words for one query.

## Machine-readable

Every command takes `--json`. Use it when you need to branch on the result rather
than show it to the user.
