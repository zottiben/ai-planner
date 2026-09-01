use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
use crate::util::normalise_remote;

/// Where a command is running, from git's point of view. The three facts below are
/// identical from every worktree of a repo except `worktree` and `branch`, which is
/// exactly the split this tool needs (D2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitContext {
    /// Stable repo key: `github.com/acme/widget`, or the main
    /// checkout's absolute path when there is no remote.
    pub repo_key: String,
    /// Last path segment of the key: `widget`.
    pub repo_name: String,
    pub remote_url: Option<String>,
    /// The main checkout, i.e. the parent of `--git-common-dir`.
    pub main_path: PathBuf,
    /// This checkout. Differs per worktree.
    pub worktree: PathBuf,
    /// `None` on a detached HEAD.
    pub branch: Option<String>,
    pub head_sha: Option<String>,
}

impl GitContext {
    pub fn detect(from: &Path) -> Result<Self> {
        let toplevel = git(from, &["rev-parse", "--show-toplevel"])?
            .ok_or_else(|| Error::NotAGitRepo(from.to_path_buf()))?;
        let worktree = PathBuf::from(&toplevel);

        let common_dir = git(
            from,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?
        .ok_or_else(|| Error::NotAGitRepo(from.to_path_buf()))?;
        let main_path = PathBuf::from(&common_dir)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| worktree.clone());

        let remote_url = git(from, &["remote", "get-url", "origin"])?.or(
            // A repo with remotes but no `origin` still has a stable identity.
            git(from, &["remote"])?
                .and_then(|list| list.lines().next().map(str::to_string))
                .and_then(|name| git(from, &["remote", "get-url", &name]).ok().flatten()),
        );

        let repo_key = match &remote_url {
            Some(url) => normalise_remote(url),
            None => main_path.to_string_lossy().to_string(),
        };
        let repo_name = repo_key
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or(&repo_key)
            .to_string();

        let branch = git(from, &["branch", "--show-current"])?.filter(|b| !b.is_empty());
        let head_sha = git(from, &["rev-parse", "--short", "HEAD"])?;

        Ok(GitContext {
            repo_key,
            repo_name,
            remote_url,
            main_path,
            worktree,
            branch,
            head_sha,
        })
    }

    pub fn worktree_str(&self) -> String {
        self.worktree.to_string_lossy().to_string()
    }

    /// True when this checkout is not the main one, i.e. a linked worktree.
    pub fn is_linked_worktree(&self) -> bool {
        self.worktree != self.main_path
    }

    pub fn dirty(&self) -> bool {
        git(&self.worktree, &["status", "--porcelain"])
            .ok()
            .flatten()
            .is_some_and(|s| !s.trim().is_empty())
    }
}

/// Run git and return trimmed stdout, or `None` when git exits non-zero (which for
/// these queries means "no such thing", not a failure worth propagating).
fn git(cwd: &Path, args: &[&str]) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| Error::Git(e.to_string()))?;
    if !out.status.success() {
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if s.is_empty() { None } else { Some(s) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        run(dir, &["init", "-q", "-b", "main"]);
        run(dir, &["config", "user.email", "t@example.com"]);
        run(dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "hi").unwrap();
        run(dir, &["add", "."]);
        run(dir, &["commit", "-qm", "init"]);
    }

    #[test]
    fn a_worktree_reports_the_main_checkout_and_the_same_repo_key() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        run(
            &main,
            &["remote", "add", "origin", "git@github.com:acme/widget.git"],
        );

        let linked = tmp.path().join("wt1");
        run(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat/x",
                linked.to_str().unwrap(),
            ],
        );

        let a = GitContext::detect(&main).unwrap();
        let b = GitContext::detect(&linked).unwrap();

        assert_eq!(a.repo_key, "github.com/acme/widget");
        assert_eq!(b.repo_key, a.repo_key);
        assert_eq!(b.repo_name, "widget");
        // Both agree on the main checkout, which is what makes one database work
        // across every worktree.
        assert_eq!(
            a.main_path.canonicalize().unwrap(),
            b.main_path.canonicalize().unwrap()
        );
        assert_ne!(a.worktree, b.worktree);
        assert_eq!(a.branch.as_deref(), Some("main"));
        assert_eq!(b.branch.as_deref(), Some("feat/x"));
        assert!(!a.is_linked_worktree());
        assert!(b.is_linked_worktree());
    }

    #[test]
    fn a_remoteless_repo_falls_back_to_the_main_checkout_path() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("solo");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);

        let ctx = GitContext::detect(&main).unwrap();
        assert!(ctx.remote_url.is_none());
        assert_eq!(ctx.repo_key, ctx.main_path.to_string_lossy());
        assert_eq!(ctx.repo_name, "solo");
    }

    #[test]
    fn outside_a_repo_is_an_error_not_a_guess() {
        let tmp = tempfile::tempdir().unwrap();
        // /tmp itself is not a repo on macOS or Linux CI.
        let err = GitContext::detect(tmp.path());
        assert!(matches!(err, Err(Error::NotAGitRepo(_))), "{err:?}");
    }
}
