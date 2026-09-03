//! Reconciling the plan against what git and GitHub can already see.
//!
//! The mechanical facts about a slice - its branch merged, its PR opened or closed -
//! are observable, so the database should not depend on an agent remembering to record
//! them. Everything here is derived from `git` and `gh`; the judgement calls (progress
//! notes, decisions, gotchas) still have to be written by whoever did the work, and
//! those are what the hooks nudge about.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::git::{self, GitContext};
use crate::model::{LogKind, Slice, Status};
use crate::store::{NewLog, SliceUpdate, Store};
use crate::util::sha256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Finding {
    /// The branch has landed on the default branch, so the slice is done.
    Merged {
        slice: String,
        branch: String,
        into: String,
    },
    /// GitHub says the PR merged.
    PrMerged {
        slice: String,
        url: String,
        number: i64,
    },
    /// A PR is open, so the slice is in review.
    PrOpen {
        slice: String,
        url: String,
        number: i64,
    },
    /// The slice records no PR link but GitHub has one.
    PrLinkMissing {
        slice: String,
        url: String,
        number: i64,
    },
    /// The branch is gone from local and remote, but the slice still holds a claim.
    BranchGone { slice: String, branch: String },
    /// Claimed in a worktree that is no longer on disk.
    WorktreeGone { slice: String, worktree: String },
}

impl Finding {
    pub fn slice(&self) -> &str {
        match self {
            Finding::Merged { slice, .. }
            | Finding::PrMerged { slice, .. }
            | Finding::PrOpen { slice, .. }
            | Finding::PrLinkMissing { slice, .. }
            | Finding::BranchGone { slice, .. }
            | Finding::WorktreeGone { slice, .. } => slice,
        }
    }

    /// One line, written for an agent to act on.
    pub fn describe(&self) -> String {
        match self {
            Finding::Merged {
                slice,
                branch,
                into,
            } => {
                format!(
                    "{slice}: `{branch}` has landed on `{into}` - the plan still says otherwise"
                )
            }
            Finding::PrMerged { slice, number, .. } => {
                format!("{slice}: PR #{number} is merged - the plan still says otherwise")
            }
            Finding::PrOpen { slice, number, .. } => {
                format!("{slice}: PR #{number} is open - the plan does not say it is in review")
            }
            Finding::PrLinkMissing { slice, number, .. } => {
                format!("{slice}: PR #{number} exists but the slice has no PR link")
            }
            Finding::BranchGone { slice, branch } => {
                format!("{slice}: `{branch}` no longer exists but the slice is still claimed")
            }
            Finding::WorktreeGone { slice, worktree } => {
                format!("{slice}: claimed in {worktree}, which is gone")
            }
        }
    }

