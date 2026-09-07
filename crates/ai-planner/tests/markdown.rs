//! What `aip show` puts on stdout, driven as a real process.
//!
//! Rendering markdown through gum is a terminal affordance, and the thing that must
//! not break is everything else: an agent, a `>` redirect and a `|` still get the
//! document byte for byte. A test binary's stdout is a pipe, so the plain path is
//! what it observes naturally, and `AI_PLANNER_MARKDOWN=always` is what forces the
//! rendered one.

use std::path::{Path, PathBuf};
use std::process::Command;

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
    /// Run `aip` with stdout on a pipe, which is what an agent and a redirect see.
    fn aip(&self, args: &[&str], env: &[(&str, &str)]) -> String {
        let mut cmd = Command::new(bin());
        cmd.args(args)
            .env("AI_PLANNER_DB", &self.db)
            .current_dir(&self.repo);
        for (key, value) in env {
            cmd.env(key, value);
        }
        let out = cmd.output().expect("aip runs");
        assert!(
            out.status.success(),
            "aip {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 output")
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
    fx.aip(&["init"], &[]);
    fx.aip(&["new", "Reusable date range picker"], &[]);
    fx.aip(
        &[
            "section",
            "scope",
            "--title",
            "Scope",
            "A **reusable** picker.",
        ],
        &[],
    );
    fx.aip(&["slice", "add", "PR1", "Shared core"], &[]);
    fx
}

/// The visible text, with the styling taken back out. gum splits a heading across
/// several escape sequences, so the words are only contiguous once they are gone.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            in_escape = ch != 'm';
            continue;
        }
        match ch {
            '\x1b' => in_escape = true,
            _ => out.push(ch),
        }
    }
    out
}

fn gum_installed() -> bool {
    Command::new("gum")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn a_piped_plan_is_the_markdown_and_nothing_else() {
    let fx = fixture();
    let piped = fx.aip(&["show"], &[]);
    assert!(piped.starts_with("# Reusable date range picker"), "{piped}");
    assert!(piped.contains("A **reusable** picker."), "{piped}");
    assert!(piped.contains("| PR1 | Shared core |"), "{piped}");
    assert!(
        !piped.contains('\x1b'),
        "piped output must carry no escape codes: {piped:?}"
    );
    assert_eq!(piped, fx.aip(&["--plain", "show"], &[]));
}

#[test]
fn plain_wins_over_asking_for_rendering() {
    let fx = fixture();
    assert_eq!(
        fx.aip(&["--plain", "show"], &[("AI_PLANNER_MARKDOWN", "always")]),
        fx.aip(&["show"], &[]),
    );
    // Machine-readable output is never a rendered document either.
    let json = fx.aip(&["--json", "show"], &[("AI_PLANNER_MARKDOWN", "always")]);
    assert!(json.trim_start().starts_with('{'), "{json}");
}

#[test]
fn forcing_rendering_hands_the_plan_to_gum() {
    let fx = fixture();
    let rendered = fx.aip(&["show"], &[("AI_PLANNER_MARKDOWN", "always")]);
    let plain = fx.aip(&["show"], &[]);
    if !gum_installed() {
        // Nothing to render with is not an error: the markdown speaks for itself.
        assert_eq!(rendered, plain);
        return;
    }
    assert_ne!(rendered, plain);
    assert!(
        rendered.contains('\x1b'),
        "gum should have styled it: {rendered:?}"
    );
    // Rendering restyles the syntax rather than dropping the content.
    let text = strip_ansi(&rendered);
    assert!(text.contains("Reusable date range picker"), "{text}");
    assert!(text.contains("Shared core"), "{text}");
    assert!(text.contains("reusable picker."), "{text}");
    assert!(!text.contains("**reusable**"), "{text}");
}
