---
name: ai-planner
description: Read and update the build plan for this repo/worktree using the `aip` CLI instead of BUILD_PLAN.md or HANDOFF.md files. Use when asked what to build next, when finishing a slice or PR, when recording a decision, gotcha or progress note, when handing off before clearing context, and when resuming a session.
---

# ai-planner: the build plan lives in the database

Build plans for this machine are rows in one SQLite database, not markdown files.
There is no `BUILD_PLAN.md` or `HANDOFF.md` to read or write - `aip` is how you read
and update the plan, and it works identically from every worktree.

**Do not create plan or handoff markdown files.** If you find one, offer to
`aip import` it rather than editing it.

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
aip ls                    # plans in this repo
aip ls --all              # every repo
aip ls --incomplete       # ready, active, in_review, blocked
aip ls "date range"       # by title, slug or ticket
```

## Machine-readable

Every command takes `--json`. Use it when you need to branch on the result rather
than show it to the user.
