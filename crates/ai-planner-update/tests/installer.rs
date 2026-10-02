//! Real installer processes, fake release transport, disposable destinations.
#![cfg(unix)]

use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    root: tempfile::TempDir,
    cli: PathBuf,
    app: PathBuf,
    tools: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("Applications and tools");
        let cli = dest.join("aip");
        let app = dest.join("ai-planner.app");
        let tools = root.path().join("tools");
        let stage = root.path().join("stage");
        for dir in [
            &dest,
            &app,
            &tools,
            &stage.join("ai-planner.app/Contents/MacOS"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        executable(&cli, "#!/bin/sh\n# old cli\necho 'aip 0.4.1'\n");
        std::fs::write(app.join("old"), "old app").unwrap();
        std::fs::create_dir(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.zottiben.ai-planner</string><key>CFBundleShortVersionString</key><string>0.4.1</string></dict></plist>").unwrap();
        executable(&stage.join("aip"), "#!/bin/sh\necho 'aip 9.0.0'\n");
        executable(
            &stage.join("ai-planner.app/Contents/MacOS/ai-planner"),
            "#!/bin/sh\nexit 0\n",
        );
        std::fs::write(stage.join("ai-planner.app/Contents/Info.plist"), "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>dev.zottiben.ai-planner</string><key>CFBundleShortVersionString</key><string>9.0.0</string></dict></plist>").unwrap();
        let archive = root.path().join("release.tar.gz");
        assert!(Command::new("tar")
            .args(["czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&stage)
            .args(["aip", "ai-planner.app"])
            .status()
            .unwrap()
            .success());
        let platform = if cfg!(target_os = "macos") {
            "macos-universal"
        } else if cfg!(target_arch = "aarch64") {
            "linux-aarch64"
        } else {
            "linux-x86_64"
        };
        let checksum = format!(
            "{:x}  ai-planner-v9.0.0-{platform}.tar.gz\n",
            Sha256::digest(std::fs::read(archive).unwrap())
        );
        std::fs::write(root.path().join("checksums.txt"), checksum).unwrap();
        executable(&tools.join("curl"), "#!/bin/sh\nwhile [ $# -gt 0 ]; do\n case \"$1\" in\n https://*) url=$1 ;;\n -o) shift; output=$1 ;;\n esac\n shift\ndone\nprintf '%s\\n' \"$url\" >> \"$FIXTURE/requests\"\ncase \"$url\" in\n */checksums.txt) cp \"$FIXTURE/checksums.txt\" \"$output\" ;;\n *) cp \"$FIXTURE/release.tar.gz\" \"$output\" ;;\nesac\n");
        Self {
            root,
            cli,
            app,
            tools,
        }
    }

    fn run(&self, cli: bool, app: bool) -> Output {
        Command::new("sh")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install/update-release.sh"))
            .arg("9.0.0")
            .arg(if cli {
                self.cli.as_path()
            } else {
                Path::new("")
            })
            .arg(if app {
                self.app.as_path()
            } else {
                Path::new("")
            })
            .args(["0.4.1", "0.4.1"])
            .env("HOME", self.root.path())
            .env("FIXTURE", self.root.path())
            .env("PATH", format!("{}:/usr/bin:/bin", self.tools.display()))
            .output()
            .unwrap()
    }

    fn assert_old(&self) {
        assert!(std::fs::read_to_string(&self.cli)
            .unwrap()
            .contains("old cli"));
        assert_eq!(
            std::fs::read_to_string(self.app.join("old")).unwrap(),
            "old app"
        );
    }
}

#[test]
fn updates_the_explicit_cli_path_and_records_scoped_provenance() {
    let f = Fixture::new();
    let out = f.run(true, false);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(&f.cli)
        .unwrap()
        .contains("aip 9.0.0"));
    assert_eq!(
        std::fs::read_to_string(f.root.path().join(".ai-planner/install-method")).unwrap(),
        "release\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join(".ai-planner/install-path"))
            .unwrap()
            .trim(),
        f.cli.to_str().unwrap()
    );
    assert!(std::fs::read_to_string(f.root.path().join("requests"))
        .unwrap()
        .lines()
        .all(|url| url.contains("/v9.0.0/")));
    assert!(!f.cli.with_extension("update-lock").exists());
}

#[test]
fn a_bad_checksum_changes_neither_target() {
    let f = Fixture::new();
    std::fs::write(f.root.path().join("release.tar.gz"), "corrupt download").unwrap();
    let out = f.run(true, cfg!(target_os = "macos"));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("checksum mismatch"));
    f.assert_old();
}

#[test]
fn missing_checksums_are_not_best_effort() {
    let f = Fixture::new();
    std::fs::write(f.root.path().join("checksums.txt"), "").unwrap();
    let out = f.run(true, false);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("missing checksum"));
    f.assert_old();
}

