//! The always-on instruction block.
//!
//! A skill is *discovered* - it fires when the agent already suspects it needs one.
//! That is the wrong shape for "the build plan lives here", which has to be true at
//! every moment of every session, so it goes in the global charter instead, where it
//! is always in context.
//!
//! Kept deliberately short. The charter competes with everything else in the prompt,
//! so this is the behavioural minimum: which command at which moment. The reasoning
//! lives in the skill, which the agent can read when it needs the detail.

/// Matches the `<!-- ai-toolbox:base-charter -->` convention: one marker top and
/// bottom, so the block can be found, replaced, or removed without touching the rest
/// of the file.
pub const MARKER: &str = "<!-- ai-planner:rules -->";

pub const RULES: &str = r#"## Build plans live in a database, not in files

This machine keeps build plans as rows, reachable with the `aip` CLI (and the
`ai-planner` MCP server when it is connected). There is no `BUILD_PLAN.md` or
`HANDOFF.md`: never create one, and never edit one you find - offer `aip import`.

Read the plan before you plan. Update it as the work happens, not at the end.

| Moment | Command |
| --- | --- |
| Starting any task | `aip status` (or `aip resume` after a context clear) |
| Before building a slice | `aip slice claim <key>` |
| Opening a PR | `aip slice set <key> in_review` + `aip slice edit <key> --pr <url>` |
| PR merged | `aip slice set <key> done` |
| Anything worth knowing later | `aip log "<what happened>" --slice <key>` |
| A choice future work must not re-open | `aip decision add "<title>" "<why>"` |
| A trap the code does not reveal | `aip gotcha add "<title>" "<detail>"` |
| Something only the user can decide | `aip question add "<question>"` |
| Before `/clear` or `/compact` | `aip handoff write --gate <name>=<result>` |

Two rules that matter more than the rest:

- **A slice status change is part of finishing the work,** not paperwork after it. A PR
  that is open while the plan says `ready` makes the plan lie to the next session.
- **`aip log` is append-only and cannot conflict,** so there is never a reason to batch
  notes up. Write them when they happen.

If the plan and reality have drifted, `aip sync` reconciles what git and `gh` can see
(merged branches, open PRs, dead claims) and `aip sync --fix` applies it."#;

/// The charter files each harness reads, at user scope.
pub fn user_targets() -> Vec<(&'static str, std::path::PathBuf)> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    vec![
        ("claude", home.join(".claude/CLAUDE.md")),
        ("codex", home.join(".codex/AGENTS.md")),
        ("agents", home.join(".agents/AGENTS.md")),
    ]
}

pub fn block() -> String {
    format!("{MARKER}\n{RULES}\n{MARKER}\n")
}

/// Append the block, replace an existing one, or report that it is already current.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Added,
    Replaced,
    AlreadyCurrent,
    Removed,
    NotPresent,
}

pub fn apply(text: &str, force: bool) -> (String, Outcome) {
    let desired = block();
    match extract(text) {
        Some((start, end)) => {
            let current = &text[start..end];
            if current == desired.trim_end() {
                return (text.to_string(), Outcome::AlreadyCurrent);
            }
            if !force {
                return (text.to_string(), Outcome::AlreadyCurrent);
            }
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..start]);
            out.push_str(desired.trim_end());
            out.push_str(&text[end..]);
            (out, Outcome::Replaced)
        }
        None => {
            let mut out = text.trim_end().to_string();
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&desired);
            (out, Outcome::Added)
        }
    }
}

pub fn remove(text: &str) -> (String, Outcome) {
    match extract(text) {
        Some((start, end)) => {
            let mut out = String::with_capacity(text.len());
            out.push_str(text[..start].trim_end());
            let tail = text[end..].trim_start_matches('\n');
            if !tail.trim().is_empty() {
                out.push_str("\n\n");
                out.push_str(tail);
            } else {
                out.push('\n');
            }
            (out, Outcome::Removed)
        }
        None => (text.to_string(), Outcome::NotPresent),
    }
}

/// Byte range of the marked block, markers included.
fn extract(text: &str) -> Option<(usize, usize)> {
    let start = text.find(MARKER)?;
    let after = start + MARKER.len();
    let end = text[after..]
        .find(MARKER)
        .map(|i| after + i + MARKER.len())?;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_is_appended_once_and_then_left_alone() {
        let charter = "<!-- ai-toolbox:base-charter -->\n# Base charter\n\nBe good.\n";

        let (once, outcome) = apply(charter, false);
        assert_eq!(outcome, Outcome::Added);
        assert!(
            once.contains("# Base charter"),
            "the existing charter survives"
        );
        assert!(once.contains("aip slice claim <key>"));
        assert_eq!(once.matches(MARKER).count(), 2);

        let (twice, outcome) = apply(&once, false);
        assert_eq!(outcome, Outcome::AlreadyCurrent);
        assert_eq!(twice, once, "a second run changes nothing");
    }

    #[test]
    fn an_outdated_block_is_replaced_in_place_without_disturbing_the_rest() {
        let stale = format!(
            "# Base charter\n\nBe good.\n\n{MARKER}\nold text\n{MARKER}\n\n## Something after\n"
        );
        let (updated, outcome) = apply(&stale, true);
        assert_eq!(outcome, Outcome::Replaced);
        assert!(!updated.contains("old text"));
        assert!(updated.contains("# Base charter"));
        assert!(
            updated.contains("## Something after"),
            "trailing content survives"
        );
        assert_eq!(updated.matches(MARKER).count(), 2);
    }

    #[test]
    fn removing_it_leaves_the_surrounding_charter_intact() {
        let (with, _) = apply("# Base charter\n\nBe good.\n", false);
        let (without, outcome) = remove(&with);
        assert_eq!(outcome, Outcome::Removed);
        assert!(!without.contains(MARKER));
        assert!(without.contains("Be good."));

        let (again, outcome) = remove(&without);
        assert_eq!(outcome, Outcome::NotPresent);
        assert_eq!(again, without);
    }

    #[test]
    fn it_is_short_enough_to_live_in_an_always_on_charter() {
        // The charter competes with the whole prompt; a long block gets skimmed.
        let lines = RULES.lines().count();
        assert!(lines < 45, "the rules block has grown to {lines} lines");
    }
}
