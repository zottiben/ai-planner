//! Updating follows the executable, not a different installation's metadata.
//! Real processes get isolated HOME, Cargo records, database and network commands.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    home: tempfile::TempDir,
    active: PathBuf,
    cargo: PathBuf,
    tools: PathBuf,
}

impl Fixture {
    fn new(source: bool, latest: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let local = home.path().join(".local/bin");
        let cargo = home.path().join(".cargo");
        let tools = home.path().join("tools");
        for dir in [&local, &cargo.join("bin"), &tools] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let active = if source {
            cargo.join("bin/aip")
        } else {
            local.join("aip")
        };
        std::fs::copy(env!("CARGO_BIN_EXE_aip"), &active).unwrap();
        let version = if source {
            env!("CARGO_PKG_VERSION")
        } else {
            "9.0.0"
        };
        std::fs::write(cargo.join(".crates.toml"), format!("[v1]\n\"ai-planner {version} (git+https://github.com/zottiben/ai-planner#abc123)\" = [\"aip\"]\n")).unwrap();
        if !source {
            executable(&cargo.join("bin/aip"), "#!/bin/sh\necho 'aip 9.0.0'\n");
        }
        executable(&tools.join("git"), "#!/bin/sh\necho 'abc123 HEAD'\n");
        executable(
            &tools.join("curl"),
            &format!("#!/bin/sh\nprintf '%s' '{{\"tag_name\":\"v{latest}\"}}'\n"),
        );
        // Shared installer behavior is covered separately with real staged files.
        // Here capture destinations selected by the CLI, then emulate its new binary.
        executable(&tools.join("sh"), "#!/bin/sh\nprintf '%s\\n' \"$2\" \"$3\" \"$4\" > \"$HOME/installer-args\"\nif [ -n \"$3\" ]; then\n cp \"$HOME/new-aip\" \"$3.next\" && chmod +x \"$3.next\" && mv \"$3.next\" \"$3\"\nfi\n");
        executable(
            &home.path().join("new-aip"),
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$HOME/setup-args\"\n",
        );
        Self {
            home,
            active,
            cargo,
            tools,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(&self.active)
            .arg("update")
            .args(args)
            .env("HOME", self.home.path())
            .env("CARGO_HOME", &self.cargo)
            .env("CARGO", self.tools.join("cargo"))
            .env("AI_PLANNER_DB", self.home.path().join("planner.db"))
            .env("PATH", format!("{}:/usr/bin:/bin", self.tools.display()))
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    #[cfg(target_os = "macos")]
    fn app(&self) -> PathBuf {
        let app = self.home.path().join("Applications/ai-planner.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.zottiben.ai-planner</string><key>CFBundleShortVersionString</key><string>0.4.1</string></dict></plist>").unwrap();
        app.canonicalize().unwrap()
    }
}

#[test]
fn a_shadowed_cargo_install_cannot_report_the_active_binary_as_current() {
    let f = Fixture::new(false, "9.0.0");
    let out = f.run(&["--check", "--no-desktop"]);
    if cfg!(feature = "model-embeddings") {
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr)
            .contains("refusing to replace it with a stock release"));
    } else {
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(stdout.contains("latest release is v9.0.0"), "{stdout}");
        assert!(
            !stdout.contains("already on the remote's latest commit"),
            "{stdout}"
        );
    }
    assert!(!f.home.path().join("installer-args").exists());
    assert!(
        !f.home.path().join(".ai-planner").exists(),
        "--check wrote to HOME"
    );
}

#[test]
fn source_metadata_is_used_for_the_actual_cargo_executable() {
    let f = Fixture::new(true, "9.0.0");
    let stdout = f.ok(&["--check", "--no-desktop"]);
    assert!(
        stdout.contains("already on the remote's latest commit"),
        "{stdout}"
    );
    assert!(!stdout.contains("latest release"), "{stdout}");
}

#[test]
fn another_release_marker_does_not_take_over_a_source_copy() {
    let f = Fixture::new(true, "9.0.0");
    std::fs::create_dir(f.home.path().join(".ai-planner")).unwrap();
    std::fs::write(
        f.home.path().join(".ai-planner/install-method"),
        "release\n",
    )
    .unwrap();
    std::fs::write(
        f.home.path().join(".ai-planner/install-path"),
        f.home.path().join(".local/bin/aip").to_str().unwrap(),
    )
    .unwrap();
    let stdout = f.ok(&["--check", "--no-desktop"]);
    assert!(
        stdout.contains("already on the remote's latest commit"),
        "{stdout}"
    );
}

