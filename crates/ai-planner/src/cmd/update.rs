use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cli::UpdateArgs;
use crate::out::{bold, dim, ok};
use crate::update::{self, Install, Source};
use ai_planner_update as release;

pub fn update(args: &UpdateArgs) -> Result<()> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let home = update::home_dir();
    let source = update::source_for_executable(&home, &update::cargo_home(), &executable)?;
    if source.is_none() {
        anyhow::ensure!(
            !cfg!(feature = "model-embeddings"),
            "this feature-enabled binary has no matching Cargo provenance; refusing to replace it with a stock release. Reinstall from its source with model-embeddings enabled"
        );
        anyhow::ensure!(
            !executable
                .components()
                .any(|part| part.as_os_str() == "target"),
            "this is a build-tree binary ({}); rebuild it there or install a release first",
            executable.display()
        );
    }
    println!(
        "{} at {}",
        bold(&format!("aip {}", env!("CARGO_PKG_VERSION"))),
        executable.display()
    );
    let cli_available = match &source {
        Some(install) => source_available(install)?,
        None => {
            println!(
                "{}",
                dim("  prebuilt GitHub release (Cargo records for other copies are ignored)")
            );
            false
        }
    };
    let app = if args.no_desktop {
        None
    } else {
        args.app.clone().or_else(|| release::default_app(&home))
    };
    let app = app
        .map(|path| path.canonicalize())
        .transpose()
        .context("locating the desktop app")?;
    let app_version = app.as_deref().map(release::app_version).transpose()?;
    if let (Some(app), Some(version)) = (&app, &app_version) {
        println!("desktop {version} at {}", app.display());
    }

    // The app is an independent component: even an unchanged source CLI must not
    // short-circuit its release check, and a newer source CLI must never be downgraded.
    let latest = if source.is_none() || app.is_some() {
        let version = release::latest_release_version()?;
        println!("{}", dim(&format!("  latest release is v{version}")));
        Some(version)
    } else {
        None
    };
    let update_cli = if source.is_some() {
        cli_available || args.force
    } else {
        release::needs_update(
            latest.as_deref().expect("release checked"),
            env!("CARGO_PKG_VERSION"),
            args.force,
        )?
    };
    let update_app = match (&latest, &app_version) {
        (Some(latest), Some(current)) => release::needs_update(latest, current, args.force)?,
        _ => false,
    };
    println!(
        "CLI: {}",
        if update_cli {
            "update available"
        } else {
            "current or newer"
        }
    );
    if app.is_some() {
        println!(
            "Desktop: {}",
            if update_app {
                "update available"
            } else {
                "current or newer"
            }
        );
    }
    if args.check {
        return Ok(());
    }
    if !update_cli && !update_app {
        println!(
            "\nNothing to do. {}",
            dim("Pass --force to reinstall current versions.")
        );
        return Ok(());
    }

    // Fail closed: starting the new app can migrate the same live database.
    if let Some(path) = backup_database()? {
        ok(&format!(
            "backed up the database to {}",
            super::setup::shown(&path)
        ));
    }
    if update_cli {
        if let Some(install) = &source {
            let cargo_args = install.cargo_args();
            println!("{} cargo {}", dim("running"), cargo_args.join(" "));
            let status = std::process::Command::new(cargo())
                .args(&cargo_args)
                .status()
                .context("running cargo install")?;
            anyhow::ensure!(
                status.success(),
                "cargo install failed; the desktop was not updated"
            );
            ok(&format!("binary reinstalled at {}", executable.display()));
        }
    }
    let release_cli = (update_cli && source.is_none()).then_some(executable.as_path());
    let release_app = if update_app { app.as_deref() } else { None };
    if release_cli.is_some() || release_app.is_some() {
        println!("Downloading and verifying the release…");
        let result = release::install_release(
            latest.as_deref().expect("release checked"),
            release_cli,
            release_app,
        )
        .context("release update failed; if the source CLI was rebuilt above it remains updated")?;
        print!("{result}");
    }
    if update_cli {
        refresh_setup(&executable)?;
    }
    println!(
        "\nDone. {}",
        dim("Restart running agents, browser servers and the desktop app to use the new binaries.")
    );
    Ok(())
}

fn source_available(install: &Install) -> Result<bool> {
    println!("{}", dim(&install.describe()));
    match &install.source {
        Source::Git { url, sha } => match (update::remote_head(url), sha) {
            (Some(head), Some(built))
                if head == *built && install.version == env!("CARGO_PKG_VERSION") =>
            {
                println!("{}", dim("  already on the remote's latest commit"));
                Ok(false)
            }
            (Some(head), _) => {
                println!(
                    "{}",
                    dim(&format!("  remote is at {}", &head[..head.len().min(10)]))
                );
                Ok(true)
            }
            (None, _) => anyhow::bail!("could not check the source remote; nothing was updated"),
        },
        Source::Path(path) => {
            anyhow::ensure!(
                path.exists(),
                "the source clone is gone: {}",
                path.display()
            );
            println!(
                "{}",
                dim("  local clone: git pull there first; updating rebuilds its current contents")
            );
            Ok(true)
        }
        Source::Registry => Ok(true),
    }
}

fn refresh_setup(executable: &Path) -> Result<()> {
    println!("{}", bold("refreshing the installed setup"));
    let status = std::process::Command::new(executable)
        .args(["setup", "--force"])
        .status()
        .context("running setup from the updated binary")?;
    anyhow::ensure!(
        status.success(),
        "binary updated, but setup failed; run {} setup --force",
        executable.display()
    );
    Ok(())
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

/// `VACUUM INTO` is consistent even while another agent is writing.
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
