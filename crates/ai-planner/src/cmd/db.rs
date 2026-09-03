use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::app::App;
use crate::cli::DbCmd;
use crate::out::{dim, ok, Table};

pub fn run(app: &App, cmd: &DbCmd) -> Result<()> {
    match cmd {
        DbCmd::Path => {
            println!("{}", app.db_path().display());
            Ok(())
        }
        DbCmd::Open => open(app),
        DbCmd::Backup { out } => backup(app, out.as_deref()),
        DbCmd::Status => status(app),
    }
}

/// Hands the file to the OS. On macOS a `.db` opens in TablePlus when it is the
/// registered handler, which is the point of keeping one database on disk.
fn open(app: &App) -> Result<()> {
    let path = app.db_path();
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(opener)
        .arg(&path)
        .status()
        .with_context(|| format!("running {opener}"))?;
    if !status.success() {
        anyhow::bail!(
            "{opener} could not open {} - open it by hand",
            path.display()
        );
    }
    ok(&format!("opened {}", path.display()));
    Ok(())
}

fn backup(app: &App, out: Option<&std::path::Path>) -> Result<()> {
    let src = app.db_path();
    let dest = match out {
        Some(p) => p.to_path_buf(),
        None => {
            let stamp = ai_planner_core::util::now()
                .replace([':', '-'], "")
                .replace('Z', "");
            let name = format!(
                "{}.{stamp}.bak",
                src.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("planner.db")
            );
            src.with_file_name(name)
        }
    };
    app.store
        .backup(&dest)
        .with_context(|| format!("backing up to {}", dest.display()))?;
    ok(&format!("backed up to {}", dest.display()));
    Ok(())
}

fn status(app: &App) -> Result<()> {
    let db = app.store.db();
    let path: PathBuf = app.db_path();
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    println!("{}", path.display());
    println!(
        "{}",
        dim(&format!(
            "schema v{} · {} pending · {:.1} KiB",
            db.schema_version()?,
            db.pending_migrations()?,
            bytes as f64 / 1024.0
        ))
    );
    println!();

    let mut table = Table::new(&["TABLE", "ROWS"]);
    for name in [
        "repo",
        "plan",
        "plan_section",
        "decision",
        "slice",
        "question",
        "gotcha",
        "log",
        "handoff",
        "plan_affinity",
        "plan_import",
    ] {
        let count: i64 = db
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM {name}"), [], |r| r.get(0))?;
        table.row(vec![name.to_string(), count.to_string()]);
    }
    table.print();
    Ok(())
}
