//! Drives the MCP server the way a harness does - one JSON-RPC message per line over
//! stdio - so the tool surface is proved end to end rather than assumed.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Server {
    fn start(db: &Path, root: &Path) -> Server {
        let mut child = Command::new(bin())
            .args(["serve", "--actor", "test-agent"])
            .arg("--root")
            .arg(root)
            .env("AI_PLANNER_DB", db)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("aip serve starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut server = Server {
            child,
            stdin,
            stdout,
            next_id: 0,
        };

        let init = server.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }),
        );
        assert!(
            init["result"]["serverInfo"]["name"] == "ai-planner",
            "{init}"
        );
        server.notify("notifications/initialized");
        server
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.next_id += 1;
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params,
        });
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();

        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("server responds");
            assert!(n > 0, "server closed the connection during {method}");
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            // Skip anything that is not the reply to this request.
            if value.get("id").and_then(|i| i.as_i64()) == Some(self.next_id) {
                return value;
            }
        }
    }

    fn notify(&mut self, method: &str) {
        let msg = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": {} });
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Call a tool and return its parsed JSON payload.
    fn call(&mut self, name: &str, args: serde_json::Value) -> serde_json::Value {
        let reply = self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": args }),
        );
        assert!(reply.get("error").is_none(), "{name} failed: {reply}");
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }

    /// Call a tool expecting it to be refused.
    fn call_err(&mut self, name: &str, args: serde_json::Value) -> String {
        let reply = self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": args }),
        );
        if let Some(err) = reply.get("error") {
            return err.to_string();
        }
        // rmcp reports tool failures as a result with isError set.
        assert_eq!(
            reply["result"]["isError"], true,
            "{name} was expected to fail: {reply}"
        );
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bin() -> PathBuf {
    // The integration test binary lives next to the one under test.
    let mut path = std::env::current_exe().expect("test exe path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("aip")
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
}

struct Fixture {
    _dir: tempfile::TempDir,
    db: PathBuf,
    repo: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("widget");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("f.txt"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    git(
        &repo,
        &["remote", "add", "origin", "git@github.com:acme/widget.git"],
    );

    let db = dir.path().join("planner.db");
    let out = Command::new(bin())
        .args(["init"])
        .env("AI_PLANNER_DB", &db)
        .current_dir(&repo)
        .output()
        .expect("aip init runs");
    assert!(out.status.success(), "aip init: {out:?}");

    Fixture {
        _dir: dir,
        db,
        repo,
    }
}

#[test]
fn the_server_exposes_the_whole_workflow_over_stdio() {
    let fx = fixture();
    let mut s = Server::start(&fx.db, &fx.repo);

    let tools = s.request("tools/list", serde_json::json!({}));
    let names: Vec<String> = tools["result"]["tools"]
        .as_array()
        .expect("a tool list")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_string())
        .collect();
    for expected in [
        "locate",
        "get_plan",
        "get_resume",
        "search_plans",
        "list_plans",
        "list_slices",
        "get_slice",
        "claim_slice",
        "release_slice",
        "set_slice_status",
        "update_slice",
        "add_slice",
        "append_log",
        "add_decision",
        "supersede_decision",
        "add_gotcha",
        "open_question",
        "list_questions",
        "answer_question",
        "update_section",
        "create_plan",
        "write_handoff",
        "import_markdown",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "missing tool {expected}"
        );
    }

    // Nothing exists yet, so locating must say so rather than invent a plan.
    let nothing = s.call("locate", serde_json::json!({}));
    assert_eq!(nothing["resolved"], false, "{nothing}");

    let plan = s.call(
        "create_plan",
        serde_json::json!({
            "title": "ACME-1234 - Reusable Date Range Picker",
            "summary": "Two variants over one shared core."
        }),
    );
    assert_eq!(plan["slug"], "acme-1234");

    s.call(
        "add_slice",
        serde_json::json!({
            "key": "PR1", "title": "Shared core",
            "scope": "The core and the fields variant.",
            "demo": "Pick Last quarter on Summary Panel.",
            "estimate_files": 40
        }),
    );
    s.call(
        "add_slice",
        serde_json::json!({ "key": "PR2", "title": "button variant" }),
    );

    // Creating a plan here taught the association, so it resolves by affinity rather
    // than by falling through to "the only plan in the repo".
    let here = s.call("locate", serde_json::json!({}));
    assert_eq!(here["plan"], "acme-1234");
    assert_eq!(here["total"], 2);
    assert_eq!(here["next_slice"], "PR1");
    assert_eq!(here["rule"], "affinity");
    assert!(here["why"].as_str().unwrap().len() > 5, "{here}");
    assert_eq!(here["has_handoff"], false);

    let claimed = s.call("claim_slice", serde_json::json!({ "key": "PR1" }));
    assert_eq!(claimed["status"], "active");
    assert_eq!(claimed["claimed_by"], "test-agent");

    s.call(
        "append_log",
        serde_json::json!({ "body": "Core landed, gates green.", "slice": "PR1" }),
    );
    s.call(
        "add_decision",
        serde_json::json!({
            "title": "The value is a specification",
            "body": "Not two resolved dates."
        }),
    );
    s.call(
        "add_gotcha",
        serde_json::json!({
            "title": "The Herd symlink is shared",
            "body": "Repoint it, then put it back."
        }),
    );
    let question = s.call(
        "open_question",
        serde_json::json!({ "body": "past or both on Summary Panel?", "slice": "PR1" }),
    );
    s.call(
        "set_slice_status",
        serde_json::json!({ "key": "PR1", "status": "in_review" }),
    );

    // The rendered plan is the same document the markdown file used to be.
    let md = s.call("get_plan", serde_json::json!({}));
    let md = md.as_str().expect("markdown");
    assert!(md.contains("# ACME-1234 - Reusable Date Range Picker"));
    assert!(md.contains("### PR1 - Shared core"));
    assert!(md.contains("**Demo:** Pick Last quarter on Summary Panel."));
    assert!(md.contains("### D1 - The value is a specification"));
    assert!(md.contains("past or both on Summary Panel?"));
    assert!(md.contains("Core landed, gates green."));

    let hits = s.call(
        "search_plans",
        serde_json::json!({ "query": "herd symlink" }),
    );
    let hits = hits.as_array().expect("hits");
    assert!(hits.iter().any(|h| h["kind"] == "gotcha"));

    s.call(
        "answer_question",
        serde_json::json!({ "id": question["id"], "answer": "past" }),
    );
    let questions = s.call("list_questions", serde_json::json!({}));
    assert_eq!(questions[0]["status"], "answered");

    let handoff = s.call(
        "write_handoff",
        serde_json::json!({
            "gates": ["typecheck=pass", "test=pass:731 tests", "lint=fail"],
            "notes": "PR2 has not been started."
        }),
    );
    assert_eq!(handoff["gates"][2]["name"], "lint");
    assert_eq!(handoff["gates"][2]["result"], "fail");

    let resume = s.call("get_resume", serde_json::json!({}));
    let resume = resume.as_str().expect("markdown");
    // A failed gate must be reported as red, not folded into a green checkpoint.
    assert!(resume.contains("Gates: RED"), "{resume}");
    assert!(resume.contains("PR2 has not been started."));
    assert!(resume.contains("The Herd symlink is shared"));

    let after = s.call("locate", serde_json::json!({}));
    assert_eq!(after["has_handoff"], true);
    assert_eq!(after["slice"], "PR1");
}

