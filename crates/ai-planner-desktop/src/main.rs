//! The board, as a desktop app (D3).
//!
//! Tauri rather than Electron, for a reason that is structural rather than aesthetic:
//! the host process here *is* Rust, so it links `ai-planner-core` and `ai-planner-ui`
//! directly and there is no IPC boundary, no second runtime, and no second copy of the
//! rules in another language. It also uses the platform webview instead of shipping
//! Chromium, which is the difference between an app measured in tens of megabytes and
//! one measured in hundreds.
//!
//! The window is deliberately thin. It starts the same axum server `aip ui` starts and
//! points the webview at it, so there is one frontend, one API and one set of tests -
//! the desktop app cannot drift from the browser app because it *is* the browser app.

// No console window behind the app on Windows. Only in release: a debug build's
// stdout is how you find out why it did not start.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::mpsc;

use anyhow::{Context, Result};
use tauri::{WebviewUrl, WebviewWindowBuilder};

use ai_planner_core::{default_db_path, Store};
use ai_planner_ui::{ServeOptions, Server};

fn main() {
    if let Err(err) = run() {
        // A desktop app has nowhere to print, so a startup failure gets a dialog. The
        // commonest one by far is "no database yet", which has a one-line fix.
        eprintln!("ai-planner: {err:#}");
        rfd_message(&format!("{err:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let path = default_db_path();
    let store = Store::open(&path).with_context(|| {
        format!(
            "opening {} - run `aip init` in a repo first",
            path.display()
        )
    })?;

    // The server needs a runtime that outlives this function, and Tauri owns the main
    // thread for the event loop, so the runtime is leaked deliberately rather than
    // dropped at the end of `run` and taking the server with it.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let runtime = Box::leak(Box::new(runtime));

    let (ready, started) = mpsc::channel();
    runtime.spawn(async move {
        match Server::bind(store, ServeOptions::default()).await {
            Ok(server) => {
                let _ = ready.send(Ok(server.url()));
                let _ = server.serve().await;
            }
            Err(err) => {
                let _ = ready.send(Err(err.to_string()));
            }
        }
    });

    let url = started
        .recv()
        .context("the board server did not start")?
        .map_err(|e| anyhow::anyhow!(e))?;
    eprintln!("ai-planner: serving {url}");
    let url = url.parse().context("the board produced an unusable URL")?;

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(move |app| {
            // Built here rather than in tauri.conf.json because the URL is not known
            // until the OS has assigned a port and the token has been minted.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("ai-planner")
                .inner_size(1280.0, 820.0)
                .min_inner_size(820.0, 480.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .context("running the desktop app")
}

/// Tauri's dialog plugin is not loaded yet when startup fails, so this uses the
/// platform's own facility and falls back to stderr where there is not one.
fn rfd_message(message: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display dialog {} with title \"ai-planner\" buttons {{\"OK\"}} with icon caution",
            applescript_string(message)
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .status();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = message;
    }
}

#[cfg(target_os = "macos")]
fn applescript_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
