//! Answering "which plan am I on?" from where the caller is standing.
//!
//! The cascade is deterministic and stops at the first hit (D7). Every rule below
//! resolves at least one plan in the sample this was built from, with no model
//! involved. Search is the last resort, and an unresolved lookup returns candidates
//! rather than a guess - a wrong answer here writes progress into the wrong plan.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::git::GitContext;
use crate::model::{Plan, Slice, Status};
use crate::store::{PlanFilter, Store};
use crate::util::ticket_key;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// `--plan`, or `$AI_PLANNER_PLAN`.
    Explicit,
    /// A slice records this exact branch.
    SliceBranch,
    /// Something is claimed in this worktree.
    WorktreeClaim,
    /// The last handoff written from this worktree.
    Handoff,
    /// Learned from previous confirmed resolutions here.
    Affinity,
    /// A ticket key in the branch name matches a plan.
    BranchTicket,
    /// The repo has exactly one unfinished plan.
    OnlyPlan,
}

impl Rule {
    pub fn why(self) -> &'static str {
        match self {
            Rule::Explicit => "you named it",
            Rule::SliceBranch => "a slice records this branch",
            Rule::WorktreeClaim => "a slice is claimed in this worktree",
            Rule::Handoff => "the last handoff from this worktree",
            Rule::Affinity => "this branch resolved here before",
            Rule::BranchTicket => "the branch name carries the ticket key",
            Rule::OnlyPlan => "the only unfinished plan in this repo",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    pub plan: Plan,
    /// The slice this worktree is on, when the rule identified one.
    pub slice: Option<Slice>,
    pub rule: Rule,
}

/// What to say when nothing resolved. Carries the candidates so the caller can offer
/// them instead of inventing an answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unresolved {
    pub reason: String,
    pub candidates: Vec<Plan>,
}

pub type Resolved = std::result::Result<Resolution, Unresolved>;

impl Store {
    /// Run the cascade. `explicit` is whatever the caller passed as `--plan`.
    pub fn resolve(&self, git: Option<&GitContext>, explicit: Option<&str>) -> Result<Resolved> {
        let repo = match git {
            Some(ctx) => self.find_repo(&ctx.repo_key)?,
            None => None,
        };
        let repo_id = repo.as_ref().map(|r| r.id);

        // 1. Named outright.
        let named = explicit
            .map(str::to_string)
            .or_else(|| std::env::var("AI_PLANNER_PLAN").ok())
            .filter(|s| !s.trim().is_empty());
        if let Some(needle) = named {
            let plan = self.find_plan(&needle, repo_id)?;
            let slice = self.slice_for(&plan, git)?;
            return Ok(Ok(Resolution {
                plan,
                slice,
                rule: Rule::Explicit,
            }));
        }

        let (Some(git), Some(repo_id)) = (git, repo_id) else {
            return Ok(Err(Unresolved {
                reason: match git {
                    Some(ctx) => format!("{} is not registered - run `aip init`", ctx.repo_key),
                    None => "not inside a git repository".to_string(),
                },
                candidates: Vec::new(),
            }));
        };
        let worktree = git.worktree_str();

        // 2. A slice names this branch. Exact, and true for most plans already.
        if let Some(branch) = git.branch.as_deref() {
            let on_branch = self.slices_on_branch(repo_id, branch)?;
            if let Some(any) = on_branch.first() {
                // A finished slice still identifies the plan - the branch was used for
                // it - but you are not "on" it, so report the live slice instead.
                let plan = self.get_plan(any.plan_id)?;
                let slice = match on_branch.iter().find(|s| !s.status.is_terminal()) {
                    Some(live) => Some(live.clone()),
                    None => self.slice_for(&plan, Some(git))?,
                };
                return Ok(Ok(Resolution {
                    plan,
                    slice,
                    rule: Rule::SliceBranch,
                }));
            }
        }

        // 3. Something is claimed here.
        if let Some(slice) = self.claimed_in(&worktree)? {
            return Ok(Ok(Resolution {
                plan: self.get_plan(slice.plan_id)?,
                slice: Some(slice),
                rule: Rule::WorktreeClaim,
            }));
        }

        // 4. The last handoff written from here.
        if let Some(plan_id) = self.last_handoff_plan(&worktree)? {
            let plan = self.get_plan(plan_id)?;
            let slice = self.slice_for(&plan, Some(git))?;
            return Ok(Ok(Resolution {
                plan,
                slice,
                rule: Rule::Handoff,
            }));
        }

        // 5. Learned from previous confirmed resolutions.
        if let Some(plan_id) = self.affinity_pick(repo_id, git.branch.as_deref(), &worktree)? {
            let plan = self.get_plan(plan_id)?;
            let slice = self.slice_for(&plan, Some(git))?;
            return Ok(Ok(Resolution {
                plan,
                slice,
                rule: Rule::Affinity,
            }));
        }

        // 6. The branch carries a ticket key.
        if let Some(key) = git.branch.as_deref().and_then(ticket_key) {
            if let Ok(plan) = self.find_plan(&key, Some(repo_id)) {
                let slice = self.slice_for(&plan, Some(git))?;
                return Ok(Ok(Resolution {
                    plan,
                    slice,
                    rule: Rule::BranchTicket,
                }));
            }
        }

        // 7. Only one unfinished plan in the repo.
        let unfinished = self.list_plans(&PlanFilter {
            repo_id: Some(repo_id),
            statuses: Status::INCOMPLETE.to_vec(),
            query: None,
        })?;
        if unfinished.len() == 1 {
            let plan = unfinished.into_iter().next().unwrap();
            let slice = self.slice_for(&plan, Some(git))?;
            return Ok(Ok(Resolution {
                plan,
                slice,
                rule: Rule::OnlyPlan,
            }));
        }

        let reason = if unfinished.is_empty() {
            "no unfinished plan in this repo".to_string()
        } else {
            format!(
                "{} unfinished plans here and nothing ties this worktree to one",
                unfinished.len()
            )
        };
        Ok(Err(Unresolved {
            reason,
            candidates: unfinished,
        }))
    }

