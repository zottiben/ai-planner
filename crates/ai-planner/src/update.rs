//! Self-update, and the assets the binary carries so it can refresh its own setup.
//!
//! Unlike `awt`, there are no release binaries to download: this is installed with
//! `cargo install`, so updating means running that again. The one thing that must not
//! be guessed is *how* it was installed - a rebuild that quietly drops
//! `--features model-embeddings` leaves semantic search broken with no error - so the
//! source and the feature list are read back out of cargo's own records rather than
//! assumed.
//!
//! The skill and the hook script are embedded with `include_str!`, which means they
//! can never be out of step with the binary and an update needs no repo clone and no
//! network beyond what cargo itself does.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const SKILL: &str = include_str!("../../../skill/SKILL.md");
pub const HOOK_SCRIPT: &str = include_str!("../../../install/hooks/ai-planner-session.sh");
pub const SKILL_NAME: &str = "ai-planner";

/// The events the hook script serves. `PreCompact` and `SessionEnd` are absent on
/// purpose: neither can inject context, so a hook there could only block.
pub const HOOK_EVENTS: [(&str, &str); 3] = [
    ("SessionStart", "session-start"),
    ("UserPromptSubmit", "user-prompt-submit"),
    ("Stop", "stop"),
];

/// Where cargo installed this from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A local clone. Updating means rebuilding whatever is in that directory now.
    Path(PathBuf),
    /// A git remote, pinned to the commit that was built.
    Git { url: String, sha: Option<String> },
    /// crates.io.
    Registry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub version: String,
    pub source: Source,
    pub features: Vec<String>,
    pub all_features: bool,
    pub no_default_features: bool,
}

impl Install {
    /// The `cargo install` invocation that reproduces this install, one version newer.
    pub fn cargo_args(&self) -> Vec<String> {
        let mut args: Vec<String> = vec!["install".into()];
        match &self.source {
            Source::Path(path) => {
                args.push("--path".into());
                args.push(path.to_string_lossy().into_owned());
            }
            Source::Git { url, .. } => {
                args.push("--git".into());
                args.push(url.clone());
                args.push("ai-planner".into());
            }
            Source::Registry => args.push("ai-planner".into()),
        }
        // `--force` because the version number does not change between commits, and
        // cargo would otherwise decide there is nothing to do.
        args.push("--force".into());
        args.push("--locked".into());
        if self.all_features {
            args.push("--all-features".into());
        } else if !self.features.is_empty() {
            args.push("--features".into());
            args.push(self.features.join(","));
        }
        if self.no_default_features {
            args.push("--no-default-features".into());
        }
        args
    }

    pub fn describe(&self) -> String {
        let where_ = match &self.source {
            Source::Path(p) => format!("local clone {}", p.display()),
            Source::Git { url, sha } => match sha {
                Some(sha) => format!("{url} @ {}", &sha[..sha.len().min(10)]),
                None => url.clone(),
            },
            Source::Registry => "crates.io".to_string(),
        };
        if self.features.is_empty() {
            where_
        } else {
            format!("{where_} (features: {})", self.features.join(", "))
        }
    }
}

pub fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from(".cargo"))
}

/// Read back how a package was installed. `None` when cargo has no record of it,
/// which means it was not installed with `cargo install` - a `target/debug` build, or
/// a binary copied into place by hand.
pub fn installed(cargo_home: &Path, package: &str) -> Result<Option<Install>> {
    let crates_toml = cargo_home.join(".crates.toml");
    if !crates_toml.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&crates_toml)
        .with_context(|| format!("reading {}", crates_toml.display()))?;

    let Some((key, version, source)) = text
        .lines()
        .filter_map(|line| line.split_once(" = "))
        .filter_map(|(key, _)| {
            let key = key.trim().trim_matches('"');
            parse_key(key).map(|(name, version, source)| (key.to_string(), name, version, source))
        })
        .find(|(_, name, _, _)| name == package)
        .map(|(key, _, version, source)| (key, version, source))
    else {
        return Ok(None);
    };

    // Feature selection lives in the sibling json, keyed by the same string.
    let (mut features, mut all_features, mut no_default_features) = (Vec::new(), false, false);
    let crates2 = cargo_home.join(".crates2.json");
    if let Ok(raw) = std::fs::read_to_string(&crates2) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(entry) = value.get("installs").and_then(|i| i.get(&key)) {
                features = entry
                    .get("features")
                    .and_then(|f| f.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                all_features = entry
                    .get("all_features")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                no_default_features = entry
                    .get("no_default_features")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
            }
        }
    }

    Ok(Some(Install {
        version,
        source,
        features,
        all_features,
        no_default_features,
    }))
}

