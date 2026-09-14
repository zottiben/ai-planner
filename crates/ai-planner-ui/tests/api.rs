//! The API, driven over a real socket.
//!
//! These go through `Server::bind` and a TCP connection rather than calling handlers
//! directly, because the parts that break are the parts a unit test skips: the token
//! middleware, the status codes, the SPA fallback, and whether the JSON is the shape
//! the client was written against.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;

use ai_planner_core::{NewPlan, NewSlice, Status, Store};
use ai_planner_ui::{ServeOptions, Server};

struct Harness {
    addr: SocketAddr,
    token: String,
    /// Kept so a test can open a *second* `Store` on the same file - the only way to
    /// exercise the cross-process liveness path, since `data_version` only moves when
    /// another connection commits.
    db: std::path::PathBuf,
    _runtime: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn start(seed: impl FnOnce(&mut Store)) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("planner.db");
        let mut store = Store::init(&path).unwrap();
        seed(&mut store);
        drop(store);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        let store = Store::open(&path).unwrap();
        let server = runtime
            .block_on(Server::bind(store, ServeOptions::default()))
            .unwrap();
        let addr = server.addr();
        let token = server.token().to_string();
        runtime.spawn(async move {
            let _ = server.serve().await;
        });

        Harness {
            addr,
            token,
            db: path,
            _runtime: runtime,
            _dir: dir,
        }
    }

    fn get(&self, path: &str) -> Response {
        self.request(path, Some(&self.token))
    }

    fn get_anonymous(&self, path: &str) -> Response {
        self.request(path, None)
    }

    fn request(&self, path: &str, token: Option<&str>) -> Response {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        let auth = match token {
            Some(t) => format!("x-planner-token: {t}\r\n"),
            None => String::new(),
        };
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {}\r\n{auth}Connection: close\r\n\r\n",
            self.addr
        )
        .unwrap();

        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").expect("a complete response");
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .expect("a status line");
        Response {
            status,
            head: head.to_lowercase(),
            body: body.to_string(),
        }
    }
}

struct Response {
    status: u16,
    head: String,
    body: String,
}

impl Response {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("expected JSON, got {:?}: {e}", self.body))
    }
}

fn seed(store: &mut Store) -> i64 {
    let repo = store
        .ensure_repo(&ai_planner_core::GitContext {
            repo_key: "example.com/acme/widget".into(),
            repo_name: "widget".into(),
            remote_url: Some("git@example.com:acme/widget.git".into()),
            main_path: Path::new("/tmp/widget").to_path_buf(),
            worktree: Path::new("/tmp/widget").to_path_buf(),
            branch: Some("main".into()),
            head_sha: None,
        })
        .unwrap();

    let plan = store
        .create_plan(NewPlan {
            repo_id: repo.id,
            title: "Ship the widget".into(),
            summary: Some("A widget, shipped.".into()),
            ..Default::default()
        })
        .unwrap();

    for (key, title, status) in [
        ("PR1", "Lay the foundation", Status::Done),
        ("PR2", "Build the thing", Status::Active),
        ("PR3", "Polish it", Status::Ready),
    ] {
        let slice = store
            .add_slice(NewSlice {
                plan_id: plan.id,
                key: key.into(),
                title: title.into(),
                scope_md: Some(format!("What {key} covers.")),
                ..Default::default()
            })
            .unwrap();
        if status != Status::Ready {
            store.set_slice_status(&slice, status, None).unwrap();
        }
    }
    plan.id
}

#[test]
fn the_api_is_shut_without_the_token() {
    let h = Harness::start(|store| {
        seed(store);
    });

    let refused = h.get_anonymous("/api/plans");
    assert_eq!(refused.status, 401);
    assert_eq!(refused.json()["code"], "unauthorized");

    let wrong = h.request("/api/plans", Some("0000000000000000"));
    assert_eq!(wrong.status, 401, "a wrong token is no better than none");

    let allowed = h.get("/api/plans");
    assert_eq!(allowed.status, 200);
}

