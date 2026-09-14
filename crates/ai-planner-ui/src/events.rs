//! Telling the browser that somebody else wrote (D4).
//!
//! The writers are other processes - four agents, the MCP server, the CLI - which rules
//! out rusqlite's `update_hook`: that only fires for writes on its own connection, and
//! it is the trap someone will otherwise fall into.
//!
//! `PRAGMA data_version` is the primitive that does work across processes. SQLite bumps
//! it on a connection whenever *another* connection has committed, so one integer read
//! on a short interval answers "has anything changed" exactly, without depending on
//! anyone having written an `updated_at` - which matters, because the log is
//! append-only and some writes touch no timestamp at all.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::state::AppState;

/// How often the watcher asks. Short enough that a board feels live, long enough that
/// an idle laptop is doing nothing measurable - a pragma read is a few microseconds.
const TICK: Duration = Duration::from_millis(400);

pub fn routes() -> Router<AppState> {
    Router::new().route("/events", get(events))
}

/// Polls `data_version` and announces every change to whoever is listening. One task
/// per server, not one per client: ten open tabs must not mean ten pollers.
pub fn watch(state: AppState, tx: broadcast::Sender<u64>) {
    tokio::spawn(async move {
        let mut last = state.data_version().unwrap_or(0);
        let mut generation = 0u64;
        loop {
            tokio::time::sleep(TICK).await;
            let Ok(current) = state.data_version() else {
                // A transient read failure is not a reason to stop watching for the
                // rest of the process's life.
                continue;
            };
            if current != last {
                last = current;
                generation += 1;
                // Err means nobody is listening, which is the normal state when no
                // browser is open. Not a failure.
                let _ = tx.send(generation);
            }
        }
    });
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.subscribe()).filter_map(|message| match message {
        Ok(generation) => Some(Ok(Event::default()
            .event("changed")
            .data(generation.to_string()))),
        // Lagged: this client fell behind the buffer. The event carries no payload
        // beyond "something changed", so a missed one and a delivered one mean the
        // same thing - the client refetches either way.
        Err(_) => Some(Ok(Event::default().event("changed").data("lagged"))),
    });

    Sse::new(stream).keep_alive(
        // Without this a proxy, or a laptop that slept, drops a silent connection and
        // the board looks live while being dead. EventSource reconnects on its own once
        // the socket actually closes.
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
