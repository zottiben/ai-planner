mod app;
mod cli;
mod cmd;
mod out;

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

use app::App;
use cli::{Cli, Command};

fn main() {
    if let Err(err) = run() {
        eprintln!("{} {err:#}", red("error:"));
        std::process::exit(1);
    }
}

fn red(s: &str) -> String {
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        format!("\x1b[31m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let cwd = match &cli.cwd {
        Some(dir) => dir.clone(),
        None => std::env::current_dir().context("reading the working directory")?,
    };
    let plan_ref = cli.plan.as_deref();

    // `init` is the one command that may create the database.
    if let Command::Init(args) = &cli.command {
        return cmd::plan::init(cli.db.as_deref(), &cwd, args);
    }

    let mut app = App::open(cli.db.as_deref(), &cwd, cli.json)?;

    match &cli.command {
        Command::Init(_) => unreachable!("handled above"),
        Command::New(args) => cmd::plan::new(&mut app, args),
        Command::Current(args) => cmd::context::current(&mut app, args, plan_ref),
        Command::Status(args) => cmd::context::status(&mut app, args, plan_ref),
        Command::Ls(args) => cmd::plan::ls(&app, args),
        Command::Show(args) => cmd::plan::show(&app, args, plan_ref),
        Command::Set(args) => cmd::plan::set(&mut app, args, plan_ref),
        Command::Edit(args) => cmd::plan::edit(&mut app, args, plan_ref),
        Command::Section(args) => cmd::plan::section(&mut app, args, plan_ref),
        Command::Source(args) => cmd::plan::source(&mut app, args, plan_ref),
        Command::Slice(c) => cmd::slice::run(&mut app, c, plan_ref),
        Command::Log(args) => cmd::note::log(&mut app, args, plan_ref),
        Command::Logs(args) => cmd::note::logs(&app, args, plan_ref),
        Command::Decision(c) => cmd::note::decision(&mut app, c, plan_ref),
        Command::Question(c) => cmd::note::question(&mut app, c, plan_ref),
        Command::Gotcha(c) => cmd::note::gotcha(&mut app, c, plan_ref),
        Command::Import(args) => cmd::import::import(&mut app, args),
        Command::Export(args) => cmd::import::export(&app, args, plan_ref),
        Command::Repos => cmd::plan::repos(&app),
        Command::Db(c) => cmd::db::run(&app, c),
    }
}

/// Bodies come from an argument, a file, or stdin. Long markdown on a command line is
/// unpleasant, and agents pipe more often than they quote.
pub fn read_body(inline: Option<&str>, file: Option<&Path>) -> Result<String> {
    if let Some(text) = inline {
        return Ok(text.to_string());
    }
    if let Some(path) = file {
        let path: PathBuf = path.to_path_buf();
        return std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()));
    }
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        anyhow::bail!("nothing to read - pass the text, --file, or pipe it in");
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    Ok(buf)
}
