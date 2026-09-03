//! Everything the binary can install for itself: the skill, the always-on rules, and
//! the harness hooks. Shared by first-time install and by `aip update`, so the two can
//! never drift apart.

use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::SetupArgs;
use crate::out::{dim, ok};
use crate::update;

pub fn setup(args: &SetupArgs) -> Result<()> {
    let project = args.project;
    install_skill(project)?;
    install_rules(project, args.force)?;
    install_hooks(project)?;

    println!();
    println!(
        "{}",
        dim("Restart your agent to pick up the skill, the rules and the hooks.")
    );
    if !project {
        println!(
            "{}",
            dim("The MCP server is registered separately: install/install-mcp.sh")
        );
    }
    Ok(())
}

/// The skill is embedded in the binary, so it cannot fall behind it and installing
/// needs no clone and no network.
pub fn install_skill(project: bool) -> Result<()> {
    for dir in update::skill_targets(project) {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join("SKILL.md");
        let unchanged = std::fs::read_to_string(&path)
            .map(|existing| existing == update::SKILL)
            .unwrap_or(false);
        if unchanged {
            println!("  {} {}", dim("have"), shown(&path));
            continue;
        }
        std::fs::write(&path, update::SKILL)
            .with_context(|| format!("writing {}", path.display()))?;
        ok(&format!("skill -> {}", shown(&path)));
    }
    Ok(())
}

pub fn install_rules(project: bool, force: bool) -> Result<()> {
    // `--force` on an update: the block's wording changes between versions, and a
    // stale copy is worse than none because it tells the agent the wrong commands.
    super::rules::run(&crate::cli::RulesCmd::Install(
        crate::cli::RulesInstallArgs { project, force },
    ))
}

pub fn install_hooks(project: bool) -> Result<()> {
    let dir = update::hook_dir(project);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let script = dir.join("ai-planner-session.sh");
    std::fs::write(&script, update::HOOK_SCRIPT)
        .with_context(|| format!("writing {}", script.display()))?;
    make_executable(&script)?;

    let settings = update::settings_path(project);
    if let Some(parent) = settings.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = std::fs::read_to_string(&settings).unwrap_or_else(|_| "{}".to_string());
    let mut value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        anyhow::anyhow!(
            "{} is not valid JSON ({e}) - not touching it. Fix it and re-run.",
            settings.display()
        )
    })?;

    let added = update::merge_hooks(&mut value, &script.to_string_lossy());
    std::fs::write(
        &settings,
        format!("{}\n", serde_json::to_string_pretty(&value)?),
    )
    .with_context(|| format!("writing {}", settings.display()))?;

    if added.is_empty() {
        println!("  {} hooks in {}", dim("have"), shown(&settings));
    } else {
        ok(&format!(
            "hooks -> {} ({})",
            shown(&settings),
            added.join(", ")
        ));
    }
    println!(
        "  {}",
        dim("Codex and Pi: call `aip hook --event <name>` from your own hooks")
    );
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn shown(path: &Path) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let s = path.to_string_lossy().to_string();
    match s.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => s,
    }
}