    /// What `aip sync --fix` would do about it.
    pub fn remedy(&self) -> String {
        match self {
            Finding::Merged { slice, .. } | Finding::PrMerged { slice, .. } => {
                format!("aip slice set {slice} done")
            }
            Finding::PrOpen { slice, .. } => format!("aip slice set {slice} in_review"),
            Finding::PrLinkMissing { slice, url, .. } => {
                format!("aip slice edit {slice} --pr {url}")
            }
            Finding::BranchGone { slice, .. } | Finding::WorktreeGone { slice, .. } => {
                format!("aip slice release {slice}")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct FixReport {
    pub applied: usize,
    pub skipped: usize,
}

impl Store {
    /// Compare every slice of a plan against git, and against GitHub when `gh` is
    /// available. Read-only.
    pub fn drift(&self, plan_id: i64, git: &GitContext, use_gh: bool) -> Result<Vec<Finding>> {
        let default = git.default_branch();
        let mut findings = Vec::new();

        for slice in self.slices(plan_id)? {
            if let Some(worktree) = &slice.worktree_path {
                if slice.claimed_by.is_some()
                    && !slice.status.is_terminal()
                    && !std::path::Path::new(worktree).exists()
                {
                    findings.push(Finding::WorktreeGone {
                        slice: slice.key.clone(),
                        worktree: worktree.clone(),
                    });
                }
            }

            let Some(branch) = slice.branch.clone().filter(|b| !b.trim().is_empty()) else {
                continue;
            };

            // GitHub is the better witness when it is reachable: it knows a squash
            // merge landed even though the branch's commits never appear in history.
            let pr = if use_gh {
                git::pull_request_for(&git.worktree, &branch)
            } else {
                None
            };

            if let Some(pr) = &pr {
                if slice.pr_url.as_deref() != Some(pr.url.as_str()) {
                    findings.push(Finding::PrLinkMissing {
                        slice: slice.key.clone(),
                        url: pr.url.clone(),
                        number: pr.number,
                    });
                }
                if pr.merged() && slice.status != Status::Done {
                    findings.push(Finding::PrMerged {
                        slice: slice.key.clone(),
                        url: pr.url.clone(),
                        number: pr.number,
                    });
                    continue;
                }
                if pr.open() && !matches!(slice.status, Status::InReview | Status::Done) {
                    findings.push(Finding::PrOpen {
                        slice: slice.key.clone(),
                        url: pr.url.clone(),
                        number: pr.number,
                    });
                    continue;
                }
            }

            // A squash merge leaves the branch unmerged by ancestry, so this only adds
            // a finding when git can prove it landed - never when it merely cannot tell.
            if let Some(into) = &default {
                if &branch != into
                    && slice.status != Status::Done
                    && git.is_merged_into(&branch, into)
                {
                    findings.push(Finding::Merged {
                        slice: slice.key.clone(),
                        branch: branch.clone(),
                        into: into.clone(),
                    });
                    continue;
                }
            }

            if slice.claimed_by.is_some()
                && !slice.status.is_terminal()
                && !git.branch_exists(&branch)
            {
                findings.push(Finding::BranchGone {
                    slice: slice.key.clone(),
                    branch,
                });
            }
        }

        Ok(findings)
    }

    /// Apply what the findings imply. Only ever moves a slice forward - it will mark a
    /// slice done or in review, never reopen one - so a stale remote cannot undo work
    /// somebody has already recorded.
    pub fn apply_drift(&mut self, plan_id: i64, findings: &[Finding]) -> Result<FixReport> {
        let mut report = FixReport::default();
        for finding in findings {
            let Some(slice) = self.slice(plan_id, finding.slice())? else {
                report.skipped += 1;
                continue;
            };
            let acted = match finding {
                Finding::Merged { .. } | Finding::PrMerged { .. } => {
                    self.set_slice_status(&slice, Status::Done, None)?;
                    true
                }
                Finding::PrOpen { .. } => {
                    self.set_slice_status(&slice, Status::InReview, None)?;
                    true
                }
                Finding::PrLinkMissing { url, .. } => {
                    self.update_slice(
                        &slice,
                        SliceUpdate {
                            pr_url: Some(url.clone()),
                            ..Default::default()
                        },
                    )?;
                    true
                }
                Finding::BranchGone { .. } | Finding::WorktreeGone { .. } => {
                    self.release_slice(&slice)?;
                    true
                }
            };
            if acted {
                report.applied += 1;
            } else {
                report.skipped += 1;
            }
        }

        if report.applied > 0 {
            self.append_log(NewLog {
                plan_id,
                kind: Some(LogKind::Status),
                body: format!(
                    "aip sync reconciled {} slice(s) from git: {}",
                    report.applied,
                    findings
                        .iter()
                        .map(|f| f.slice())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                ..Default::default()
            })?;
        }
        Ok(report)
    }

    /// Has this exact complaint already been made in this worktree? Recorded when it
    /// has, so a per-turn hook says its piece once and only speaks again when the
    /// underlying state changes.
    pub fn take_nudge(&mut self, worktree: &str, kind: &str, body: &str) -> Result<bool> {
        let fingerprint = sha256(body.as_bytes());
        let (worktree, kind) = (worktree.to_string(), kind.to_string());
        self.db_mut().write(|tx| {
            let changed = tx.execute(
                "INSERT INTO nudge (worktree_path, kind, fingerprint, at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(worktree_path, kind, fingerprint) DO NOTHING",
                rusqlite::params![worktree, kind, fingerprint, crate::util::now()],
            )?;
            Ok(changed == 1)
        })
    }

    /// Slices this worktree holds that have had no progress note since they were
    /// claimed. The one drift the agent alone can fix.
    pub fn unlogged_claims(&self, plan_id: i64, worktree: &str) -> Result<Vec<Slice>> {
        let mut out = Vec::new();
        for slice in self.slices(plan_id)? {
            if slice.claimed_by.is_none() || slice.status.is_terminal() {
                continue;
            }
            if slice.worktree_path.as_deref() != Some(worktree) {
                continue;
            }
            let since = slice.claimed_at.clone().unwrap_or_default();
            let notes: i64 = self.db().conn().query_row(
                "SELECT COUNT(*) FROM log
                 WHERE slice_id = ?1 AND kind IN ('progress','verification','blocker')
                   AND at >= ?2",
                rusqlite::params![slice.id, since],
                |r| r.get(0),
            )?;
            if notes == 0 {
                out.push(slice);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_says_what_is_wrong_and_what_fixes_it() {
        let f = Finding::PrOpen {
            slice: "PR2".into(),
            url: "https://github.com/acme/widget/pull/412".into(),
            number: 412,
        };
        assert!(f.describe().contains("PR #412 is open"));
        assert_eq!(f.remedy(), "aip slice set PR2 in_review");
        assert_eq!(f.slice(), "PR2");
    }
}