    /// The slice within a plan that this worktree is on: claimed here first, then one
    /// matching the branch, then the first unfinished one.
    pub fn slice_for(&self, plan: &Plan, git: Option<&GitContext>) -> Result<Option<Slice>> {
        let slices = self.slices(plan.id)?;
        if let Some(git) = git {
            let worktree = git.worktree_str();
            if let Some(s) = slices.iter().find(|s| {
                s.worktree_path.as_deref() == Some(worktree.as_str()) && s.claimed_by.is_some()
            }) {
                return Ok(Some(s.clone()));
            }
            if let Some(branch) = git.branch.as_deref() {
                if let Some(s) = slices
                    .iter()
                    .find(|s| s.branch.as_deref() == Some(branch) && !s.status.is_terminal())
                {
                    return Ok(Some(s.clone()));
                }
            }
        }
        Ok(slices
            .into_iter()
            .find(|s| matches!(s.status, Status::Active | Status::InReview)))
    }

    /// The next thing to pick up. What this worktree already holds comes first - you
    /// resume your own work before starting anything new - then the first slice that
    /// is ready or going and not held by someone else.
    pub fn next_slice(&self, plan_id: i64, worktree: Option<&str>) -> Result<Option<Slice>> {
        let slices = self.slices(plan_id)?;
        let mine = slices.iter().find(|s| {
            !s.status.is_terminal()
                && s.claimed_by.is_some()
                && s.worktree_path.as_deref() == worktree
        });
        if let Some(s) = mine {
            return Ok(Some(s.clone()));
        }
        Ok(slices
            .into_iter()
            .filter(|s| matches!(s.status, Status::Ready | Status::Active))
            .find(|s| s.claimed_by.is_none()))
    }

    fn claimed_in(&self, worktree: &str) -> Result<Option<Slice>> {
        let slices = self.slices_claimed_in(worktree)?;
        Ok(slices
            .iter()
            .find(|s| !s.status.is_terminal())
            .or_else(|| slices.first())
            .cloned())
    }

