use ai_planner_core::render::slice_meta_line;
use ai_planner_core::{NewSlice, SliceUpdate, Status};
use anyhow::Result;

use crate::app::App;
use crate::cli::*;
use crate::out::{dim, ok, status_colour, Table};
use crate::read_body;

pub fn run(app: &mut App, cmd: &SliceCmd, plan_ref: Option<&str>) -> Result<()> {
    match cmd {
        SliceCmd::Add(args) => add(app, args, plan_ref),
        SliceCmd::Ls => ls(app, plan_ref),
        SliceCmd::Show { key } => show(app, key, plan_ref),
        SliceCmd::Set(args) => set(app, args, plan_ref),
        SliceCmd::Edit(args) => edit(app, args, plan_ref),
        SliceCmd::Claim { key } => claim(app, key, plan_ref),
        SliceCmd::Release { key } => release(app, key, plan_ref),
        SliceCmd::Stale => stale(app),
    }
}

fn add(app: &mut App, args: &SliceAddArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let scope = match (&args.scope, &args.scope_file) {
        (None, None) => None,
        _ => Some(read_body(
            args.scope.as_deref(),
            args.scope_file.as_deref(),
        )?),
    };
    let slice = app.store.add_slice(NewSlice {
        plan_id: plan.id,
        key: args.key.clone(),
        title: args.title.clone(),
        status: Some(Status::parse(&args.status)?),
        scope_md: scope,
        demo_md: args.demo.clone(),
        estimate_files: args.files,
        branch: args.branch.clone(),
        base_branch: args.base.clone().or_else(|| plan.base_branch.clone()),
        ord: args.ord,
    })?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&slice)?);
    } else {
        ok(&format!("{} · {} - {}", plan.slug, slice.key, slice.title));
    }
    Ok(())
}

fn ls(app: &App, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let slices = app.store.slices(plan.id)?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&slices)?);
        return Ok(());
    }
    if slices.is_empty() {
        println!("{}", dim("no slices yet - `aip slice add PR1 \"<title>\"`"));
        return Ok(());
    }

    let here = app.git.as_ref().map(|g| g.worktree_str());
    let mut table = Table::new(&["", "KEY", "STATUS", "BRANCH", "CLAIMED", "TITLE"]);
    for s in &slices {
        // Mark what is claimed in *this* worktree, which is the thing you want to see
        // when four of them are busy.
        let claimed = match (&s.claimed_by, &s.worktree_path) {
            (Some(by), Some(wt)) if Some(wt) == here.as_ref() => format!("{by} (here)"),
            (Some(by), Some(wt)) => format!("{by} in {}", short_path(wt)),
            (Some(by), None) => by.clone(),
            _ => String::new(),
        };
        table.row(vec![
            status_colour(s.status, s.status.marker()),
            s.key.clone(),
            status_colour(s.status, s.status.as_str()),
            s.branch.clone().unwrap_or_default(),
            claimed,
            s.title.clone(),
        ]);
    }
    table.print();
    Ok(())
}

fn show(app: &App, key: &str, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let slice = app.store.require_slice(plan.id, key)?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&slice)?);
        return Ok(());
    }

    println!("# {} - {}\n", slice.key, slice.title);
    println!("{}\n", slice_meta_line(&slice));
    if !slice.scope_md.trim().is_empty() {
        println!("{}\n", slice.scope_md.trim_end());
    }
    if let Some(demo) = slice.demo_md.as_deref().filter(|d| !d.trim().is_empty()) {
        println!("**Demo:** {demo}\n");
    }
    if let Some(reason) = slice.blocked_reason.as_deref() {
        println!("**Blocked:** {reason}\n");
    }

    let history = app.store.slice_log(slice.id, Some(10))?;
    if !history.is_empty() {
        println!("## History\n");
        for e in history {
            let date = e.at.split('T').next().unwrap_or(&e.at);
            println!("- {date} - {}", e.body.replace('\n', " "));
        }
    }
    Ok(())
}