#[test]
fn the_token_may_also_ride_in_the_query_because_event_source_cannot_set_headers() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let by_query = h.get_anonymous(&format!("/api/plans?t={}", h.token));
    assert_eq!(by_query.status, 200);
    assert_eq!(by_query.json().as_array().unwrap().len(), 1);
}

#[test]
fn plans_carry_the_progress_roll_ups_the_sidebar_needs() {
    let h = Harness::start(|store| {
        seed(store);
    });

    let plans = h.get("/api/plans").json();
    let plan = &plans[0];
    assert_eq!(plan["slug"], "ship-the-widget");
    assert_eq!(plan["repo_name"], "widget");
    assert_eq!(plan["slices"], 3);
    assert_eq!(plan["done"], 1);
    assert_eq!(plan["percent"], 33);
    assert_eq!(plan["open_questions"], 0);
}

#[test]
fn a_plan_with_no_slices_has_no_percentage_rather_than_zero() {
    let h = Harness::start(|store| {
        let repo = store
            .ensure_repo(&ai_planner_core::GitContext {
                repo_key: "example.com/acme/empty".into(),
                repo_name: "empty".into(),
                remote_url: None,
                main_path: Path::new("/tmp/empty").to_path_buf(),
                worktree: Path::new("/tmp/empty").to_path_buf(),
                branch: None,
                head_sha: None,
            })
            .unwrap();
        store
            .create_plan(NewPlan {
                repo_id: repo.id,
                title: "Nothing yet".into(),
                ..Default::default()
            })
            .unwrap();
    });

    let plans = h.get("/api/plans").json();
    assert_eq!(plans[0]["slices"], 0);
    assert!(
        plans[0]["percent"].is_null(),
        "0% means started and got nowhere; null means not broken down yet"
    );
}

#[test]
fn the_board_returns_every_column_even_the_empty_ones() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let plan_id = h.get("/api/plans").json()[0]["id"].as_i64().unwrap();

    let board = h.get(&format!("/api/plans/{plan_id}/board")).json();
    let columns = board["columns"].as_array().unwrap();
    assert_eq!(
        columns.len(),
        Status::ALL.len(),
        "a column that vanishes when it empties cannot be dragged into"
    );

    let by_status = |name: &str| -> Vec<String> {
        columns
            .iter()
            .find(|c| c["status"] == name)
            .map(|c| {
                c["slices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s["key"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap()
    };
    assert_eq!(by_status("done"), ["PR1"]);
    assert_eq!(by_status("active"), ["PR2"]);
    assert_eq!(by_status("ready"), ["PR3"]);
    assert!(by_status("blocked").is_empty());
}

#[test]
fn a_ticket_carries_its_plan_its_repo_and_its_own_log() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let plan_id = h.get("/api/plans").json()[0]["id"].as_i64().unwrap();
    let board = h.get(&format!("/api/plans/{plan_id}/board")).json();
    let active = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["status"] == "active")
        .unwrap()["slices"][0]
        .clone();

    let detail = h
        .get(&format!("/api/slices/{}", active["id"].as_i64().unwrap()))
        .json();
    assert_eq!(detail["slice"]["key"], "PR2");
    assert_eq!(detail["slice"]["scope_md"], "What PR2 covers.");
    assert_eq!(detail["plan_slug"], "ship-the-widget");
    assert_eq!(detail["repo"], "widget");
    assert!(
        !detail["log"].as_array().unwrap().is_empty(),
        "moving PR2 to active appends a status entry, and the drawer shows it"
    );
}

#[test]
fn meta_publishes_the_status_vocabulary_so_the_client_does_not_hardcode_it() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let meta = h.get("/api/meta").json();
    let statuses = meta["statuses"].as_array().unwrap();
    assert_eq!(statuses.len(), Status::ALL.len());
    assert_eq!(statuses[0]["value"], "draft");

    let in_review = statuses.iter().find(|s| s["value"] == "in_review").unwrap();
    assert_eq!(in_review["label"], "In review", "a label is for reading");
    assert_eq!(in_review["terminal"], false);
    assert!(meta["database"].as_str().unwrap().ends_with("planner.db"));
}

