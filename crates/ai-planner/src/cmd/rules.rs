use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::cli::RulesCmd;
use crate::out::{bold, dim, ok};
use crate::rules;

pub fn run(cmd: &RulesCmd) -> Result<()> {
    match cmd {
        RulesCmd::Show => {
            print!("{}", rules::block());
            Ok(())
        }
        RulesCmd::Install(args) => install(args.project, args.force),
        RulesCmd::Uninstall(args) => uninstall(args.project),
        RulesCmd::Status => status(),
    }
}

/// Where the block goes. User scope by default: a build plan is not a property of one
/// repo, and the point is that no repo has to be set up for this to work.
fn targets(project: bool) -> Vec<(String, PathBuf)> {
    if project {
        vec![
            ("project".to_string(), PathBuf::from("AGENTS.md")),
            ("project".to_string(), PathBuf::from("CLAUDE.md")),
        ]
    } else {
        rules::user_targets()
            .into_iter()
            .map(|(name, path)| (name.to_string(), path))
            .collect()
    }
}

fn install(project: bool, force: bool) -> Result<()> {
    let mut touched = 0;
    for (harness, path) in targets(project) {
        // A charter that does not exist yet means that harness is not set up here.
        // Claude Code and the .agents convention are created anyway, since those are
        // the two that always apply; the rest are left alone.
        let always =
            path.ends_with(".claude/CLAUDE.md") || path.ends_with(".agents/AGENTS.md") || project;
        if !path.exists() && !always {
            println!("  {} {}", dim("skip"), shown(&path));
            println!("      {}", dim(&format!("{harness} has no charter here")));
            continue;
        }

        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let (updated, outcome) = rules::apply(&existing, force);
        match outcome {
            rules::Outcome::AlreadyCurrent if !force => {
                println!("  {} {}", dim("have"), shown(&path));
                continue;
            }
            _ => {}
        }
        if updated == existing {
            println!("  {} {}", dim("have"), shown(&path));
            continue;
        }

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, &updated).with_context(|| format!("writing {}", path.display()))?;
        ok(&format!(
            "{} {}",
            match outcome {
                rules::Outcome::Replaced => "updated",
                _ => "added to",
            },
            shown(&path)
        ));
        touched += 1;
    }

    if touched == 0 {
        println!(
            "{}",
            dim("nothing to do - pass --force to rewrite an existing block")
        );
    } else {
        println!();
        println!(
            "The rules are now always in context. {}",
            dim("Restart your agent to pick them up.")
        );
    }
    Ok(())
}

fn uninstall(project: bool) -> Result<()> {
    for (_, path) in targets(project) {
        if !path.exists() {
            continue;
        }
        let existing = std::fs::read_to_string(&path)?;
        let (updated, outcome) = rules::remove(&existing);
        if outcome == rules::Outcome::Removed {
            std::fs::write(&path, updated)?;
            ok(&format!("removed from {}", shown(&path)));
        }
    }
    Ok(())
}

fn status() -> Result<()> {
    println!("{}", bold("charter files"));
    for (harness, path) in rules::user_targets() {
        let state = match std::fs::read_to_string(&path) {
            Ok(text) if text.contains(rules::MARKER) => "installed",
            Ok(_) => "not installed",
            Err(_) => "no charter",
        };
        println!("  {:<8} {:<14} {}", harness, state, shown(&path));
    }
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = PathBuf::from(name);
        if path.exists() {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let state = if text.contains(rules::MARKER) {
                "installed"
            } else {
                "not installed"
            };
            println!("  {:<8} {:<14} ./{name}", "project", state);
        }
    }
    Ok(())
}

fn shown(path: &std::path::Path) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let s = path.to_string_lossy().to_string();
    match s.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => s,
    }
}
