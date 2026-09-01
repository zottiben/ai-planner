use ai_planner_core::embed::{self, Embedder};
use ai_planner_core::{SearchOptions, Status};
use anyhow::Result;

use crate::app::App;
use crate::cli::{EmbedArgs, FindArgs};
use crate::out::{bold, dim, ok, status_colour, truncate, Table};

pub fn find(app: &mut App, args: &FindArgs) -> Result<()> {
    let query = args.query.join(" ");
    if query.trim().is_empty() {
        anyhow::bail!("nothing to search for");
    }

    // The lexical index is derived, so it is rebuilt on demand rather than maintained
    // on every write. At this scale that costs milliseconds and can never be stale.
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

    // Load the model only when there is a semantic index to use it against, so a
    // lexical-only setup never pays the model's start-up cost.
    let embedder: Option<Box<dyn Embedder>> = if args.lexical || !embed::available() {
        None
    } else {
        match app.store.embedding_state()? {
            Some((model, _)) => embed::build(&model, None).ok(),
            None => None,
        }
    };

    let hits = app.store.search_with(
        &query,
        &SearchOptions {
            prefer_repo: app.repo_id(),
            only_repo: if args.all { None } else { app.repo_id() },
            statuses,
            limit: args.limit,
            lexical_only: args.lexical,
        },
        embedder.as_deref(),
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

    let semantic = embedder.is_some();
    let mut table = if semantic {
        Table::new(&["PLAN", "STATUS", "WHERE", "VIA", "MATCH"])
    } else {
        Table::new(&["PLAN", "STATUS", "WHERE", "MATCH"])
    };
    for hit in &hits {
        let reference = match hit.kind.as_str() {
            "plan" => "title".to_string(),
            _ => format!("{} {}", hit.kind, hit.reference),
        };
        let mut row = vec![
            bold(&hit.plan.slug),
            status_colour(hit.plan.status, hit.plan.status.as_str()),
            dim(&truncate(&reference, 28)),
        ];
        if semantic {
            row.push(dim(hit.matched));
        }
        row.push(truncate(&hit.snippet, 56));
        table.row(row);
    }
    table.print();
    Ok(())
}

pub fn embed_cmd(app: &mut App, args: &EmbedArgs) -> Result<()> {
    if args.clear {
        let n = app.store.clear_embeddings()?;
        ok(&format!("removed {n} vectors - search is lexical again"));
        return Ok(());
    }

    if args.status {
        match app.store.embedding_state()? {
            Some((model, rows)) => {
                println!("{model}");
                println!("{}", dim(&format!("{rows} vectors")));
                if !embed::available() {
                    println!(
                        "{}",
                        dim("this build cannot use them - reinstall with --features model-embeddings")
                    );
                }
            }
            None => println!("{}", dim("no semantic index - run `aip embed`")),
        }
        return Ok(());
    }

    if !embed::available() {
        anyhow::bail!(
            "this build has no embedding model.\n  Reinstall with:\n    cargo install --git \
             https://github.com/zottiben/ai-planner ai-planner --locked --features model-embeddings"
        );
    }

    // Refuse to embed a stale picture: the lexical index is what defines the units.
    app.store.reindex()?;

    if args.model_dir.is_none() {
        println!(
            "{}",
            dim("loading the model (first run downloads it once into ~/.cache/ai-planner)…")
        );
    }
    let embedder = embed::build(&args.model, args.model_dir.as_deref())?;

    if args.force {
        app.store.clear_embeddings()?;
    }
    let stats = app.store.embed_all(embedder.as_ref())?;

    if app.json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    ok(&format!(
        "{} ({} dims): {} embedded, {} unchanged, {} removed",
        embedder.id(),
        embedder.dims(),
        stats.embedded,
        stats.unchanged,
        stats.removed
    ));
    println!(
        "{}",
        dim("`aip find` now fuses meaning with words; `--lexical` turns it off for one query")
    );
    Ok(())
}