#[test]
fn a_missing_plan_is_a_404_not_a_500() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let missing = h.get("/api/plans/9999/board");
    assert_eq!(missing.status, 404);
    assert_eq!(missing.json()["code"], "not_found");
}

#[test]
fn a_client_route_falls_back_to_the_app_so_a_reload_does_not_404() {
    let h = Harness::start(|store| {
        seed(store);
    });

    let index = h.get_anonymous("/");
    assert_eq!(index.status, 200);
    assert!(index.head.contains("text/html"));

    // Deep links are the point of the drawer having a URL at all.
    let deep = h.get_anonymous("/plan/ship-the-widget/slice/PR2");
    assert_eq!(deep.status, 200);
    assert!(deep.head.contains("text/html"));
    assert_eq!(deep.body, index.body);

    // The app is served unauthenticated on purpose - it is a static shell, and it
    // cannot read anything until the token in the URL reaches /api.
    assert!(index.body.contains("<!doctype html>"));
}

#[test]
fn the_bundle_is_never_cached_so_an_upgrade_is_not_a_hard_refresh() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let index = h.get_anonymous("/");
    assert!(
        index.head.contains("no-cache"),
        "the bundle ships with the binary, so a stale copy is a version mismatch"
    );
}

#[test]
fn the_real_frontend_is_compiled_into_the_binary() {
    // The point of D2: no dev server, no node at run time, no `dist` directory to
    // find on disk. If this fails, `aip ui` serves an apology instead of a board.
    let bundle = ai_planner_ui::bundle();
    assert!(
        bundle.embedded,
        "no frontend bundle - run `npm ci && npm run build` in ui/ and rebuild"
    );

    let h = Harness::start(|store| {
        seed(store);
    });

    let index = h.get_anonymous("/");
    assert!(
        index.body.contains("<div id=\"root\">"),
        "that is not the React app"
    );

    let script = h.get_anonymous("/app.js");
    assert_eq!(script.status, 200);
    assert!(
        script.head.contains("text/javascript"),
        "a bundle served as octet-stream will not execute"
    );

    let styles = h.get_anonymous("/index.css");
    assert_eq!(styles.status, 200);
    assert!(styles.head.contains("text/css"));
}

// -- writes ---------------------------------------------------------------------

impl Harness {
    fn post(&self, path: &str, body: serde_json::Value) -> Response {
        self.send("POST", path, body)
    }

    fn patch(&self, path: &str, body: serde_json::Value) -> Response {
        self.send("PATCH", path, body)
    }

    fn send(&self, method: &str, path: &str, body: serde_json::Value) -> Response {
        let payload = body.to_string();
        let mut stream = TcpStream::connect(self.addr).unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nx-planner-token: {}\r\n\
             content-type: application/json\r\ncontent-length: {}\r\n\
             Connection: close\r\n\r\n{payload}",
            self.addr,
            self.token,
            payload.len()
        )
        .unwrap();

        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").expect("a complete response");
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .expect("a status line");
        Response {
            status,
            head: head.to_lowercase(),
            body: body.to_string(),
        }
    }

    /// The id of a slice by key, through the API rather than around it.
    fn slice_id(&self, key: &str) -> i64 {
        let plan_id = self.get("/api/plans").json()[0]["id"].as_i64().unwrap();
        self.get(&format!("/api/plans/{plan_id}/board")).json()["columns"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["slices"].as_array().unwrap().clone())
            .find(|s| s["key"] == key)
            .map(|s| s["id"].as_i64().unwrap())
            .unwrap_or_else(|| panic!("no slice {key}"))
    }

    fn plan_id(&self) -> i64 {
        self.get("/api/plans").json()[0]["id"].as_i64().unwrap()
    }
}

#[test]
fn dragging_a_card_moves_the_slice_and_records_it() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    let moved = h.post(
        &format!("/api/slices/{id}/status"),
        serde_json::json!({"status": "active"}),
    );
    assert_eq!(moved.status, 200);
    assert_eq!(moved.json()["status"], "active");

    // The status change is a log entry in the same transaction, so history survives
    // without anyone remembering to write it.
    let detail = h.get(&format!("/api/slices/{id}")).json();
    assert_eq!(detail["slice"]["status"], "active");
    assert!(detail["log"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["kind"] == "status"));
}

