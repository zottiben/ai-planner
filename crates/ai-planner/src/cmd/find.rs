use ai_planner_core::{SearchOptions, Status};
use anyhow::Result;

use crate::app::App;
use crate::cli::FindArgs;
use crate::out::{bold, dim, status_colour, Table};

pub fn find(app: &mut App, args: &FindArgs) -> Result<()> {
    let query = args.query.join(" ");
    if query.trim().is_empty() {
        anyhow::bail!("nothing to search for");
    }

    // The index is derived, so it is rebuilt on demand rather than maintained on every
    // write. At this scale that costs single-digit milliseconds and can never be stale.
    if args.reindex || app.store.search_rows()? == 0 {
        app.store.reindex()?;
    }

    let mut statuses = Vec::new();
    for s in &args.status {
        statuses.push(Status::parse(s)?);
    }
    if args.incomplete {
        statuses.extend_from_slice(&Status::INCOMPLETE);
    }

    let hits = app.store.search(
        &query,
        &SearchOptions {
            prefer_repo: app.repo_id(),
            only_repo: if args.all { None } else { app.repo_id() },
            statuses,
            limit: args.limit,
        },
    )?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
        return Ok(());
    }
    if hits.is_empty() {
        println!("{}", dim(&format!("nothing matches {query:?}")));
        if !args.all {
            println!("{}", dim("try --all to search every repo"));
        }
        return Ok(());
    }

    let mut table = Table::new(&["PLAN", "STATUS", "WHERE", "MATCH"]);
    for hit in &hits {
        let reference = match hit.kind.as_str() {
            "plan" => "title".to_string(),
            "log" => format!("log {}", hit.reference),
            _ => format!("{} {}", hit.kind, hit.reference),
        };
        table.row(vec![
            bold(&hit.plan.slug),
            status_colour(hit.plan.status, hit.plan.status.as_str()),
            dim(&crate::out::truncate(&reference, 28)),
            crate::out::truncate(&hit.snippet, 60),
        ]);
    }
    table.print();
    Ok(())
}
