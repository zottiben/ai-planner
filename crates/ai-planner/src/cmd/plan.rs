use ai_planner_core::{
    render_plan, GitContext, NewPlan, PlanFilter, PlanUpdate, Renders, SectionWrite, Status, Store,
};
use anyhow::{Context, Result};

use crate::app::App;
use crate::cli::*;
use crate::out::{bold, dim, ok, status_colour, Table};
use crate::read_body;

pub fn init(db: Option<&std::path::Path>, cwd: &std::path::Path, args: &InitArgs) -> Result<()> {
    let path = db
        .map(|p| p.to_path_buf())
        .unwrap_or_else(ai_planner_core::default_db_path);
    let fresh = !path.exists();
    let mut store = Store::init(&path).with_context(|| format!("creating {}", path.display()))?;
    if fresh {
        ok(&format!("created {}", path.display()));
    }

    let mut git = GitContext::detect(cwd)
        .with_context(|| format!("{} is not inside a git repository", cwd.display()))?;
    if let Some(name) = &args.name {
        git.repo_name = name.clone();
    }
    let repo = store.ensure_repo(&git)?;

    ok(&format!("registered {} ({})", repo.name, repo.key));
    if git.is_linked_worktree() {
        println!(
            "{}",
            dim(&format!(
                "  worktree {} shares this database with {}",
                git.worktree.display(),
                git.main_path.display()
            ))
        );
    }
    println!("{}", dim(&format!("  database  {}", path.display())));
    println!("\nNext: {}", bold("aip new \"<title>\""));
    Ok(())
}

pub fn new(app: &mut App, args: &NewArgs) -> Result<()> {
    let repo = app.require_repo()?.clone();
    let git = app.git.clone();
    let plan = app.store.create_plan(NewPlan {
        repo_id: repo.id,
        title: args.title.clone(),
        slug: args.slug.clone(),
        status: Some(Status::parse(&args.status)?),
        summary: args.summary.clone(),
        ticket_key: args.ticket.clone(),
        ticket_url: args.ticket_url.clone(),
        base_branch: args
            .base
            .clone()
            .or_else(|| git.as_ref().and_then(|g| g.branch.clone())),
        owner: args.owner.clone(),
        raw_md: None,
        source_path: None,
        bare: false,
    })?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }
    ok(&format!("{} - {}", plan.slug, plan.title));
    println!(
        "{}",
        dim("  fill it in with `aip section`, `aip decision add`, `aip slice add`")
    );
    Ok(())
}

pub fn ls(app: &App, args: &LsArgs) -> Result<()> {
    let mut statuses = Vec::new();
    for s in &args.status {
        statuses.push(Status::parse(s)?);
    }
    if args.incomplete {
        statuses.extend_from_slice(&Status::INCOMPLETE);
    }

    let filter = PlanFilter {
        repo_id: if args.all { None } else { app.repo_id() },
        statuses,
        query: args.query.clone(),
    };
    let plans = app.store.list_plans(&filter)?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&plans)?);
        return Ok(());
    }
    if plans.is_empty() {
        println!("{}", dim("no plans - `aip new \"<title>\"` starts one"));
        return Ok(());
    }

    let mut table = Table::new(&["", "PLAN", "STATUS", "SLICES", "REPO", "TITLE"]);
    for plan in &plans {
        let slices = app.store.slices(plan.id)?;
        let done = slices.iter().filter(|s| s.status == Status::Done).count();
        let progress = if slices.is_empty() {
            "-".to_string()
        } else {
            format!("{done}/{}", slices.len())
        };
        table.row(vec![
            status_colour(plan.status, plan.status.marker()),
            plan.slug.clone(),
            status_colour(plan.status, plan.status.as_str()),
            progress,
            plan.repo_name.clone(),
            plan.title.clone(),
        ]);
    }
    table.print();
    Ok(())
}