#[test]
fn blocking_without_a_reason_is_refused_rather_than_recorded_blank() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    let bare = h.post(
        &format!("/api/slices/{id}/status"),
        serde_json::json!({"status": "blocked"}),
    );
    assert_eq!(bare.status, 400);
    assert_eq!(bare.json()["code"], "bad_request");

    let with_reason = h.post(
        &format!("/api/slices/{id}/status"),
        serde_json::json!({"status": "blocked", "reason": "waiting on the lender sandbox"}),
    );
    assert_eq!(with_reason.status, 200);
    assert_eq!(
        with_reason.json()["blocked_reason"],
        "waiting on the lender sandbox"
    );
}

#[test]
fn a_claim_another_worktree_holds_is_refused_and_names_the_holder() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    let mine = h.post(
        &format!("/api/slices/{id}/claim"),
        serde_json::json!({"worktree": "/tmp/widget-a"}),
    );
    assert_eq!(mine.status, 200);
    assert_eq!(mine.json()["worktree_path"], "/tmp/widget-a");

    // Same actor, different worktree: a genuine clash, not a re-claim.
    let theirs = h.post(
        &format!("/api/slices/{id}/claim"),
        serde_json::json!({"worktree": "/tmp/widget-b"}),
    );
    assert_eq!(theirs.status, 409);

    let body = theirs.json();
    assert_eq!(body["code"], "already_claimed");
    // The whole point: the board can say who and where, not merely "failed".
    assert_eq!(body["slice"], "PR3");
    assert_eq!(body["worktree"], "/tmp/widget-a");
    assert!(body["holder"].as_str().is_some_and(|h| !h.is_empty()));
}

#[test]
fn releasing_hands_a_slice_back() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    h.post(
        &format!("/api/slices/{id}/claim"),
        serde_json::json!({"worktree": "/tmp/widget-a"}),
    );
    let released = h.post(&format!("/api/slices/{id}/release"), serde_json::json!({}));
    assert_eq!(released.status, 200);
    assert!(released.json()["claimed_by"].is_null());

    let retaken = h.post(
        &format!("/api/slices/{id}/claim"),
        serde_json::json!({"worktree": "/tmp/widget-b"}),
    );
    assert_eq!(retaken.status, 200, "a released slice is anyone's to take");
}

#[test]
fn claiming_fills_the_branch_in_but_never_overwrites_the_planned_one() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    h.patch(
        &format!("/api/slices/{id}"),
        serde_json::json!({"branch": "planned/branch"}),
    );
    let claimed = h.post(
        &format!("/api/slices/{id}/claim"),
        serde_json::json!({"worktree": "/tmp/w", "branch": "whatever-im-on"}),
    );

    assert_eq!(
        claimed.json()["branch"],
        "planned/branch",
        "rule 14 - a slice claimed from the wrong branch must keep pointing at its own work"
    );
}

#[test]
fn a_patch_leaves_the_fields_it_does_not_mention_alone() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    h.patch(
        &format!("/api/slices/{id}"),
        serde_json::json!({"pr_url": "https://example.com/acme/widget/pull/9"}),
    );
    let after = h.patch(
        &format!("/api/slices/{id}"),
        serde_json::json!({"estimate_files": 4}),
    );

    assert_eq!(after.json()["estimate_files"], 4);
    assert_eq!(
        after.json()["pr_url"],
        "https://example.com/acme/widget/pull/9",
        "two people editing different attributes must not clobber each other"
    );
    assert_eq!(after.json()["title"], "Polish it");
    assert_eq!(after.json()["scope_md"], "What PR3 covers.");
}

#[test]
fn a_note_lands_against_its_slice_and_an_empty_one_is_refused() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let plan_id = h.plan_id();

    let blank = h.post(
        &format!("/api/plans/{plan_id}/log"),
        serde_json::json!({"body": "   "}),
    );
    assert_eq!(blank.status, 400);

    let noted = h.post(
        &format!("/api/plans/{plan_id}/log"),
        serde_json::json!({"body": "the sandbox came back", "slice": "PR3"}),
    );
    assert_eq!(noted.status, 200);

    let detail = h.get(&format!("/api/slices/{}", h.slice_id("PR3"))).json();
    assert!(detail["log"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["body"] == "the sandbox came back"));
}

