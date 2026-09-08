//! `aip delete`, driven as a real process.
//!
//! Deleting is the only write the tool cannot take back, so what is tested here is
//! mostly what it *refuses*: an unnamed plan, an unconfirmed delete with nowhere to
//! prompt, and a dry run that leaves everything where it was.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
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

impl Fixture {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(bin())
            .args(args)
            .env("AI_PLANNER_DB", &self.db)
            .current_dir(&self.repo)
            .output()
            .expect("aip runs")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "aip {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 output")
    }

    fn err(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(!out.status.success(), "aip {args:?} was expected to fail");
        String::from_utf8(out.stderr).expect("utf-8 output")
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let mut args = args.to_vec();
        args.push("--json");
        serde_json::from_str(&self.ok(&args)).expect("json output")
    }

    fn plans(&self) -> Vec<String> {
        self.json(&["ls"])
            .as_array()
            .expect("a plan list")
            .iter()
            .map(|p| p["slug"].as_str().unwrap_or_default().to_string())
            .collect()
    }
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

    let fx = Fixture {
        _dir: dir,
        db: repo.join("planner.db"),
        repo: repo.clone(),
    };
    fx.ok(&["init"]);
    fx.ok(&["new", "ACME-1234 - Reusable Date Range Picker"]);
    fx.ok(&["slice", "add", "PR1", "Shared core", "--files", "40"]);
    fx.ok(&["log", "Grounded and planned."]);
    fx.ok(&[
        "gotcha",
        "add",
        "The Herd symlink is shared",
        "Put it back.",
    ]);
    fx
}

#[test]
fn deleting_takes_the_plan_and_everything_on_it() {
    let fx = fixture();

    let report = fx.json(&["delete", "acme-1234", "--yes"]);
    assert_eq!(report["deleted"], true);
    assert_eq!(report["plan"]["slug"], "acme-1234");
    assert_eq!(report["plan"]["slices"], 1);
    assert_eq!(report["plan"]["gotchas"], 1);
    assert!(report["plan"]["log_entries"].as_i64().unwrap() >= 1);

    assert!(fx.plans().is_empty());
    // The plan is out of the search index with it, not only out of the table.
    assert!(fx
        .json(&["find", "Herd", "symlink"])
        .as_array()
        .unwrap()
        .is_empty());
    // And `aip status` says there is nothing here rather than pointing at a ghost.
    let err = fx.err(&["show"]);
    assert!(err.contains("aip new"), "{err}");
}

#[test]
fn a_delete_is_never_taken_from_the_worktree_and_never_happens_unasked() {
    let fx = fixture();

    // No plan named: the resolution cascade is deliberately not consulted.
    let err = fx.err(&["delete"]);
    assert!(err.contains("name the plan to delete"), "{err}");

    // Named, but nothing said yes, and a test's stdin is not a terminal to ask at.
    let err = fx.err(&["delete", "acme-1234"]);
    assert!(err.contains("--yes"), "{err}");

    // A dry run shows the size of it and changes nothing.
    let report = fx.json(&["delete", "acme-1234", "--dry-run"]);
    assert_eq!(report["deleted"], false);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["plan"]["slices"], 1);

    // The plain form is readable rather than a row count.
    let out = fx.ok(&["delete", "acme-1234", "--dry-run"]);
    assert!(
        out.contains("ACME-1234 - Reusable Date Range Picker"),
        "{out}"
    );
    assert!(out.contains("1 slice"), "{out}");
    assert!(out.contains("nothing was deleted"), "{out}");

    assert_eq!(fx.plans(), vec!["acme-1234"]);
}

#[test]
fn a_plan_claimed_in_another_worktree_survives_until_the_delete_is_forced() {
    let fx = fixture();
    let other = fx._dir.path().join("wt2");
    git(
        &fx.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/core",
            other.to_str().unwrap(),
        ],
    );
    let out = Command::new(bin())
        .args(["slice", "claim", "PR1"])
        .env("AI_PLANNER_DB", &fx.db)
        .current_dir(&other)
        .output()
        .expect("aip runs");
    assert!(out.status.success(), "{out:?}");

    let err = fx.err(&["delete", "acme-1234", "--yes"]);
    assert!(err.contains("claimed in another worktree"), "{err}");
    assert_eq!(fx.plans(), vec!["acme-1234"]);

    let report = fx.json(&["delete", "acme-1234", "--yes", "--force"]);
    assert_eq!(report["deleted"], true);
    assert_eq!(report["plan"]["held"][0]["key"], "PR1");
    assert!(fx.plans().is_empty());
}