pub fn show(app: &App, args: &ShowArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(args.plan.as_deref().or(plan_ref))?;

    if args.raw {
        match app.store.raw_md(plan.id)? {
            Some(raw) => print!("{raw}"),
            None => anyhow::bail!("{} was not imported from a file", plan.slug),
        }
        return Ok(());
    }

    if let Some(key) = &args.section {
        let section = app
            .store
            .section(plan.id, key)?
            .ok_or_else(|| anyhow::anyhow!("{} has no section {key:?}", plan.slug))?;
        if app.json {
            println!("{}", serde_json::to_string_pretty(&section)?);
        } else {
            println!("{}", section.body);
        }
        return Ok(());
    }

    let bundle = app.store.bundle(plan.id)?;
    if app.json {
        println!("{}", serde_json::to_string_pretty(&bundle)?);
    } else {
        println!("{}", render_plan(&bundle));
    }
    Ok(())
}

pub fn set(app: &mut App, args: &SetArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(args.plan.as_deref().or(plan_ref))?;
    let status = Status::parse(&args.status)?;
    let updated = app.store.set_plan_status(&plan, status)?;
    if app.json {
        println!("{}", serde_json::to_string_pretty(&updated)?);
    } else {
        ok(&format!(
            "{} is now {}",
            updated.slug,
            status_colour(updated.status, updated.status.as_str())
        ));
    }
    Ok(())
}

pub fn edit(app: &mut App, args: &EditArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let updated = app.store.update_plan(
        &plan,
        PlanUpdate {
            title: args.title.clone(),
            summary: args.summary.clone(),
            ticket_key: args.ticket.clone(),
            ticket_url: args.ticket_url.clone(),
            base_branch: args.base.clone(),
            owner: args.owner.clone(),
        },
    )?;
    if app.json {
        println!("{}", serde_json::to_string_pretty(&updated)?);
    } else {
        ok(&format!("updated {}", updated.slug));
    }
    Ok(())
}

pub fn section(app: &mut App, args: &SectionArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let incoming = read_body(args.body.as_deref(), args.file.as_deref())?;

    let body = if args.append {
        let existing = app
            .store
            .section(plan.id, &args.key)?
            .map(|s| s.body)
            .unwrap_or_default();
        if existing.trim().is_empty() {
            incoming
        } else {
            format!("{}\n\n{}", existing.trim_end(), incoming.trim())
        }
    } else {
        incoming
    };

    let renders = match &args.renders {
        Some(r) => Some(Renders::parse(r)?),
        None => None,
    };
    let section = app.store.set_section(
        plan.id,
        SectionWrite {
            key: &args.key,
            title: args.title.as_deref(),
            body: &body,
            renders,
            ord: args.ord,
            expect_rev: args.expect_rev,
        },
    )?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&section)?);
    } else {
        ok(&format!(
            "{} · {} (rev {})",
            plan.slug, section.title, section.rev
        ));
    }
    Ok(())
}

pub fn source(app: &mut App, args: &SourceArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    app.store
        .add_source(plan.id, &args.kind, &args.reference, args.note.as_deref())?;
    ok(&format!("{} grounded on {}", plan.slug, args.reference));
    Ok(())
}

pub fn repos(app: &App) -> Result<()> {
    let repos = app.store.repos()?;
    if app.json {
        println!("{}", serde_json::to_string_pretty(&repos)?);
        return Ok(());
    }
    if repos.is_empty() {
        println!("{}", dim("no repos registered - run `aip init` in one"));
        return Ok(());
    }
    let mut table = Table::new(&["REPO", "PLANS", "KEY", "MAIN CHECKOUT"]);
    for repo in &repos {
        let count = app.store.list_plans(&PlanFilter {
            repo_id: Some(repo.id),
            ..Default::default()
        })?;
        table.row(vec![
            repo.name.clone(),
            count.len().to_string(),
            repo.key.clone(),
            repo.main_path.clone().unwrap_or_default(),
        ]);
    }
    table.print();
    Ok(())
}