fn set(app: &mut App, args: &SliceSetArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let slice = app.store.require_slice(plan.id, &args.key)?;
    let status = Status::parse(&args.status)?;
    if status == Status::Blocked && args.reason.is_none() {
        println!(
            "{}",
            dim("note: blocking without --reason leaves the next session guessing")
        );
    }
    let updated = app
        .store
        .set_slice_status(&slice, status, args.reason.as_deref())?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&updated)?);
    } else {
        ok(&format!(
            "{} · {} is now {}",
            plan.slug,
            updated.key,
            status_colour(updated.status, updated.status.as_str())
        ));
        report_remaining(app, plan.id)?;
    }
    Ok(())
}

fn edit(app: &mut App, args: &SliceEditArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let slice = app.store.require_slice(plan.id, &args.key)?;
    let scope = match (&args.scope, &args.scope_file) {
        (None, None) => None,
        _ => Some(read_body(
            args.scope.as_deref(),
            args.scope_file.as_deref(),
        )?),
    };
    let updated = app.store.update_slice(
        &slice,
        SliceUpdate {
            title: args.title.clone(),
            scope_md: scope,
            demo_md: args.demo.clone(),
            estimate_files: args.files,
            branch: args.branch.clone(),
            base_branch: args.base.clone(),
            pr_url: args.pr.clone(),
            blocked_reason: None,
        },
    )?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&updated)?);
    } else {
        ok(&format!("updated {} · {}", plan.slug, updated.key));
    }
    Ok(())
}

fn claim(app: &mut App, key: &str, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let git = app.require_git()?.clone();
    let slice = app.store.require_slice(plan.id, key)?;
    let claimed = app
        .store
        .claim_slice(&slice, &git.worktree_str(), git.branch.as_deref())?;
    // Claiming is the strongest possible confirmation of "this worktree means this
    // plan", so it teaches the association too.
    if let Some(repo_id) = app.repo_id() {
        app.store
            .record_affinity(plan.id, repo_id, git.branch.as_deref(), &git.worktree_str())?;
    }

    if app.json {
        println!("{}", serde_json::to_string_pretty(&claimed)?);
    } else {
        ok(&format!(
            "{} · {} claimed for {}",
            plan.slug,
            claimed.key,
            short_path(&git.worktree_str())
        ));
        if let Some(branch) = &claimed.branch {
            println!("{}", dim(&format!("  branch {branch}")));
        }
    }
    Ok(())
}

fn release(app: &mut App, key: &str, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let slice = app.store.require_slice(plan.id, key)?;
    let released = app.store.release_slice(&slice)?;
    ok(&format!("{} · {} released", plan.slug, released.key));
    Ok(())
}

fn stale(app: &App) -> Result<()> {
    let stale = app.store.stale_claims()?;
    if app.json {
        println!("{}", serde_json::to_string_pretty(&stale)?);
        return Ok(());
    }
    if stale.is_empty() {
        ok("no stale claims");
        return Ok(());
    }
    println!(
        "{}",
        dim("claimed in worktrees that no longer exist - release them if the work is gone:")
    );
    let mut table = Table::new(&["KEY", "STATUS", "CLAIMED BY", "WORKTREE"]);
    for s in &stale {
        table.row(vec![
            s.key.clone(),
            s.status.to_string(),
            s.claimed_by.clone().unwrap_or_default(),
            s.worktree_path.clone().unwrap_or_default(),
        ]);
    }
    table.print();
    Ok(())
}

fn report_remaining(app: &App, plan_id: i64) -> Result<()> {
    let slices = app.store.slices(plan_id)?;
    let done = slices.iter().filter(|s| s.status == Status::Done).count();
    let next = slices
        .iter()
        .find(|s| matches!(s.status, Status::Ready | Status::Active));
    match next {
        Some(n) => println!(
            "{}",
            dim(&format!(
                "  {done}/{} done · next: {} - {}",
                slices.len(),
                n.key,
                n.title
            ))
        ),
        None if done == slices.len() && !slices.is_empty() => {
            println!("{}", dim("  every slice is done"))
        }
        None => {}
    }
    Ok(())
}

/// Worktree paths are long and mostly boilerplate; the tail is what identifies them.
pub fn short_path(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let trimmed = path.strip_prefix(&home).map(|r| format!("~{r}"));
    trimmed.unwrap_or_else(|| path.to_string())
}