    fn last_handoff_plan(&self, worktree: &str) -> Result<Option<i64>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .db()
            .conn()
            .query_row(
                "SELECT plan_id FROM handoff WHERE worktree_path = ?1 ORDER BY at DESC, id DESC LIMIT 1",
                [worktree],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn affinity_pick(
        &self,
        repo_id: i64,
        branch: Option<&str>,
        worktree: &str,
    ) -> Result<Option<i64>> {
        use rusqlite::OptionalExtension;
        // Branch is a stronger signal than worktree: worktrees get reused for
        // unrelated work, branches do not.
        Ok(self
            .db()
            .conn()
            .query_row(
                "SELECT plan_id FROM plan_affinity
                 WHERE repo_id = ?1 AND (branch = ?2 OR worktree_path = ?3)
                 ORDER BY (branch = ?2) DESC, hits DESC, last_at DESC
                 LIMIT 1",
                rusqlite::params![repo_id, branch.unwrap_or(""), worktree],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewPlan, NewSlice};

    struct Fx {
        _dir: tempfile::TempDir,
        store: Store,
        repo_id: i64,
    }

    fn fixture() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(&dir.path().join("p.db")).unwrap();
        let repo_id = store
            .db()
            .conn()
            .query_row(
                "INSERT INTO repo (key, name, created_at)
                 VALUES ('github.com/acme/widget', 'widget', '2026-01-01T00:00:00Z') RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        Fx {
            _dir: dir,
            store,
            repo_id,
        }
    }

    fn ctx(branch: &str, worktree: &str) -> GitContext {
        GitContext {
            repo_key: "github.com/acme/widget".into(),
            repo_name: "widget".into(),
            remote_url: None,
            main_path: "/src/widget".into(),
            worktree: worktree.into(),
            branch: Some(branch.into()),
            head_sha: None,
        }
    }

    fn plan(fx: &mut Fx, title: &str) -> Plan {
        fx.store
            .create_plan(NewPlan {
                repo_id: fx.repo_id,
                title: title.into(),
                status: Some(Status::Active),
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn a_branch_named_by_a_slice_resolves_to_that_slice() {
        let mut fx = fixture();
        let p = plan(&mut fx, "ACME-1234 - Picker");
        plan(&mut fx, "Canvas Editor");
        fx.store
            .add_slice(NewSlice {
                plan_id: p.id,
                key: "PR1".into(),
                title: "Core".into(),
                branch: Some("feat/date-range-picker".into()),
                ..Default::default()
            })
            .unwrap();

        let r = fx
            .store
            .resolve(Some(&ctx("feat/date-range-picker", "/wt/3")), None)
            .unwrap()
            .expect("resolves");
        assert_eq!(r.rule, Rule::SliceBranch);
        assert_eq!(r.plan.id, p.id);
        assert_eq!(r.slice.unwrap().key, "PR1");
    }

    #[test]
    fn a_finished_slice_still_names_the_plan_but_is_not_reported_as_current() {
        let mut fx = fixture();
        let p = plan(&mut fx, "ACME-1234 - Picker");
        plan(&mut fx, "Canvas Editor");
        let pr1 = fx
            .store
            .add_slice(NewSlice {
                plan_id: p.id,
                key: "PR1".into(),
                title: "Core".into(),
                branch: Some("feat/date-range-picker".into()),
                ..Default::default()
            })
            .unwrap();
        fx.store.set_slice_status(&pr1, Status::Done, None).unwrap();

        let r = fx
            .store
            .resolve(Some(&ctx("feat/date-range-picker", "/wt/3")), None)
            .unwrap()
            .expect("resolves");
        assert_eq!(r.rule, Rule::SliceBranch);
        assert_eq!(r.plan.id, p.id);
        assert!(r.slice.is_none(), "a done slice is not what you are on");
    }

    #[test]
    fn a_ticket_key_in_the_branch_resolves_without_any_slice() {
        let mut fx = fixture();
        let p = plan(&mut fx, "ACME-1234 - Picker");
        plan(&mut fx, "Canvas Editor");

        let r = fx
            .store
            .resolve(Some(&ctx("feature/acme-1234-csv-export", "/wt/4")), None)
            .unwrap()
            .expect("resolves");
        assert_eq!(r.rule, Rule::BranchTicket);
        assert_eq!(r.plan.id, p.id);
    }

    #[test]
    fn a_claim_wins_over_a_ticket_key_in_the_branch() {
        let mut fx = fixture();
        let picker = plan(&mut fx, "ACME-1234 - Picker");
        let canvas = plan(&mut fx, "Canvas Editor");
        let slice = fx
            .store
            .add_slice(NewSlice {
                plan_id: canvas.id,
                key: "S2".into(),
                title: "Add entities".into(),
                ..Default::default()
            })
            .unwrap();
        fx.store.claim_slice(&slice, "/wt/4", None).unwrap();

        // The branch says ACME-1234, but this worktree is demonstrably working on
        // Canvas Editor. What is actually happening beats what the name suggests.
        let r = fx
            .store
            .resolve(Some(&ctx("feature/acme-1234-csv-export", "/wt/4")), None)
            .unwrap()
            .expect("resolves");
        assert_eq!(r.rule, Rule::WorktreeClaim);
        assert_eq!(r.plan.id, canvas.id);
        assert_ne!(r.plan.id, picker.id);
    }

    #[test]
    fn a_lone_unfinished_plan_resolves_with_nothing_else_to_go_on() {
        let mut fx = fixture();
        let p = plan(&mut fx, "Accounts V2");

        let r = fx
            .store
            .resolve(Some(&ctx("some/unrelated-branch", "/wt/1")), None)
            .unwrap()
            .expect("resolves");
        assert_eq!(r.rule, Rule::OnlyPlan);
        assert_eq!(r.plan.id, p.id);
    }

    #[test]
    fn ambiguity_returns_candidates_instead_of_guessing() {
        let mut fx = fixture();
        plan(&mut fx, "Accounts V2");
        plan(&mut fx, "Canvas Editor");

        let unresolved = fx
            .store
            .resolve(Some(&ctx("some/unrelated-branch", "/wt/1")), None)
            .unwrap()
            .expect_err("must not guess");
        assert_eq!(unresolved.candidates.len(), 2);
        assert!(unresolved.reason.contains("2 unfinished plans"));
    }

    #[test]
    fn affinity_remembers_a_branch_that_was_resolved_by_hand() {
        let mut fx = fixture();
        plan(&mut fx, "Accounts V2");
        let canvas = plan(&mut fx, "Canvas Editor");

        // Two candidates, so nothing resolves on its own.
        assert!(fx
            .store
            .resolve(Some(&ctx("feat/canvas", "/wt/2")), None)
            .unwrap()
            .is_err());

        // Naming it once teaches the association.
        let named = fx
            .store
            .resolve(Some(&ctx("feat/canvas", "/wt/2")), Some("canvas-editor"))
            .unwrap()
            .expect("resolves");
        assert_eq!(named.rule, Rule::Explicit);
        fx.store
            .record_affinity(canvas.id, fx.repo_id, Some("feat/canvas"), "/wt/2")
            .unwrap();

        let learned = fx
            .store
            .resolve(Some(&ctx("feat/canvas", "/wt/2")), None)
            .unwrap()
            .expect("resolves");
        assert_eq!(learned.rule, Rule::Affinity);
        assert_eq!(learned.plan.id, canvas.id);
    }

    #[test]
    fn the_next_slice_skips_what_another_worktree_holds() {
        let mut fx = fixture();
        let p = plan(&mut fx, "Picker");
        for key in ["PR1", "PR2", "PR3"] {
            fx.store
                .add_slice(NewSlice {
                    plan_id: p.id,
                    key: key.into(),
                    title: key.into(),
                    ..Default::default()
                })
                .unwrap();
        }
        let pr1 = fx.store.require_slice(p.id, "PR1").unwrap();
        fx.store.set_slice_status(&pr1, Status::Done, None).unwrap();
        let pr2 = fx.store.require_slice(p.id, "PR2").unwrap();
        fx.store.claim_slice(&pr2, "/wt/9", None).unwrap();

        let next = fx.store.next_slice(p.id, Some("/wt/3")).unwrap().unwrap();
        assert_eq!(next.key, "PR3");

        // From the worktree that holds PR2, PR2 is what you resume - even though PR3
        // is unclaimed and PR1 came first.
        let mine = fx.store.next_slice(p.id, Some("/wt/9")).unwrap().unwrap();
        assert_eq!(mine.key, "PR2");
    }
}
