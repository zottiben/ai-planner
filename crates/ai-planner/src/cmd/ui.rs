//! `aip ui` - the board.
//!
//! One process and one binary: the frontend is compiled in, so there is no dev server
//! to start and nothing to install (D2). The runtime lives here rather than around
//! `main` for the same reason `serve` does - nothing else in the tool is async.

use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use ai_planner_ui::{ServeOptions, Server};

use crate::app::App;
use crate::cli::UiArgs;

pub fn ui(mut app: App, args: &UiArgs) -> Result<()> {
    // The board writes as whoever is sitting in front of it. `aip ui` is a human at a
    // keyboard, so the log should say so rather than blaming an agent.
    app.store.set_actor(
        args.actor
            .clone()
            .unwrap_or_else(|| format!("{}@ui", ai_planner_core::default_actor())),
    );

    let json = app.json;
    let options = ServeOptions {
        port: args.port,
        token: args.token.clone(),
    };

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?
        .block_on(async move {
            let server = Server::bind(app.store, options)
                .await
                .context("starting the board")?;
            let url = server.url();

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "url": url,
                        "addr": server.addr().to_string(),
                        "token": server.token(),
                    })
                );
            } else {
                println!("board  {url}");
                println!("       ctrl-c to stop");
            }

            if !args.no_open {
                // Opening is a convenience, never a reason to fail: a headless box or
                // a locked-down desktop still has a perfectly good URL printed above.
                if let Err(err) = open_browser(&url) {
                    eprintln!("note: could not open a browser ({err}) - open the URL above");
                }
            }

            server.serve().await.context("serving the board")
        })
}

fn open_browser(url: &str) -> Result<()> {
    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        // `start` is a cmd builtin, not a program, and the empty string is the window
        // title - without it cmd treats a quoted URL as the title and opens nothing.
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };

    Command::new(program)
        .args(args)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("running {program}"))?;
    Ok(())
}
