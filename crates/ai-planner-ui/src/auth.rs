//! The per-run token (D5).
//!
//! Binding to loopback is not an access control. Any page in any tab can POST to
//! `http://127.0.0.1:<port>` - it cannot *read* the reply without CORS, but a write is
//! a write, and this API can move slices and steal claims. So every request carries a
//! secret that only the process that printed the URL knows.

use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::{Error, Result};
use crate::state::AppState;

pub const TOKEN_HEADER: &str = "x-planner-token";
pub const TOKEN_QUERY: &str = "t";

/// 128 bits from the OS. Minted per run and never written to disk: a board that
/// outlives its process is a board someone forgot was listening.
pub fn mint_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| {
        Error::Internal(ai_planner_core::Error::invalid(format!(
            "no system randomness for the session token: {e}"
        )))
    })?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response> {
    if presented(&request).is_some_and(|t| constant_time_eq(&t, state.token())) {
        return Ok(next.run(request).await);
    }
    Err(Error::Unauthorized)
}

/// Header first, then the query string. `EventSource` cannot set headers, so the SSE
/// stream in PR5 has no way to authenticate other than the query - which is why the
/// query form exists at all, rather than being a convenience.
fn presented(request: &Request) -> Option<String> {
    if let Some(value) = request.headers().get(TOKEN_HEADER) {
        return value.to_str().ok().map(str::to_string);
    }
    if let Some(value) = request.headers().get(AUTHORIZATION) {
        if let Some(bearer) = value.to_str().ok()?.strip_prefix("Bearer ") {
            return Some(bearer.to_string());
        }
    }
    let query = request.uri().query()?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == TOKEN_QUERY).then(|| value.to_string())
    })
}

/// Compares every byte regardless of where the first mismatch is. The exposure here is
/// small, but a token check that returns early is a habit worth not having.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_token_is_hex_and_not_the_same_twice() {
        let a = mint_token().unwrap();
        let b = mint_token().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "a token that repeats is not a token");
    }

    #[test]
    fn comparison_rejects_prefixes_and_lengths() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(!constant_time_eq("", "a"));
    }
}