#[cfg(not(feature = "model-embeddings"))]
#[test]
fn a_release_update_targets_the_executing_copy_and_runs_its_setup() {
    let f = Fixture::new(false, "9.0.0");
    f.ok(&["--no-desktop"]);
    let args = std::fs::read_to_string(f.home.path().join("installer-args")).unwrap();
    assert_eq!(
        args,
        format!("9.0.0\n{}\n\n", f.active.canonicalize().unwrap().display())
    );
    assert_eq!(
        std::fs::read_to_string(f.home.path().join("setup-args")).unwrap(),
        "setup --force\n"
    );
    assert!(std::fs::read_to_string(f.cargo.join("bin/aip"))
        .unwrap()
        .contains("aip 9.0.0"));
}

#[cfg(target_os = "macos")]
#[test]
fn a_current_source_cli_still_updates_an_old_desktop() {
    let f = Fixture::new(true, env!("CARGO_PKG_VERSION"));
    let app = f.app();
    let stdout = f.ok(&["--app", app.to_str().unwrap()]);
    assert!(stdout.contains("CLI: current or newer"), "{stdout}");
    assert!(stdout.contains("Desktop: update available"), "{stdout}");
    let args = std::fs::read_to_string(f.home.path().join("installer-args")).unwrap();
    assert_eq!(
        args,
        format!("{}\n\n{}\n", env!("CARGO_PKG_VERSION"), app.display())
    );
    assert!(!f.home.path().join("setup-args").exists());
}

#[cfg(all(target_os = "macos", not(feature = "model-embeddings")))]
#[test]
fn a_current_release_cli_still_updates_an_old_desktop() {
    let f = Fixture::new(false, env!("CARGO_PKG_VERSION"));
    let app = f.app();
    f.ok(&["--app", app.to_str().unwrap()]);
    let args = std::fs::read_to_string(f.home.path().join("installer-args")).unwrap();
    assert_eq!(
        args,
        format!("{}\n\n{}\n", env!("CARGO_PKG_VERSION"), app.display())
    );
}

#[cfg(not(feature = "model-embeddings"))]
#[test]
fn a_failed_release_check_is_not_reported_as_success() {
    let f = Fixture::new(false, "9.0.0");
    executable(
        &f.tools.join("curl"),
        "#!/bin/sh\necho 'offline' >&2\nexit 7\n",
    );
    let out = f.run(&["--check", "--no-desktop"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("offline"));
    assert!(!f.home.path().join("installer-args").exists());
}

#[test]
fn rebuilding_the_actual_source_copy_keeps_its_feature_flags() {
    let f = Fixture::new(true, "9.0.0");
    let key = format!(
        "ai-planner {} (git+https://github.com/zottiben/ai-planner#abc123)",
        env!("CARGO_PKG_VERSION")
    );
    let records = serde_json::json!({"installs": { key: {"features": ["model-embeddings"], "all_features": false, "no_default_features": true} }});
    std::fs::write(f.cargo.join(".crates2.json"), records.to_string()).unwrap();
    executable(&f.tools.join("cargo"), "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$HOME/cargo-args\"\ncp \"$HOME/new-aip\" \"$CARGO_HOME/bin/aip.next\" && chmod +x \"$CARGO_HOME/bin/aip.next\" && mv \"$CARGO_HOME/bin/aip.next\" \"$CARGO_HOME/bin/aip\"\n");
    f.ok(&["--force", "--no-desktop"]);
    let args = std::fs::read_to_string(f.home.path().join("cargo-args")).unwrap();
    assert!(
        args.contains("--features model-embeddings --no-default-features"),
        "{args}"
    );
    assert_eq!(
        std::fs::read_to_string(f.home.path().join("setup-args")).unwrap(),
        "setup --force\n"
    );
    assert!(!f.home.path().join("installer-args").exists());
}

#[cfg(not(feature = "model-embeddings"))]
#[test]
fn force_does_not_downgrade_a_release_cli() {
    let f = Fixture::new(false, "0.1.0");
    let stdout = f.ok(&["--force", "--no-desktop"]);
    assert!(stdout.contains("Nothing to do"), "{stdout}");
    assert!(!f.home.path().join("installer-args").exists());
}

#[cfg(not(feature = "model-embeddings"))]
#[test]
fn failed_setup_does_not_turn_a_partial_update_into_success() {
    let f = Fixture::new(false, "9.0.0");
    executable(&f.home.path().join("new-aip"), "#!/bin/sh\nexit 1\n");
    let out = f.run(&["--no-desktop"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("binary updated, but setup failed"));
}
