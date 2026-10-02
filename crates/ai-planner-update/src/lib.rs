//! Release updates belong to a particular executable, not whichever `aip` happens
//! to be first on a child process's PATH. The desktop calls the same installer with
//! no CLI target, so a source CLI (and its selected features) is never overwritten.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

pub const RELEASES_URL: &str = "https://github.com/zottiben/ai-planner/releases/latest";
const RELEASE_API: &str = "https://api.github.com/repos/zottiben/ai-planner/releases/latest";
const INSTALLER: &str = include_str!("../../../install/update-release.sh");

pub fn latest_release_version() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "-H",
            "User-Agent: aip",
            RELEASE_API,
        ])
        .output()
        .context("running curl to check for a release")?;
    if !output.status.success() {
        anyhow::bail!(
            "checking GitHub failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    release_version(&output.stdout)
}

pub fn release_version(raw: &[u8]) -> Result<String> {
    let value: serde_json::Value =
        serde_json::from_slice(raw).context("reading GitHub's response")?;
    let version = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .and_then(|tag| tag.strip_prefix('v'))
        .context("the latest release has no v-prefixed tag")?;
    semver::Version::parse(version).context("the latest release is not SemVer")?;
    Ok(version.to_string())
}

pub fn release_order(latest: &str, current: &str) -> Result<Ordering> {
    Ok(semver::Version::parse(latest)
        .context("the latest release is not SemVer")?
        .cmp(&semver::Version::parse(current).context("the installed version is not SemVer")?))
}

/// Treat every component independently. A current CLI can accompany an old app;
/// --force permits a reinstall, never a downgrade.
pub fn needs_update(latest: &str, current: &str, force: bool) -> Result<bool> {
    Ok(match release_order(latest, current)? {
        Ordering::Greater => true,
        Ordering::Equal => force,
        Ordering::Less => false,
    })
}

pub fn default_app(home: &Path) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    [
        home.join("Applications/ai-planner.app"),
        PathBuf::from("/Applications/ai-planner.app"),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

pub fn app_bundle(executable: &Path) -> Result<PathBuf> {
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
        .context("this is not an installed .app bundle; install the desktop release first")
}

fn plist(app: &Path, key: &str) -> Result<String> {
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .with_context(|| format!("reading the version of {}", app.display()))?;
    if !output.status.success() {
        anyhow::bail!("could not read the version of {}", app.display());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

pub fn app_version(app: &Path) -> Result<String> {
    anyhow::ensure!(
        plist(app, "CFBundleIdentifier")? == "dev.zottiben.ai-planner",
        "{} is not an ai-planner app",
        app.display()
    );
    let version = plist(app, "CFBundleShortVersionString")?;
    semver::Version::parse(&version).context("the app version is not SemVer")?;
    Ok(version)
}

fn cli_version(cli: &Path) -> Result<String> {
    let output = Command::new(cli)
        .arg("--version")
        .output()
        .context("checking the on-disk CLI version")?;
    anyhow::ensure!(
        output.status.success(),
        "could not read the version of {}",
        cli.display()
    );
    let text = String::from_utf8(output.stdout)?;
    let version = text
        .trim()
        .strip_prefix("aip ")
        .context("target is not an aip binary")?;
    semver::Version::parse(version).context("the CLI version is not SemVer")?;
    Ok(version.to_string())
}

/// A pinned release, with explicit destinations. Download and stage everything
/// before replacing either target. The script rolls back on a failed replacement.
/// It never elevates privileges or guesses a directory from PATH.
pub fn install_release(version: &str, cli: Option<&Path>, app: Option<&Path>) -> Result<String> {
    semver::Version::parse(version).context("the release version is not SemVer")?;
    anyhow::ensure!(
        cli.is_some() || app.is_some(),
        "no installation target selected"
    );
    anyhow::ensure!(
        !cfg!(windows),
        "install the Windows release from {RELEASES_URL}"
    );
    if app.is_some() {
        anyhow::ensure!(
            cfg!(target_os = "macos"),
            "in-place desktop updates require macOS; use {RELEASES_URL}"
        );
    }
    // Re-read after the user's confirmation. Another updater may have installed
    // a newer build while the dialog was open. The script also checks these
    // snapshots under its target locks and again immediately before publication.
    let cli_version = cli.map(cli_version).transpose()?;
    let app_version = app.map(app_version).transpose()?;
    for current in [cli_version.as_deref(), app_version.as_deref()]
        .into_iter()
        .flatten()
    {
        anyhow::ensure!(
            needs_update(version, current, true)?,
            "refusing to downgrade installed {current} to {version}; check for updates again"
        );
    }
    let scratch = tempfile::tempdir().context("creating an installer directory")?;
    let script = scratch.path().join("update.sh");
    std::fs::write(&script, INSTALLER)?;
    let output = Command::new("sh")
        .arg(&script)
        .arg(version)
        .arg(cli.unwrap_or_else(|| Path::new("")))
        .arg(app.unwrap_or_else(|| Path::new("")))
        .arg(cli_version.as_deref().unwrap_or_default())
        .arg(app_version.as_deref().unwrap_or_default())
        .output()
        .context("running the release installer")?;
    if !output.status.success() {
        anyhow::bail!(
            "release installation failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_component_is_compared_independently_and_never_downgraded() {
        assert!(!needs_update("0.5.1", "0.5.1", false).unwrap());
        assert!(needs_update("0.5.1", "0.4.1", false).unwrap());
        assert!(!needs_update("0.5.1", "0.6.0", true).unwrap());
        assert!(needs_update("0.5.1", "0.5.1", true).unwrap());
        assert!(needs_update("0.5.1", "0.5.1-rc.1", false).unwrap());
    }

    #[test]
    fn github_tags_must_be_versions_not_shell_arguments() {
        assert_eq!(
            release_version(br#"{"tag_name":"v0.5.1"}"#).unwrap(),
            "0.5.1"
        );
        for body in [
            br#"{}"#.as_slice(),
            br#"{"tag_name":"nightly"}"#,
            br#"{"tag_name":"v1;echo bad"}"#,
        ] {
            assert!(release_version(body).is_err());
        }
    }

    #[test]
    fn the_app_target_is_the_running_bundle_not_a_hardcoded_copy() {
        assert_eq!(
            app_bundle(Path::new(
                "/Users/me/Applications/ai-planner.app/Contents/MacOS/ai-planner"
            ))
            .unwrap(),
            PathBuf::from("/Users/me/Applications/ai-planner.app")
        );
        assert!(app_bundle(Path::new("/tmp/target/debug/ai-planner")).is_err());
    }
}
