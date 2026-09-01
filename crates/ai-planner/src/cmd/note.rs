use ai_planner_core::{DecisionStatus, LogKind, NewDecision, NewLog};
use anyhow::Result;

use crate::app::App;
use crate::cli::*;
use crate::out::{dim, ok, Table};
use crate::read_body;

pub fn log(app: &mut App, args: &LogArgs, plan_ref: Option<&str>) -> Result<()> {
    let body = if args.body.is_empty() {
        read_body(None, None)?
    } else {
        args.body.join(" ")
    };
    let plan = app.plan(plan_ref)?;
    let slice_id = match &args.slice {
        Some(key) => Some(app.store.require_slice(plan.id, key)?.id),
        None => None,
    };
    let git = app.git.clone();

    app.store.append_log(NewLog {
        plan_id: plan.id,
        slice_id,
        kind: Some(LogKind::parse(&args.kind)?),
        body,
        branch: git.as_ref().and_then(|g| g.branch.clone()),
        worktree_path: git.as_ref().map(|g| g.worktree_str()),
        at: None,
    })?;

    ok(&format!("logged against {}", plan.slug));
    Ok(())
}

pub fn logs(app: &App, args: &LogsArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let entries = match &args.slice {
        Some(key) => {
            let slice = app.store.require_slice(plan.id, key)?;
            app.store.slice_log(slice.id, Some(args.limit))?
        }
        None => app.store.log(plan.id, Some(args.limit))?,
    };

    if app.json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    if entries.is_empty() {
        println!("{}", dim("nothing logged yet"));
        return Ok(());
    }
    for e in entries {
        let date = e.at.split('T').next().unwrap_or(&e.at);
        let who = e.actor.as_deref().unwrap_or("-");
        let slice = e
            .slice_key
            .as_deref()
            .map(|k| format!("{k} "))
            .unwrap_or_default();
        println!(
            "{} {}",
            dim(&format!("{date} {who:<10} {:<12}", e.kind.as_str())),
            format_args!("{slice}{}", e.body.replace('\n', "\n  "))
        );
    }
    Ok(())
}

pub fn decision(app: &mut App, cmd: &DecisionCmd, plan_ref: Option<&str>) -> Result<()> {
    match cmd {
        DecisionCmd::Add(args) => {
            let plan = app.plan(plan_ref)?;
            let body = match (&args.body, &args.file) {
                (None, None) => String::new(),
                _ => read_body(args.body.as_deref(), args.file.as_deref())?,
            };
            let d = app.store.add_decision(NewDecision {
                plan_id: plan.id,
                key: args.key.clone(),
                title: args.title.clone(),
                body,
                status: Some(DecisionStatus::parse(&args.status)?),
                ord: None,
            })?;
            if app.json {
                println!("{}", serde_json::to_string_pretty(&d)?);
            } else {
                ok(&format!("{} · {} - {}", plan.slug, d.key, d.title));
            }
            Ok(())
        }
        DecisionCmd::Ls => {
            let plan = app.plan(plan_ref)?;
            let decisions = app.store.decisions(plan.id)?;
            if app.json {
                println!("{}", serde_json::to_string_pretty(&decisions)?);
                return Ok(());
            }
            if decisions.is_empty() {
                println!("{}", dim("no decisions recorded"));
                return Ok(());
            }
            let mut table = Table::new(&["KEY", "STATUS", "TITLE"]);
            for d in &decisions {
                table.row(vec![d.key.clone(), d.status.to_string(), d.title.clone()]);
            }
            table.print();
            Ok(())
        }
        DecisionCmd::Supersede(args) => {
            let plan = app.plan(plan_ref)?;
            let d =
                app.store
                    .supersede_decision(plan.id, &args.key, &args.by, args.note.as_deref())?;
            ok(&format!("{} superseded by {}", d.key, args.by));
            Ok(())
        }
    }
}

pub fn question(app: &mut App, cmd: &QuestionCmd, plan_ref: Option<&str>) -> Result<()> {
    match cmd {
        QuestionCmd::Add(args) => {
            let plan = app.plan(plan_ref)?;
            let slice_id = match &args.slice {
                Some(key) => Some(app.store.require_slice(plan.id, key)?.id),
                None => None,
            };
            let id = app.store.add_question(plan.id, slice_id, &args.body)?;
            ok(&format!("question {id} raised on {}", plan.slug));
            Ok(())
        }
        QuestionCmd::Ls { all } => {
            let plan = app.plan(plan_ref)?;
            let questions = app.store.questions(plan.id, !all)?;
            if app.json {
                println!("{}", serde_json::to_string_pretty(&questions)?);
                return Ok(());
            }
            if questions.is_empty() {
                println!("{}", dim("no open questions"));
                return Ok(());
            }
            for q in questions {
                let mark = if q.status == "open" { ' ' } else { 'x' };
                let scope = q
                    .slice_key
                    .as_deref()
                    .map(|k| format!("({k}) "))
                    .unwrap_or_default();
                println!("[{mark}] {:<4} {scope}{}", q.id, q.body);
                if let Some(a) = q.answer {
                    println!("       {}", dim(&format!("answer: {a}")));
                }
            }
            Ok(())
        }
        QuestionCmd::Answer { id, answer } => {
            app.store.answer_question(*id, answer)?;
            ok(&format!("question {id} answered"));
            Ok(())
        }
    }
}

pub fn gotcha(app: &mut App, cmd: &GotchaCmd, plan_ref: Option<&str>) -> Result<()> {
    match cmd {
        GotchaCmd::Add(args) => {
            let plan = app.plan(plan_ref)?;
            let body = match (&args.body, &args.file) {
                (None, None) => String::new(),
                _ => read_body(args.body.as_deref(), args.file.as_deref())?,
            };
            app.store.add_gotcha(plan.id, &args.title, &body)?;
            ok(&format!("{} · {}", plan.slug, args.title));
            Ok(())
        }
        GotchaCmd::Ls => {
            let plan = app.plan(plan_ref)?;
            let gotchas = app.store.gotchas(plan.id)?;
            if app.json {
                println!("{}", serde_json::to_string_pretty(&gotchas)?);
                return Ok(());
            }
            if gotchas.is_empty() {
                println!("{}", dim("no gotchas recorded"));
                return Ok(());
            }
            for g in gotchas {
                println!("## {}\n", g.title);
                if !g.body.trim().is_empty() {
                    println!("{}\n", g.body.trim_end());
                }
            }
            Ok(())
        }
    }
}
