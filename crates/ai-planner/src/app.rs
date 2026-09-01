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

    /// Resolve `--plan`, falling back to the resolution cascade when it is absent.
    /// PR1 implements the explicit half; PR2 adds the branch and affinity rules.
    pub fn plan(&self, explicit: Option<&str>) -> Result<Plan> {
        let needle = explicit
            .map(str::to_string)
            .or_else(|| std::env::var("AI_PLANNER_PLAN").ok())
            .filter(|s| !s.trim().is_empty());

        if let Some(needle) = needle {
            return Ok(self.store.find_plan(&needle, self.repo_id())?);
        }

        if let Some(repo) = &self.repo {
            let active = self.store.list_plans(&ai_planner_core::PlanFilter {
                repo_id: Some(repo.id),
                statuses: ai_planner_core::Status::INCOMPLETE.to_vec(),
                query: None,
            })?;
            if active.len() == 1 {
                return Ok(active.into_iter().next().unwrap());
            }
            if active.is_empty() {
                anyhow::bail!(
                    "no unfinished plan in {} - pass --plan, or `aip new` to start one",
                    repo.name
                );
            }
            let names = active
                .iter()
                .map(|p| p.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "{} unfinished plans here ({names}) - pass --plan",
                active.len()
            );
        }

        anyhow::bail!("pass --plan (not inside a registered repo)")
    }

    pub fn db_path(&self) -> PathBuf {
        self.store.path().to_path_buf()
    }
}
