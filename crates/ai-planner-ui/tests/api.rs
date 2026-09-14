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