#[test]
fn a_second_updater_cannot_touch_a_locked_target() {
    let f = Fixture::new();
    std::fs::create_dir(f.cli.with_extension("update-lock")).unwrap();
    let out = f.run(true, false);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("another update holds"));
    f.assert_old();
    assert!(
        f.cli.with_extension("update-lock").exists(),
        "must not clear somebody else's lock"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_desktop_only_update_does_not_replace_a_source_cli_or_its_provenance() {
    let f = Fixture::new();
    let out = f.run(false, true);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!f.app.join("old").exists());
    assert_eq!(ai_planner_update::app_version(&f.app).unwrap(), "9.0.0");
    assert!(std::fs::read_to_string(&f.cli).unwrap().contains("old cli"));
    assert!(!f.root.path().join(".ai-planner").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn both_components_are_replaced_by_the_same_verified_release() {
    let f = Fixture::new();
    let out = f.run(true, true);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(&f.cli)
        .unwrap()
        .contains("aip 9.0.0"));
    assert_eq!(ai_planner_update::app_version(&f.app).unwrap(), "9.0.0");
    assert!(!f.app.join("old").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn a_failed_cli_replacement_rolls_the_app_back_too() {
    let f = Fixture::new();
    executable(&f.tools.join("mv"), "#!/bin/sh\ncase \"$2\" in */next) echo 'simulated replacement failure' >&2; exit 1 ;; esac\nexec /bin/mv \"$@\"\n");
    let out = f.run(true, true);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("simulated replacement failure"));
    f.assert_old();
}

#[test]
fn a_changed_target_is_refused_under_the_lock_before_downloading() {
    let f = Fixture::new();
    executable(&f.cli, "#!/bin/sh\necho 'aip 10.0.0'\n");
    let out = f.run(true, false);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("CLI changed since checking"));
    assert!(!f.root.path().join("requests").exists());
    assert!(std::fs::read_to_string(&f.cli).unwrap().contains("10.0.0"));
}

#[test]
fn the_shared_installer_rechecks_on_disk_versions_after_confirmation() {
    let f = Fixture::new();
    executable(&f.cli, "#!/bin/sh\necho 'aip 10.0.0'\n");
    let error = ai_planner_update::install_release("9.0.0", Some(&f.cli), None).unwrap_err();
    assert!(
        error.to_string().contains("refusing to downgrade"),
        "{error}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn an_unrelated_app_is_never_replaced() {
    let f = Fixture::new();
    let plist = f.app.join("Contents/Info.plist");
    let text = std::fs::read_to_string(&plist)
        .unwrap()
        .replace("dev.zottiben.ai-planner", "dev.example.other-app");
    std::fs::write(&plist, text).unwrap();
    let error = ai_planner_update::app_version(&f.app).unwrap_err();
    assert!(error.to_string().contains("not an ai-planner app"));
    let out = f.run(false, true);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not an ai-planner app"));
    f.assert_old();
}

#[test]
fn a_target_changed_during_download_is_not_overwritten() {
    let f = Fixture::new();
    let curl = f.tools.join("curl");
    let script = std::fs::read_to_string(&curl).unwrap();
    executable(
        &curl,
        &format!(
            r#"{script}
printf '%s\n' '#!/bin/sh' 'echo "aip 10.0.0"' > "$FIXTURE/Applications and tools/aip"
"#
        ),
    );
    let out = f.run(true, false);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("CLI changed since checking"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(&f.cli).unwrap().contains("10.0.0"));
}

#[test]
fn provenance_failure_restores_the_cli_and_previous_markers() {
    let f = Fixture::new();
    let provenance = f.root.path().join(".ai-planner");
    std::fs::create_dir(&provenance).unwrap();
    std::fs::write(provenance.join("install-method"), "release\n").unwrap();
    std::fs::write(provenance.join("install-path"), "/other/aip\n").unwrap();
    executable(&f.tools.join("cp"), "#!/bin/sh\ncase \"$1\" in */install-method) echo 'provenance write failed' >&2; exit 1 ;; esac\nexec /bin/cp \"$@\"\n");
    let out = f.run(true, false);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("provenance write failed"));
    f.assert_old();
    assert_eq!(
        std::fs::read_to_string(provenance.join("install-method")).unwrap(),
        "release\n"
    );
    assert_eq!(
        std::fs::read_to_string(provenance.join("install-path")).unwrap(),
        "/other/aip\n"
    );
}