#[test]
fn an_unknown_status_is_a_bad_request_not_a_corrupt_row() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    let nonsense = h.post(
        &format!("/api/slices/{id}/status"),
        serde_json::json!({"status": "nearly"}),
    );
    assert_eq!(nonsense.status, 400);
    assert_eq!(
        h.get(&format!("/api/slices/{id}")).json()["slice"]["status"],
        "ready"
    );
}

#[test]
fn writes_need_the_token_too() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let id = h.slice_id("PR3");

    let payload = serde_json::json!({"status": "done"}).to_string();
    let mut stream = TcpStream::connect(h.addr).unwrap();
    write!(
        stream,
        "POST /api/slices/{id}/status HTTP/1.1\r\nHost: {}\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nConnection: close\r\n\r\n{payload}",
        h.addr,
        payload.len()
    )
    .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();

    assert!(
        raw.starts_with("HTTP/1.1 401"),
        "got: {}",
        &raw[..40.min(raw.len())]
    );
    assert_eq!(
        h.get(&format!("/api/slices/{id}")).json()["slice"]["status"],
        "ready",
        "an unauthenticated write must not land"
    );
}

// -- liveness -------------------------------------------------------------------

impl Harness {
    /// Open the SSE stream and collect whatever arrives within `window`, then give up.
    /// The socket is read with a timeout because the stream never ends on its own.
    fn listen(&self, window: std::time::Duration) -> String {
        let mut stream = TcpStream::connect(self.addr).unwrap();
        write!(
            stream,
            "GET /api/events?t={} HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\n\r\n",
            self.token, self.addr
        )
        .unwrap();
        stream.set_read_timeout(Some(window)).unwrap();

        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break, // the read timed out, which is how this always ends
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    /// Write to the same database from a second `Store`, the way an agent or the CLI
    /// would - a different connection, which is the only case `data_version` reports.
    fn write_from_another_process(&self, f: impl FnOnce(&mut Store)) {
        let mut store = Store::open(&self.db).unwrap();
        f(&mut store);
    }
}

#[test]
fn a_write_from_another_connection_reaches_the_stream() {
    let h = Harness::start(|store| {
        seed(store);
    });

    let listener = {
        let addr = h.addr;
        let token = h.token.clone();
        std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            write!(
                stream,
                "GET /api/events?t={token} HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n\r\n"
            )
            .unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(4)))
                .unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while let Ok(n) = stream.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if String::from_utf8_lossy(&buf).contains("event: changed") {
                    break;
                }
            }
            String::from_utf8_lossy(&buf).to_string()
        })
    };

    // Give the subscription time to land before making the change it should report.
    std::thread::sleep(std::time::Duration::from_millis(600));
    h.write_from_another_process(|store| {
        let plan = store.list_plans(&Default::default()).unwrap().remove(0);
        let slice = store.require_slice(plan.id, "PR3").unwrap();
        store
            .set_slice_status(&slice, Status::Active, None)
            .unwrap();
    });

    let received = listener.join().unwrap();
    assert!(
        received.contains("event: changed"),
        "an agent writing from another process must reach the board; got: {received:?}"
    );
    assert!(received.to_lowercase().contains("text/event-stream"));
}

#[test]
fn the_stream_stays_quiet_when_nothing_changes() {
    let h = Harness::start(|store| {
        seed(store);
    });

    // Long enough for several watcher ticks. A stream that announces on a timer rather
    // than on a change is a stream that refetches the board forever.
    let received = h.listen(std::time::Duration::from_millis(1600));
    assert!(
        !received.contains("event: changed"),
        "nothing was written, so nothing should have been announced; got: {received:?}"
    );
}

#[test]
fn the_event_stream_needs_the_token_like_everything_else() {
    let h = Harness::start(|store| {
        seed(store);
    });
    let refused = h.get_anonymous("/api/events");
    assert_eq!(refused.status, 401);
}
