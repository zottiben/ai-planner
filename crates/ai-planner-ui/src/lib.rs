//! A local board over the planner database.
//!
//! The server is Rust on `ai-planner-core` rather than a second service in another
//! language (D1): the store already owns the `IMMEDIATE` write transaction, the SQL
//! claim guard and the append-only log contract, and a board that reached around them
//! could corrupt work the CLI and the MCP server are careful to protect. Every handler
//! here calls `Store`; none of them writes SQL.

mod assets;
mod auth;
mod error;
mod read;
mod state;

use std::net::{Ipv4Addr, SocketAddr};

use axum::Router;
use tokio::net::TcpListener;

use ai_planner_core::Store;

pub use auth::{TOKEN_HEADER, TOKEN_QUERY};
pub use error::{Error, Result};
pub use state::AppState;

#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    /// 0 asks the OS for a free one, so two boards never fight over a number (D5).
    pub port: u16,
    /// Supply one to keep a URL stable across restarts. Otherwise it is minted fresh.
    pub token: Option<String>,
}

/// Bound, but not yet serving. Split in two so `aip ui` can print and open the real
/// URL - which it cannot know until the OS has assigned the port - before it blocks.
pub struct Server {
    addr: SocketAddr,
    token: String,
    listener: TcpListener,
    router: Router,
}

impl Server {
    pub async fn bind(store: Store, options: ServeOptions) -> Result<Server> {
        let token = match options.token {
            Some(t) if !t.trim().is_empty() => t,
            _ => auth::mint_token()?,
        };
        let state = AppState::new(store, token.as_str());

        let api = read::routes().route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

        let router = Router::new()
            .nest("/api", api)
            .fallback(assets::serve)
            .with_state(state);

        // Loopback only, and deliberately not configurable. This API can move any
        // slice in any repo on the machine; it is not something to expose by flag.
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, options.port)))
            .await
            .map_err(|e| {
                Error::Io(std::io::Error::new(
                    e.kind(),
                    format!("binding 127.0.0.1:{}: {e}", options.port),
                ))
            })?;
        let addr = listener.local_addr()?;

        Ok(Server {
            addr,
            token,
            listener,
            router,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// The URL to open. The token rides in the query because it is the only channel a
    /// freshly opened browser tab has.
    pub fn url(&self) -> String {
        format!("http://{}/?{}={}", self.addr, TOKEN_QUERY, self.token)
    }

    pub async fn serve(self) -> Result<()> {
        axum::serve(self.listener, self.router).await?;
        Ok(())
    }
}
