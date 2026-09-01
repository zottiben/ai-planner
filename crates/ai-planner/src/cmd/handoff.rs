use ai_planner_core::handoff::{Gate, NewHandoff};
use ai_planner_core::Status;
use anyhow::Result;

use crate::app::App;
use crate::cli::{HandoffCmd, ResumeArgs};
use crate::cmd::slice::short_path;
use crate::out::{bold, dim, ok};
use crate::read_body;

pub fn run(app: &mut App, cmd: &HandoffCmd, plan_ref: Option<&str>) -> Result<()> {
    match cmd {
        HandoffCmd::Write(args) => write(app, args, plan_ref),
        HandoffCmd::Show => show(app, plan_ref),
        HandoffCmd::Ls => ls(app, plan_ref),
    }
}

fn write(app: &mut App, args: &crate::cli::HandoffWriteArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let git = app.require_git()?.clone();

    let gates: Vec<Gate> = args.gate.iter().map(|g| Gate::parse(g)).collect();
    let resume = match (&args.notes, &args.notes_file) {
        (None, None) => String::new(),
        _ => read_body(args.notes.as_deref(), args.notes_file.as_deref())?,
    };
    let next = if args.next.is_empty() {
        // Nothing said: the plan already knows what comes next.
        match app.store.next_slice(plan.id, Some(&git.worktree_str()))? {
            Some(s) => format!("{} - {}", s.key, s.title),
            None => String::new(),
        }
    } else {
        args.next
            .iter()
            .map(|n| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    if git.dirty() && !args.allow_dirty {
        anyhow::bail!(
            "this worktree has uncommitted changes - commit and push first, or pass \
             --allow-dirty to record the handoff anyway"
        );
    }

    let handoff = app.store.write_handoff(NewHandoff {
        plan_id: plan.id,
        worktree_path: git.worktree_str(),
        branch: git.branch.clone(),
        head_sha: git.head_sha.clone(),
        gates,
        resume_md: resume,
        next_md: next,
    })?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&handoff)?);
        return Ok(());
    }

    ok(&format!(
        "handoff for {} in {}",
        bold(&plan.slug),
        short_path(&handoff.worktree_path)
    ));
    let red: Vec<&str> = handoff
        .gates
        .iter()
        .filter(|g| !g.passed())
        .map(|g| g.name.as_str())
        .collect();
    if !red.is_empty() {
        // Say it plainly rather than letting a red gate pass as a checkpoint.
        println!(
            "  {}",
            dim(&format!("RED: {} - not a green baseline", red.join(", ")))
        );
    } else if !handoff.gates.is_empty() {
        println!("  {}", dim(&format!("{} gates green", handoff.gates.len())));
    } else {
        println!(
            "  {}",
            dim("no gates recorded - the next context has nothing to trust")
        );
    }
    println!("\nResume in a fresh session with: {}", bold("aip resume"));
    Ok(())
}

fn show(app: &App, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let worktree = app.git.as_ref().map(|g| g.worktree_str());
    let handoff = worktree
        .as_deref()
        .map(|wt| app.store.latest_handoff(plan.id, wt))
        .transpose()?
        .flatten();

    match handoff {
        Some(h) if app.json => println!("{}", serde_json::to_string_pretty(&h)?),
        Some(h) => {
            println!(
                "{} · {} @ {}",
                h.at,
                h.branch.as_deref().unwrap_or("?"),
                h.head_sha.as_deref().unwrap_or("?")
            );
            for g in &h.gates {
                let mark = if g.passed() { "✔" } else { "✘" };
                println!("  {mark} {} {}", g.name, g.detail.as_deref().unwrap_or(""));
            }
            if !h.next_md.trim().is_empty() {
                println!("\n{}\n\n{}", bold("Next"), h.next_md.trim());
            }
            if !h.resume_md.trim().is_empty() {
                println!("\n{}\n\n{}", bold("Notes"), h.resume_md.trim());
            }
        }
        None => println!("{}", dim("no handoff from this worktree yet")),
    }
    Ok(())
}

