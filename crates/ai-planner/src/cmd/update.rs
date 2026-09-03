use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::cli::UpdateArgs;
use crate::out::{bold, dim, ok};
use crate::update::{self, Source};

const PACKAGE: &str = "ai-planner";

pub fn update(args: &UpdateArgs) -> Result<()> {
    let home = update::cargo_home();
    let Some(install) = update::installed(&home, PACKAGE)? else {
        anyhow::bail!(
            "cargo has no record of installing {PACKAGE}, so there is nothing to update.\n  \
             This binary was not installed with `cargo install` - if you are running it out of \
             ./target, build it there instead."
        );
    };

    println!(
        "{} {}",
        bold(&format!("aip {}", env!("CARGO_PKG_VERSION"))),
        dim(&install.describe())
    );

    // Say what an update would even mean before doing one. For a git install that is
    // a commit comparison; for a local clone it is whatever the clone contains now,
    // which only the user can speak to.
    let available = match &install.source {
        Source::Git { url, sha } => match (update::remote_head(url), sha) {
            (Some(head), Some(built)) if head == *built => {
                println!("{}", dim("  already on the remote's latest commit"));
                false
            }
            (Some(head), _) => {
                println!(
                    "{}",
                    dim(&format!("  remote is at {}", &head[..head.len().min(10)]))
                );
                true
            }
            (None, _) => {
                println!(
                    "{}",
                    dim("  could not reach the remote - will rebuild anyway")
                );
                true
            }
        },
        Source::Path(path) => {
            if !path.exists() {
                anyhow::bail!(
                    "the clone it was installed from is gone: {}\n  Re-install with: cargo \
                     install --git {} ai-planner --locked",
                    path.display(),
                    env!("CARGO_PKG_REPOSITORY")
                );
            }
            println!(
                "{}",
                dim("  installed from a local clone - `git pull` there first to get new commits")
            );
            true
        }
        Source::Registry => true,
    };

    if args.check {
        return Ok(());
    }
    if !available && !args.force {
        println!(
            "\nNothing to do. {}",
            dim("Pass --force to rebuild anyway.")
        );
        return Ok(());
    }

    // A newer binary may add migrations, and those run silently on first open. Take a
    // copy first so a bad upgrade is recoverable.
    match backup_database() {
        Ok(Some(path)) => ok(&format!(
            "backed up the database to {}",
            super::setup::shown(&path)
        )),
        Ok(None) => println!("{}", dim("no database yet - nothing to back up")),
        Err(err) => println!(
            "{}",
            dim(&format!("could not back up the database: {err:#}"))
        ),
    }

    let cargo_args = install.cargo_args();
    println!();
    println!("{} cargo {}", dim("running"), cargo_args.join(" "));
    let status = std::process::Command::new(cargo())
        .args(&cargo_args)
        .status()
        .context("running cargo install")?;
    if !status.success() {
        anyhow::bail!("cargo install failed - the previous binary is untouched");
    }
    ok("binary reinstalled");

    // The skill, the rules and the hooks live outside the binary, and every one of
    // them changes between versions. Refreshing them is the half of an update that is
    // easy to forget and produces the strangest symptoms when skipped.
    println!();
    println!("{}", bold("refreshing the installed setup"));
    let installed_aip = update::cargo_home().join("bin").join("aip");
    let refreshed = std::process::Command::new(&installed_aip)
        .args(["setup", "--force"])
        .status();
    match refreshed {
        Ok(status) if status.success() => {}
        // Run by the *new* binary, so it picks up the new skill and rules. If that
        // fails, fall back to this binary's own copies rather than leaving them stale.
        _ => {
            println!(
                "{}",
                dim("  the new binary could not run setup - using this one")
            );
            super::setup::setup(&crate::cli::SetupArgs {
                project: false,
                force: true,
            })?;
        }
    }

    println!();
    println!(
        "Done. {}",
        dim("Restart your agent, then run `aip doctor`.")
    );
    Ok(())
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

/// `VACUUM INTO` a timestamped copy, which is consistent even while other agents are
/// mid-write.
fn backup_database() -> Result<Option<PathBuf>> {
    let path = ai_planner_core::default_db_path();
    if !path.exists() {
        return Ok(None);
    }
    let store = ai_planner_core::Store::open(&path)?;
    let stamp = ai_planner_core::util::now()
        .replace([':', '-'], "")
        .replace('Z', "");
    let dest = path.with_file_name(format!(
        "{}.{stamp}.pre-update.bak",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("planner.db")
    ));
    store.backup(&dest)?;
    Ok(Some(dest))
}
