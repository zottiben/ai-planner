use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ai_planner_core::import::{looks_like_handoff, ImportOptions, Outcome};
use ai_planner_core::{render_plan, GitContext};
use anyhow::{Context, Result};

use crate::app::App;
use crate::cli::{ExportArgs, ImportArgs};
use crate::cmd::slice::short_path;
use crate::out::{bold, dim, ok};

pub fn import(app: &mut App, args: &ImportArgs) -> Result<()> {
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    for path in &args.paths {
        if path.is_dir() {
            collect(path, 0, &mut files)?;
        } else {
            files.insert(path.clone());
        }
    }
    for dir in &args.scan {
        collect(dir, 0, &mut files)?;
    }
    if files.is_empty() {
        anyhow::bail!("nothing to import - pass files, a directory, or --scan <dir>");
    }

    // Plans before handoffs: a handoff attaches to a plan that has to exist first.
    let mut ordered: Vec<PathBuf> = files.into_iter().collect();
    ordered.sort_by_key(|p| {
        let md = std::fs::read_to_string(p).unwrap_or_default();
        (looks_like_handoff(p, &md), p.clone())
    });

    let opts = ImportOptions {
        replace: args.replace,
        dry_run: args.dry_run,
    };
    if opts.dry_run {
        println!("{}", dim("dry run - nothing is written"));
    }

    let mut conflicts = 0;
    for path in &ordered {
        let md =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let repo_id = match GitContext::detect(dir) {
            Ok(git) => app.store.ensure_repo(&git)?.id,
            Err(_) => match app.repo_id() {
                Some(id) => id,
                None => {
                    println!("  {} {}", dim("skip"), short_path(&path.to_string_lossy()));
                    println!("       {}", dim("not inside a git repository"));
                    continue;
                }
            },
        };

        let mut outcome = app.store.import_file(repo_id, path, &md, opts)?;
        // `--as` only rescues what nothing else could identify; it never overrides a
        // match the document made for itself.
        if let (Outcome::Skipped { .. }, Some(target), true) =
            (&outcome, &args.attach_to, looks_like_handoff(path, &md))
        {
            let plan = app.store.find_plan(target, Some(repo_id))?;
            let worktree = GitContext::detect(dir)
                .map(|g| g.worktree_str())
                .unwrap_or_else(|_| dir.to_string_lossy().to_string());
            if !opts.dry_run {
                app.store.attach_handoff(plan.id, path, &md, &worktree)?;
            }
            outcome = Outcome::HandoffAttached { plan, worktree };
        }
        if matches!(outcome, Outcome::Conflict { .. }) {
            conflicts += 1;
        }
        report(path, &outcome);
    }

    if conflicts > 0 {
        println!();
        println!(
            "{}",
            dim(&format!(
                "{conflicts} conflict(s): two copies of one plan have drifted apart. \
                 Compare them, then re-run with --replace to keep the one you passed."
            ))
        );
    }
    Ok(())
}

fn report(path: &Path, outcome: &Outcome) {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    match outcome {
        Outcome::Created {
            plan,
            slices,
            decisions,
            log,
        } => {
            ok(&format!("{} <- {name}", bold(&plan.slug)));
            println!(
                "       {}",
                dim(&format!(
                    "{slices} slices · {decisions} decisions · {log} log entries · {}",
                    plan.status
                ))
            );
        }
        Outcome::AlreadyImported { plan, first_seen } => {
            println!("  {} {name}", dim("dup "));
            println!(
                "       {}",
                dim(&format!(
                    "identical to {} already imported as {}",
                    short_path(first_seen),
                    plan.slug
                ))
            );
        }
        Outcome::Conflict {
            plan,
            existing_sources,
        } => {
            println!("  {} {name}", dim("!   "));
            println!(
                "       {}",
                dim(&format!(
                    "{} already exists, imported from {}",
                    plan.slug,
                    existing_sources
                        .iter()
                        .map(|s| short_path(s))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            );
        }
        Outcome::Replaced { plan } => ok(&format!("{} replaced from {name}", bold(&plan.slug))),
        Outcome::HandoffAttached { plan, worktree } => {
            ok(&format!("{} <- {name}", bold(&plan.slug)));
            println!(
                "       {}",
                dim(&format!("handoff for {}", short_path(worktree)))
            );
        }
        Outcome::Skipped { reason } => {
            println!("  {} {name}", dim("skip"));
            println!("       {}", dim(reason));
        }
        Outcome::Planned {
            title,
            slug,
            slices,
            decisions,
            log,
        } => {
            println!("  {} {name}", dim("would"));
            println!(
                "       {}",
                dim(&format!(
                    "{slug} - {title} ({slices} slices, {decisions} decisions, {log} log entries)"
                ))
            );
        }
    }
}

/// Find the documents this tool replaces. Depth is capped and the usual dependency
/// directories are skipped, so scanning a worktree root does not walk a whole repo.
fn collect(dir: &Path, depth: usize, out: &mut BTreeSet<PathBuf>) -> Result<()> {
    if depth > 4 {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "node_modules" | "vendor" | "target" | "dist" | "build" | "storage"
                )
            {
                continue;
            }
            collect(&path, depth + 1, out)?;
            continue;
        }
        if !name.to_lowercase().ends_with(".md") {
            continue;
        }
        let upper = name.to_uppercase();
        if upper.contains("BUILD_PLAN")
            || upper.contains("BUILD-PLAN")
            || upper.starts_with("HANDOFF")
        {
            out.insert(path);
        }
    }
    Ok(())
}

pub fn export(app: &App, args: &ExportArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(args.plan.as_deref().or(plan_ref))?;
    let md = if args.raw {
        app.store
            .raw_md(plan.id)?
            .ok_or_else(|| anyhow::anyhow!("{} was not imported from a file", plan.slug))?
    } else {
        render_plan(&app.store.bundle(plan.id)?)
    };

    match &args.out {
        Some(path) => {
            if path.exists() && !args.force {
                anyhow::bail!(
                    "{} already exists - pass --force to overwrite",
                    path.display()
                );
            }
            std::fs::write(path, &md).with_context(|| format!("writing {}", path.display()))?;
            ok(&format!("{} -> {}", plan.slug, path.display()));
        }
        None => print!("{md}"),
    }
    Ok(())
}