fn ls(app: &App, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let handoffs = app.store.handoffs_for(plan.id)?;
    if app.json {
        println!("{}", serde_json::to_string_pretty(&handoffs)?);
        return Ok(());
    }
    if handoffs.is_empty() {
        println!("{}", dim("no handoffs recorded for this plan"));
        return Ok(());
    }
    let here = app.git.as_ref().map(|g| g.worktree_str());
    let mut table = crate::out::Table::new(&["WHEN", "BRANCH", "HEAD", "GATES", "WORKTREE"]);
    for h in &handoffs {
        let red = h.gates.iter().filter(|g| !g.passed()).count();
        let gates = match (h.gates.len(), red) {
            (0, _) => "-".to_string(),
            (n, 0) => format!("{n} green"),
            (_, r) => format!("{r} RED"),
        };
        let wt = if Some(&h.worktree_path) == here.as_ref() {
            format!("{} (here)", short_path(&h.worktree_path))
        } else {
            short_path(&h.worktree_path)
        };
        table.row(vec![
            h.at.split('T').next().unwrap_or(&h.at).to_string(),
            h.branch.clone().unwrap_or_default(),
            h.head_sha.clone().unwrap_or_default(),
            gates,
            wt,
        ]);
    }
    table.print();
    Ok(())
}

pub fn resume(app: &mut App, args: &ResumeArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let worktree = app.git.as_ref().map(|g| g.worktree_str());
    let md = app.store.render_resume(plan.id, worktree.as_deref())?;

    if app.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "plan": plan.slug,
                "resume_md": md,
            }))?
        );
        return Ok(());
    }
    print!("{md}");

    // Picking the work back up is the point, so offer to claim it in one step.
    if args.claim {
        if let Some(next) = app.store.next_slice(plan.id, worktree.as_deref())? {
            if next.claimed_by.is_none() {
                let git = app.require_git()?.clone();
                app.store
                    .claim_slice(&next, &git.worktree_str(), git.branch.as_deref())?;
                println!("\n");
                ok(&format!("claimed {} for this worktree", next.key));
            }
        }
    }
    Ok(())
}

/// One line of context for a session-start hook. Silent when there is nothing useful
/// to say, so it never adds noise to an unrelated session.
pub fn hook(app: &mut App, plan_ref: Option<&str>) -> Result<()> {
    let git = match &app.git {
        Some(g) => g.clone(),
        None => return Ok(()),
    };
    let resolved = app.store.resolve(Some(&git), plan_ref)?;
    let Ok(resolution) = resolved else {
        return Ok(());
    };

    let plan = &resolution.plan;
    let slices = app.store.slices(plan.id)?;
    let done = slices.iter().filter(|s| s.status == Status::Done).count();
    let worktree = git.worktree_str();

    let mut line = format!(
        "Build plan: {} - {} ({}), {done}/{} slices done.",
        plan.slug,
        plan.title,
        plan.status,
        slices.len()
    );
    if let Some(on) = &resolution.slice {
        line.push_str(&format!(" On {} ({}).", on.key, on.status));
    }
    if let Some(next) = app.store.next_slice(plan.id, Some(&worktree))? {
        if Some(next.key.as_str()) != resolution.slice.as_ref().map(|s| s.key.as_str()) {
            line.push_str(&format!(" Next: {} - {}.", next.key, next.title));
        }
    }
    let open = app.store.questions(plan.id, true)?.len();
    if open > 0 {
        line.push_str(&format!(" {open} open question(s)."));
    }
    if app.store.latest_handoff(plan.id, &worktree)?.is_some() {
        line.push_str(" A handoff exists for this worktree - run `aip resume` before starting.");
    }
    line.push_str(" Use `aip status`, `aip show`, `aip log`, `aip slice set`.");

    println!(
        "{}",
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": line,
            }
        })
    );
    Ok(())
}