/// `ai-planner 0.1.0 (git+https://github.com/zottiben/ai-planner#930e0bf)` ->
/// name, version, source.
fn parse_key(key: &str) -> Option<(String, String, Source)> {
    let mut parts = key.splitn(3, ' ');
    let name = parts.next()?.to_string();
    let version = parts.next()?.to_string();
    let origin = parts.next()?.trim();
    let origin = origin.strip_prefix('(')?.strip_suffix(')')?;

    let source = if let Some(rest) = origin.strip_prefix("path+file://") {
        Source::Path(PathBuf::from(rest))
    } else if let Some(rest) = origin.strip_prefix("git+") {
        match rest.split_once('#') {
            Some((url, sha)) => Source::Git {
                url: url.to_string(),
                sha: Some(sha.to_string()),
            },
            None => Source::Git {
                url: rest.to_string(),
                sha: None,
            },
        }
    } else if origin.starts_with("registry+") {
        Source::Registry
    } else {
        return None;
    };
    Some((name, version, source))
}

/// The commit the remote's default branch is on, so a git install can say whether
/// there is anything to update to.
pub fn remote_head(url: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["ls-remote", url, "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(str::to_string)
}

/// Where the skill goes. Both conventions, because the harnesses disagree.
pub fn skill_targets(project: bool) -> Vec<PathBuf> {
    if project {
        vec![
            PathBuf::from(".claude/skills").join(SKILL_NAME),
            PathBuf::from(".agents/skills").join(SKILL_NAME),
        ]
    } else {
        let home = home_dir();
        vec![
            home.join(".claude/skills").join(SKILL_NAME),
            home.join(".agents/skills").join(SKILL_NAME),
        ]
    }
}

pub fn hook_dir(project: bool) -> PathBuf {
    if project {
        PathBuf::from(".agents/hooks")
    } else {
        home_dir().join(".ai-planner/hooks")
    }
}

pub fn settings_path(project: bool) -> PathBuf {
    if project {
        PathBuf::from(".claude/settings.json")
    } else {
        home_dir().join(".claude/settings.json")
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Merge our three hook entries into a harness settings document, leaving every other
/// hook alone. Matching is on the script path rather than the whole command, so an
/// install from before the extra events existed is upgraded in place instead of being
/// left behind as a duplicate.
pub fn merge_hooks(settings: &mut serde_json::Value, script: &str) -> Vec<String> {
    use serde_json::{json, Value};

    if !settings.is_object() {
        *settings = json!({});
    }
    let hooks = settings
        .as_object_mut()
        .expect("object")
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }

    let mut added = Vec::new();
    for (event, arg) in HOOK_EVENTS {
        let command = format!("{script} {arg}");
        let groups = hooks
            .as_object_mut()
            .expect("object")
            .entry(event)
            .or_insert_with(|| json!([]));
        if !groups.is_array() {
            *groups = json!([]);
        }
        let array = groups.as_array_mut().expect("array");

        let mut refreshed = false;
        for group in array.iter_mut() {
            let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            for hook in list.iter_mut() {
                let matches = hook
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.contains(script));
                if matches {
                    hook["command"] = Value::String(command.clone());
                    refreshed = true;
                }
            }
        }
        if !refreshed {
            array.push(json!({ "hooks": [{ "type": "command", "command": command }] }));
            added.push(event.to_string());
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_install_is_read_back_with_its_features() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".crates.toml"),
            "[v1]\n\"ai-planner 0.1.0 (path+file:///Users/me/src/ai-planner/crates/ai-planner)\" = [\"aip\"]\n\
             \"other 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"other\"]\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".crates2.json"),
            r#"{"installs":{"ai-planner 0.1.0 (path+file:///Users/me/src/ai-planner/crates/ai-planner)":
               {"features":["model-embeddings"],"all_features":false,"no_default_features":false}}}"#,
        )
        .unwrap();

        let install = installed(dir.path(), "ai-planner").unwrap().unwrap();
        assert_eq!(
            install.source,
            Source::Path(PathBuf::from("/Users/me/src/ai-planner/crates/ai-planner"))
        );
        assert_eq!(install.features, vec!["model-embeddings"]);

        // The rebuild must carry the features over, or semantic search silently stops
        // working with no error to explain it.
        let args = install.cargo_args();
        assert_eq!(
            args,
            vec![
                "install",
                "--path",
                "/Users/me/src/ai-planner/crates/ai-planner",
                "--force",
                "--locked",
                "--features",
                "model-embeddings",
            ]
        );
    }

    #[test]
    fn a_git_install_keeps_its_url_and_records_the_commit_it_was_built_from() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".crates.toml"),
            "[v1]\n\"ai-planner 0.1.0 (git+https://github.com/zottiben/ai-planner#930e0bfe9efd4d25b5d7abf6d4aedd6d876c6ea4)\" = [\"aip\"]\n",
        )
        .unwrap();

        let install = installed(dir.path(), "ai-planner").unwrap().unwrap();
        match &install.source {
            Source::Git { url, sha } => {
                assert_eq!(url, "https://github.com/zottiben/ai-planner");
                assert_eq!(sha.as_deref().unwrap().len(), 40);
            }
            other => panic!("expected a git source, got {other:?}"),
        }
        assert_eq!(
            install.cargo_args(),
            vec![
                "install",
                "--git",
                "https://github.com/zottiben/ai-planner",
                "ai-planner",
                "--force",
                "--locked",
            ]
        );
    }

    #[test]
    fn a_binary_cargo_never_installed_is_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        assert!(installed(dir.path(), "ai-planner").unwrap().is_none());

        std::fs::write(dir.path().join(".crates.toml"), "[v1]\n").unwrap();
        assert!(installed(dir.path(), "ai-planner").unwrap().is_none());
    }

    #[test]
    fn hooks_are_merged_without_disturbing_anything_else() {
        let mut settings: serde_json::Value = serde_json::from_str(
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/other/hook.sh"}]}]},
                "permissions":{"allow":["Bash"]}}"#,
        )
        .unwrap();

        let added = merge_hooks(&mut settings, "/home/me/.ai-planner/hooks/session.sh");
        assert_eq!(added.len(), 3, "all three events are new here");

        // The unrelated hook and unrelated settings survive.
        let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts[0]["hooks"][0]["command"], "/other/hook.sh");
        assert_eq!(settings["permissions"]["allow"][0], "Bash");
        assert_eq!(
            starts[1]["hooks"][0]["command"],
            "/home/me/.ai-planner/hooks/session.sh session-start"
        );
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["command"],
            "/home/me/.ai-planner/hooks/session.sh stop"
        );

        // Merging again changes nothing.
        let again = merge_hooks(&mut settings, "/home/me/.ai-planner/hooks/session.sh");
        assert!(again.is_empty());
        assert_eq!(
            settings["hooks"]["SessionStart"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn an_older_single_event_install_is_upgraded_in_place() {
        // Before the extra events existed the command had no argument at all.
        let script = "/home/me/.ai-planner/hooks/session.sh";
        let mut settings: serde_json::Value = serde_json::from_str(&format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"{script}"}}]}}]}}}}"#
        ))
        .unwrap();

        let added = merge_hooks(&mut settings, script);
        assert_eq!(added, vec!["UserPromptSubmit", "Stop"]);
        let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(
            starts.len(),
            1,
            "the stale entry is rewritten, not duplicated"
        );
        assert_eq!(
            starts[0]["hooks"][0]["command"],
            format!("{script} session-start")
        );
    }

    #[test]
    fn the_embedded_assets_are_the_real_ones() {
        assert!(SKILL.starts_with("---\nname: ai-planner"));
        assert!(HOOK_SCRIPT.contains("aip -C \"$proj\" hook --event"));
    }
}
