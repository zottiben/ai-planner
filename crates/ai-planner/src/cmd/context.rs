use ai_planner_core::{Resolution, Status};
use anyhow::Result;
use serde::Serialize;

use crate::app::App;
use crate::cli::{CurrentArgs, StatusArgs};
use crate::cmd::slice::short_path;
use crate::out::{bold, dim, status_colour};

#[derive(Serialize)]
struct CurrentJson<'a> {
    plan: &'a str,
    title: &'a str,
    status: Status,
    slice: Option<&'a str>,
    rule: ai_planner_core::Rule,
    why: &'a str,
}

pub fn current(app: &mut App, args: &CurrentArgs, plan_ref: Option<&str>) -> Result<()> {
    let resolution = resolve(app, plan_ref)?;
    let r = &resolution;

    if app.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&CurrentJson {
                plan: &r.plan.slug,
                title: &r.plan.title,
                status: r.plan.status,
                slice: r.slice.as_ref().map(|s| s.key.as_str()),
                rule: r.rule,
                why: r.rule.why(),
            })?
        );
        return Ok(());
    }

    match &r.slice {
        Some(slice) => println!("{} · {}", bold(&r.plan.slug), slice.key),
        None => println!("{}", bold(&r.plan.slug)),
    }
    if args.why {
        println!("{}", dim(&format!("  {}", r.rule.why())));
    }
    Ok(())
}

pub fn status(app: &mut App, args: &StatusArgs, plan_ref: Option<&str>) -> Result<()> {
    let resolution = match resolve(app, plan_ref) {
        Ok(r) => r,
        // Not resolving is a normal state, not a crash: say what is available.
        Err(err) if !app.json => {
            println!("{err}");
            return list_candidates(app);
        }
        Err(err) => return Err(err),
    };

    let plan = &resolution.plan;
    let slices = app.store.slices(plan.id)?;
    let done = slices.iter().filter(|s| s.status == Status::Done).count();
    let worktree = app.git.as_ref().map(|g| g.worktree_str());
    let next = app.store.next_slice(plan.id, worktree.as_deref())?;

    if app.json {
        let bundle = app.store.bundle(plan.id)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "plan": plan,
                "rule": resolution.rule,
                "why": resolution.rule.why(),
                "on": resolution.slice,
                "next": next,
                "done": done,
                "total": slices.len(),
                "open_questions": bundle.questions.iter().filter(|q| q.status == "open").count(),
            }))?
        );
        return Ok(());
    }

    if args.oneline {
        let on = resolution
            .slice
            .as_ref()
            .map(|s| format!(" · {} {}", s.key, s.status))
            .unwrap_or_default();
        println!(
            "{} - {} ({}){on} · {done}/{} slices",
            plan.slug,
            plan.title,
            plan.status,
            slices.len()
        );
        return Ok(());
    }

    println!(
        "{} {}  {}",
        bold(&plan.slug),
        status_colour(plan.status, plan.status.as_str()),
        plan.title
    );
    println!("{}", dim(&format!("  {}", resolution.rule.why())));
    println!();

    if let Some(git) = &app.git {
        let branch = git.branch.as_deref().unwrap_or("detached HEAD");
        let sha = git
            .head_sha
            .as_deref()
            .map(|s| format!(" @ {s}"))
            .unwrap_or_default();
        println!(
            "  {}  {}  {}",
            dim("worktree"),
            short_path(&git.worktree_str()),
            dim(&format!("({branch}{sha})"))
        );
    }
    println!("  {}  {done}/{} slices done", dim("progress"), slices.len());
    println!();

    if let Some(on) = &resolution.slice {
        let here = on.worktree_path == worktree && on.claimed_by.is_some();
        println!(
            "  {}        {}  {}  {}{}",
            dim("on"),
            on.key,
            status_colour(on.status, on.status.as_str()),
            on.title,
            if here {
                dim("  (claimed here)")
            } else {
                String::new()
            }
        );
    }
    match &next {
        Some(n) if Some(n.key.as_str()) != resolution.slice.as_ref().map(|s| s.key.as_str()) => {
            println!(
                "  {}    {:<4} {}  {}",
                dim("next"),
                n.key,
                status_colour(n.status, &pad_status(n.status)),
                n.title
            );
        }
        None if done == slices.len() && !slices.is_empty() => {
            println!("  {}    every slice is done", dim("next"));
        }
        _ => {}
    }

    let questions = app.store.questions(plan.id, true)?;
    if !questions.is_empty() {
        println!("\n  {} ({})", dim("open questions"), questions.len());
        for q in questions.iter().take(5) {
            let scope = q
                .slice_key
                .as_deref()
                .map(|k| format!("({k}) "))
                .unwrap_or_default();
            println!("    - {scope}{}", q.body);
        }
    }

    let log = app.store.log(plan.id, Some(3))?;
    if !log.is_empty() {
        println!("\n  {}", dim("recent"));
        for e in log {
            let date = e.at.split('T').next().unwrap_or(&e.at);
            let first = e.body.lines().next().unwrap_or_default();
            println!("    {} {}", dim(date), first);
        }
    }
    Ok(())
}

/// Statuses vary in width; padding before colouring keeps the columns straight.
fn pad_status(status: Status) -> String {
    format!("{:<9}", status.as_str())
}

/// Resolve and remember. Confirming a resolution is exactly when the association is
/// worth learning, so this is the one place affinity is recorded (D9).
fn resolve(app: &mut App, plan_ref: Option<&str>) -> Result<Resolution> {
    let git = app.git.clone();
    let resolved = app.store.resolve(git.as_ref(), plan_ref)?;
    match resolved {
        Ok(r) => {
            if let (Some(git), Some(repo_id)) = (&git, app.repo_id()) {
                app.store.record_affinity(
                    r.plan.id,
                    repo_id,
                    git.branch.as_deref(),
                    &git.worktree_str(),
                )?;
            }
            Ok(r)
        }
        Err(unresolved) => anyhow::bail!("{}", unresolved.reason),
    }
}

fn list_candidates(app: &App) -> Result<()> {
    let plans = app.store.list_plans(&ai_planner_core::PlanFilter {
        repo_id: app.repo_id(),
        statuses: Status::INCOMPLETE.to_vec(),
        query: None,
    })?;
    if plans.is_empty() {
        println!("{}", dim("`aip new \"<title>\"` starts one"));
        return Ok(());
    }
    println!("{}", dim("pick one with -p:"));
    for p in plans {
        println!("  {}  {}", p.slug, p.title);
    }
    Ok(())
}
