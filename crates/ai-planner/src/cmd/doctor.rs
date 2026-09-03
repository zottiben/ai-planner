use std::path::{Path, PathBuf};

use ai_planner_core::{PlanFilter, Status};
use anyhow::Result;

use crate::app::App;
use crate::cmd::slice::short_path;
use crate::out::{colour, dim};

enum Check {
    Ok(String),
    Warn(String, String),
}

pub fn doctor(app: &mut App) -> Result<()> {
    let mut checks: Vec<Check> = Vec::new();

    let db = app.db_path();
    checks.push(Check::Ok(format!("database {}", db.display())));

    let pending = app.store.db().pending_migrations()?;
    if pending == 0 {
        checks.push(Check::Ok(format!(
            "schema v{}, up to date",
            app.store.db().schema_version()?
        )));
    } else {
        checks.push(Check::Warn(
            format!("{pending} migration(s) pending"),
            "run any aip command to apply them".into(),
        ));
    }

    match (&app.repo, &app.git) {
        (Some(repo), Some(git)) => {
            checks.push(Check::Ok(format!("repo {} registered", repo.name)));
            if git.is_linked_worktree() {
                checks.push(Check::Ok(format!(
                    "worktree {} shares the database with the main checkout",
                    short_path(&git.worktree_str())
                )));
            }
        }
        (None, Some(git)) => checks.push(Check::Warn(
            format!("{} is not registered", git.repo_key),
            "run `aip init` here".into(),
        )),
        _ => checks.push(Check::Warn(
            "not inside a git repository".into(),
            "most commands need one".into(),
        )),
    }

    // Claims left behind by worktrees that have since been destroyed. Reported, never
    // cleaned up automatically - the work may be real even when the checkout is gone.
    let stale = app.store.stale_claims()?;
    if stale.is_empty() {
        checks.push(Check::Ok("no stale claims".into()));
    } else {
        for s in &stale {
            let plan = app.store.get_plan(s.plan_id)?;
            checks.push(Check::Warn(
                format!(
                    "{} · {} is claimed in a worktree that no longer exists",
                    plan.slug, s.key
                ),
                format!("aip -p {} slice release {}", plan.slug, s.key),
            ));
        }
    }

    // Blocked with no reason is the state that costs the next session the most.
    for plan in app.store.list_plans(&PlanFilter {
        repo_id: app.repo_id(),
        ..Default::default()
    })? {
        for slice in app.store.slices(plan.id)? {
            if slice.status == Status::Blocked
                && slice
                    .blocked_reason
                    .as_deref()
                    .is_none_or(|r| r.trim().is_empty())
            {
                checks.push(Check::Warn(
                    format!(
                        "{} · {} is blocked with no reason recorded",
                        plan.slug, slice.key
                    ),
                    format!(
                        "aip -p {} slice set {} blocked --reason \"...\"",
                        plan.slug, slice.key
                    ),
                ));
            }
        }
    }

    // The files this tool exists to replace, still sitting in a worktree.
    let leftovers = leftover_files(app)?;
    let mut imported: Vec<String> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for path in &leftovers {
        let shown = short_path(&path.to_string_lossy());
        if app.store.import_path_seen(&path.to_string_lossy())? {
            imported.push(shown);
        } else {
            unknown.push(shown);
        }
    }
    if leftovers.is_empty() {
        checks.push(Check::Ok("no plan or handoff markdown left on disk".into()));
    }
    if !unknown.is_empty() {
        checks.push(Check::Warn(
            format!(
                "{} plan/handoff file(s) are not in the database",
                unknown.len()
            ),
            format!("aip import {}", unknown.join(" ")),
        ));
    }
    if !imported.is_empty() {
        checks.push(Check::Warn(
            format!(
                "{} plan/handoff file(s) are imported and can be deleted",
                imported.len()
            ),
            format!("rm {}", imported.join(" ")),
        ));
    }

    // The half of an install that lives outside the binary, and so goes stale quietly.
    let rules_installed = crate::rules::user_targets().iter().any(|(_, path)| {
        std::fs::read_to_string(path)
            .map(|t| t.contains(crate::rules::MARKER))
            .unwrap_or(false)
    });
    if rules_installed {
        checks.push(Check::Ok(
            "the always-on rules are in the global charter".into(),
        ));
    } else {
        checks.push(Check::Warn(
            "the always-on rules are not in any global charter".into(),
            "aip rules install".into(),
        ));
    }

    let skills = crate::update::skill_targets(false);
    let stale: Vec<String> = skills
        .iter()
        .map(|dir| dir.join("SKILL.md"))
        .filter(|path| {
            std::fs::read_to_string(path)
                .map(|t| t != crate::update::SKILL)
                .unwrap_or(true)
        })
        .map(|path| short_path(&path.to_string_lossy()))
        .collect();
    if stale.is_empty() {
        checks.push(Check::Ok("the installed skill matches this binary".into()));
    } else {
        checks.push(Check::Warn(
            format!(
                "the skill is missing or out of date in {}",
                stale.join(", ")
            ),
            "aip setup".into(),
        ));
    }

    let rows = app.store.search_rows()?;
    if rows == 0 {
        checks.push(Check::Warn(
            "search index is empty".into(),
            "aip find --reindex <query>".into(),
        ));
    } else {
        checks.push(Check::Ok(format!("search index holds {rows} rows")));
    }

    let warns = checks
        .iter()
        .filter(|c| matches!(c, Check::Warn(..)))
        .count();
    for check in &checks {
        match check {
            Check::Ok(msg) => println!("{} {msg}", tick()),
            Check::Warn(msg, fix) => {
                println!("{} {msg}", cross());
                println!("  {}", dim(fix));
            }
        }
    }
    println!();
    if warns == 0 {
        println!("all good");
    } else {
        println!("{warns} thing(s) to look at");
    }
    Ok(())
}

fn tick() -> String {
    if colour() {
        "\x1b[32m✓\x1b[0m".into()
    } else {
        "ok  ".into()
    }
}

fn cross() -> String {
    if colour() {
        "\x1b[33m!\x1b[0m".into()
    } else {
        "!   ".into()
    }
}

/// Plan and handoff markdown in this repo's checkouts, including its worktrees.
fn leftover_files(app: &App) -> Result<Vec<PathBuf>> {
    let Some(git) = &app.git else {
        return Ok(Vec::new());
    };
    let mut roots = vec![git.worktree.clone(), git.main_path.clone()];
    if let Ok(out) = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&git.worktree)
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                roots.push(PathBuf::from(path));
            }
        }
    }
    roots.sort();
    roots.dedup();

    let mut found: Vec<PathBuf> = Vec::new();
    for root in roots {
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_uppercase();
            if !name.ends_with(".MD") {
                continue;
            }
            if name.contains("BUILD_PLAN")
                || name.contains("BUILD-PLAN")
                || name.starts_with("HANDOFF")
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found.dedup_by(|a, b| same_file(a, b));
    Ok(found)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}
