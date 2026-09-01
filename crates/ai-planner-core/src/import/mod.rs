pub mod ingest;
pub mod parse;

pub use ingest::{looks_like_handoff, sha256, ImportOptions, Outcome};
pub use parse::{parse_plan, ParsedDecision, ParsedLog, ParsedPlan, ParsedSection, ParsedSlice};

/// Lift a handoff's "Gotchas" section into rows. This is the highest-value part of a
/// handoff - the API quirks and verification tricks the code alone does not reveal -
/// and it is the part that gets lost when a HANDOFF.md is deleted.
pub fn handoff_gotchas(md: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut in_section = false;
    let mut current: Option<(String, Vec<String>)> = None;
    let mut fenced = false;

    for line in md.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && line.starts_with("## ") {
            if let Some((title, body)) = current.take() {
                out.push((title, body.join("\n").trim().to_string()));
            }
            in_section = line[3..].trim().to_lowercase().contains("gotcha");
            continue;
        }
        if !in_section {
            continue;
        }
        if !fenced && line.starts_with("### ") {
            if let Some((title, body)) = current.take() {
                out.push((title, body.join("\n").trim().to_string()));
            }
            current = Some((line[4..].trim().to_string(), Vec::new()));
            continue;
        }
        if let Some((_, body)) = current.as_mut() {
            body.push(line.to_string());
        }
    }
    if let Some((title, body)) = current.take() {
        out.push((title, body.join("\n").trim().to_string()));
    }
    out.retain(|(t, _)| !t.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gotchas_are_lifted_out_of_a_handoff_and_nothing_else_is() {
        let md = "# HANDOFF - ACME-1234\n\n\
                  ## RESUME HERE\n\nStuff.\n\n\
                  ## Gotchas\n\n\
                  ### Verifying in the browser costs you the shared Herd symlink\n\n\
                  `app.test` is one symlink shared by every worktree.\n\n\
                  ### Test queries\n\n\
                  Use `getByLabelText`.\n\n\
                  ## Open questions for Ben\n\nNone.\n";
        let gotchas = handoff_gotchas(md);
        assert_eq!(gotchas.len(), 2);
        assert!(gotchas[0].0.starts_with("Verifying in the browser"));
        assert!(gotchas[0].1.contains("one symlink shared"));
        assert_eq!(gotchas[1].0, "Test queries");
        assert!(!gotchas[1].1.contains("Open questions"));
    }

    #[test]
    fn a_handoff_is_told_apart_from_a_plan() {
        use std::path::Path;
        assert!(looks_like_handoff(
            Path::new("HANDOFF-ACME-1234.md"),
            "# Something\n"
        ));
        assert!(looks_like_handoff(
            Path::new("notes.md"),
            "# HANDOFF - ACME-1234 Reusable Date Range Picker\n"
        ));
        assert!(!looks_like_handoff(
            Path::new("ACME-1234_BUILD_PLAN.md"),
            "# ACME-1234 - Reusable Date Range Picker\n"
        ));
    }
}