#[test]
fn a_slice_another_worktree_holds_is_refused_not_stolen() {
    let fx = fixture();
    let mut first = Server::start(&fx.db, &fx.repo);
    first.call(
        "create_plan",
        serde_json::json!({ "title": "Canvas Editor" }),
    );
    first.call(
        "add_slice",
        serde_json::json!({ "key": "S1", "title": "Canvas" }),
    );
    first.call("claim_slice", serde_json::json!({ "key": "S1" }));

    // A second worktree of the same repo, sharing the one database.
    let other = fx._dir.path().join("wt2");
    git(
        &fx.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/x",
            other.to_str().unwrap(),
        ],
    );
    let mut second = Server::start(&fx.db, &other);

    let err = second.call_err("claim_slice", serde_json::json!({ "key": "S1" }));
    assert!(err.contains("already claimed"), "{err}");

    // The second worktree still sees the plan, and sees who holds the slice.
    let slices = second.call("list_slices", serde_json::json!({}));
    assert_eq!(slices[0]["claimed_by"], "test-agent");
    assert!(slices[0]["worktree_path"]
        .as_str()
        .unwrap()
        .ends_with("widget"));
}

#[test]
fn a_concurrent_section_edit_is_refused_rather_than_overwritten() {
    let fx = fixture();
    let mut s = Server::start(&fx.db, &fx.repo);
    s.call("create_plan", serde_json::json!({ "title": "Accounts V2" }));

    let first = s.call(
        "update_section",
        serde_json::json!({ "key": "grounding", "body": "Agent A's grounding." }),
    );
    let rev = first["rev"].as_i64().expect("a rev");

    s.call(
        "update_section",
        serde_json::json!({ "key": "grounding", "body": "Agent B got here first." }),
    );

    let err = s.call_err(
        "update_section",
        serde_json::json!({
            "key": "grounding", "body": "Agent A's stale write", "expect_rev": rev
        }),
    );
    assert!(err.contains("changed by another writer"), "{err}");

    let md = s.call("get_plan", serde_json::json!({}));
    assert!(md.as_str().unwrap().contains("Agent B got here first."));
}

#[test]
fn importing_defaults_to_a_dry_run() {
    let fx = fixture();
    let file = fx.repo.join("ACME-1201_BUILD_PLAN.md");
    std::fs::write(
        &file,
        "# ACME-1201 - Uplift assign to\n\n## 1. Outcome\n\nDo the thing.\n\n\
         ## 4. Delivery\n\n### S1 - Extract the field\n\nWork.\n",
    )
    .unwrap();

    let mut s = Server::start(&fx.db, &fx.repo);
    let report = s.call(
        "import_markdown",
        serde_json::json!({ "paths": [file.to_string_lossy()] }),
    );
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["files"][0]["outcome"], "would_create");
    assert_eq!(report["files"][0]["plan"], "acme-1201");
    assert!(s
        .call("list_plans", serde_json::json!({}))
        .as_array()
        .unwrap()
        .is_empty());

    let report = s.call(
        "import_markdown",
        serde_json::json!({ "paths": [file.to_string_lossy()], "dry_run": false }),
    );
    assert_eq!(report["files"][0]["outcome"], "created");
    let plans = s.call("list_plans", serde_json::json!({}));
    assert_eq!(plans.as_array().unwrap().len(), 1);
    // Import never removes the source file.
    assert!(file.exists());
}
