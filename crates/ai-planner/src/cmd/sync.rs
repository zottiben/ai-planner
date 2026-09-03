use ai_planner_core::git;
use anyhow::Result;

use crate::app::App;
use crate::cli::SyncArgs;
use crate::out::{bold, dim, ok};

pub fn sync(app: &mut App, args: &SyncArgs, plan_ref: Option<&str>) -> Result<()> {
    let plan = app.plan(plan_ref)?;
    let ctx = app.require_git()?.clone();
    let use_gh = !args.no_gh && git::gh_available();

    let findings = app.store.drift(plan.id, &ctx, use_gh)?;

    if app.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "plan": plan.slug,
                "used_gh": use_gh,
                "findings": findings,
            }))?
        );
        if args.fix && !findings.is_empty() {
            app.store.apply_drift(plan.id, &findings)?;
        }
        return Ok(());
    }

    if findings.is_empty() {
        ok(&format!("{} matches git", plan.slug));
        if !use_gh {
            println!(
                "{}",
                dim("  gh is unavailable, so PR state was not checked - only branch history")
            );
        }
        return Ok(());
    }

    println!(
        "{} {}",
        bold(&plan.slug),
        dim(&format!("{} thing(s) out of sync", findings.len()))
    );
    for finding in &findings {
        println!("  {}", finding.describe());
        if !args.fix {
            println!("    {}", dim(&finding.remedy()));
        }
    }

    if !args.fix {
        println!();
        println!("{}", dim("re-run with --fix to apply these"));
        return Ok(());
    }

    let report = app.store.apply_drift(plan.id, &findings)?;
    println!();
    ok(&format!("applied {}", report.applied));
    if report.skipped > 0 {
        println!("{}", dim(&format!("  skipped {}", report.skipped)));
    }
    Ok(())
}
