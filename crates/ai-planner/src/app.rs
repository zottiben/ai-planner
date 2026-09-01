use std::path::{Path, PathBuf};

use ai_planner_core::{default_db_path, GitContext, Plan, Repo, Store};
use anyhow::{Context, Result};

/// Everything a command needs: the database, and where the caller is standing.
pub struct App {
    pub store: Store,
    pub git: Option<GitContext>,
    pub repo: Option<Repo>,
    pub json: bool,
}

impl App {
    pub fn open(db: Option<&Path>, cwd: &Path, json: bool) -> Result<App> {
        let path = db.map(Path::to_path_buf).unwrap_or_else(default_db_path);
        let store = Store::open(&path).with_context(|| format!("opening {}", path.display()))?;
        let git = GitContext::detect(cwd).ok();
        let repo = match &git {
            Some(ctx) => store.find_repo(&ctx.repo_key)?,
            None => None,
        };
        Ok(App {
            store,
            git,
            repo,
            json,
        })
    }

    pub fn repo_id(&self) -> Option<i64> {
        self.repo.as_ref().map(|r| r.id)
    }

    /// The repo the caller is standing in, or a clear instruction when they are not
    /// standing in a registered one.
    pub fn require_repo(&self) -> Result<&Repo> {
        match (&self.repo, &self.git) {
            (Some(repo), _) => Ok(repo),
            (None, Some(git)) => anyhow::bail!(
                "{} is not registered yet - run `aip init` here first",
                git.repo_key
            ),
            (None, None) => {
                anyhow::bail!("not inside a git repository - pass --repo or cd into one")
            }
        }
    }

    pub fn require_git(&self) -> Result<&GitContext> {
        self.git
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("not inside a git repository"))
    }

    /// Which plan this command acts on. Every command shares the one cascade, so
    /// `aip log "..."` in a worktree lands where `aip status` said it would.
    pub fn plan(&self, explicit: Option<&str>) -> Result<Plan> {
        match self.store.resolve(self.git.as_ref(), explicit)? {
            Ok(resolution) => Ok(resolution.plan),
            Err(unresolved) => {
                let hint = if unresolved.candidates.is_empty() {
                    " - `aip new \"<title>\"` starts one".to_string()
                } else {
                    format!(
                        " - pass -p with one of: {}",
                        unresolved
                            .candidates
                            .iter()
                            .map(|p| p.slug.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                anyhow::bail!("{}{hint}", unresolved.reason)
            }
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.store.path().to_path_buf()
    }
}
