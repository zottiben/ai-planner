//! Serving the frontend out of the binary (D2).
//!
//! PR1 ships a placeholder that proves the server and the token work from a browser.
//! PR2 swaps `lookup` for the embedded Vite bundle; everything around it - the SPA
//! fallback, the content types, the cache headers - is already correct and stays.

use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

/// Anything that is not a file is the app's own route, so it gets `index.html` and the
/// client router takes it from there. Without this, reloading on `/plan/kanban-ui`
/// is a 404.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    match lookup(path) {
        Some((body, mime)) => file(body, mime),
        None => match lookup("index.html") {
            Some((body, mime)) => file(body, mime),
            None => (StatusCode::NOT_FOUND, "not found").into_response(),
        },
    }
}

fn lookup(path: &str) -> Option<(&'static [u8], &'static str)> {
    match path {
        "" | "index.html" => Some((PLACEHOLDER.as_bytes(), "text/html; charset=utf-8")),
        _ => None,
    }
}

fn file(body: &'static [u8], mime: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
            // The bundle is rebuilt with the binary and served from memory, so a
            // cached copy from a previous version is exactly what we do not want.
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, must-revalidate"),
            ),
        ],
        body,
    )
        .into_response()
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>ai-planner</title>
<style>
  :root { color-scheme: light dark; --line: color-mix(in srgb, currentColor 15%, transparent); }
  body { font: 15px/1.55 ui-sans-serif, system-ui, -apple-system, sans-serif;
         margin: 0; padding: 3rem 1.5rem; display: flex; justify-content: center; }
  main { width: 100%; max-width: 46rem; }
  h1 { font-size: 1.4rem; margin: 0 0 .25rem; letter-spacing: -0.01em; }
  p.sub { margin: 0 0 2rem; opacity: .65; }
  section { border: 1px solid var(--line); border-radius: 10px; padding: 1rem 1.25rem; margin-bottom: 1rem; }
  h2 { font-size: .75rem; text-transform: uppercase; letter-spacing: .08em; opacity: .6; margin: 0 0 .75rem; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { display: flex; gap: .6rem; align-items: baseline; padding: .3rem 0; border-bottom: 1px solid var(--line); }
  li:last-child { border-bottom: 0; }
  .marker { width: 1rem; text-align: center; opacity: .7; }
  .title { flex: 1; }
  .meta { font-size: .8rem; opacity: .55; font-variant-numeric: tabular-nums; }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .85em; }
</style>
</head>
<body>
<main>
  <h1>ai-planner</h1>
  <p class="sub">The API is up. The board arrives in PR2.</p>
  <section><h2>Database</h2><div id="meta" class="meta">loading…</div></section>
  <section><h2>Plans</h2><ul id="plans"><li class="meta">loading…</li></ul></section>
</main>
<script>
const token = new URLSearchParams(location.search).get('t') ?? '';
const api = (path) => fetch('/api' + path, { headers: { 'x-planner-token': token } })
  .then((r) => r.ok ? r.json() : r.json().then((e) => Promise.reject(new Error(e.error))));

const escape = (s) => String(s ?? '').replace(/[&<>"]/g, (c) =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

Promise.all([api('/meta'), api('/plans')]).then(([meta, plans]) => {
  const marks = Object.fromEntries(meta.statuses.map((s) => [s.value, s.marker]));
  document.getElementById('meta').textContent = meta.database + '  ·  ' + plans.length + ' plans';
  document.getElementById('plans').innerHTML = plans.map((p) => `
    <li><span class="marker">${escape(marks[p.status] ?? '·')}</span>
        <span class="title">${escape(p.title)}</span>
        <span class="meta">${escape(p.repo_name)} · ${p.percent === null ? '—' : p.percent + '%'}</span></li>`
  ).join('') || '<li class="meta">no plans yet</li>';
}).catch((err) => {
  document.getElementById('meta').textContent = err.message;
});
</script>
</body>
</html>
"#;
