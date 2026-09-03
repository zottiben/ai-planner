//! Reconciliation has to be driven by real git state, so these tests build actual
//! repos and merge actual branches. `gh` is never invoked: branch history alone must
//! be enough, because a hook cannot make a network call on every turn.

use std::path::Path;
use std::process::Command;

use ai_planner_core::drift::Finding;
use ai_planner_core::store::{NewPlan, NewSlice};
use ai_planner_core::{GitContext, Status, Store};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fx {
    _dir: tempfile::TempDir,
    store: Store,
    ctx: GitContext,
    plan_id: i64,
}

/// A repo with a `main` branch and one commit, plus a plan with one slice per branch
/// the caller asks for.
fn fixture(branches: &[&str]) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("widget");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("f.txt"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);

    for branch in branches {
        git(&repo, &["checkout", "-q", "-b", branch]);
        std::fs::write(
            repo.join(format!("{}.txt", branch.replace('/', "-"))),
            "work",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", &format!("work on {branch}")]);
        git(&repo, &["checkout", "-q", "main"]);
    }

    let mut store = Store::init(&dir.path().join("planner.db")).unwrap();
    let ctx = GitContext::detect(&repo).unwrap();
    let repo_id = store.ensure_repo(&ctx).unwrap().id;
    let plan = store
        .create_plan(NewPlan {
            repo_id,
            title: "ACME-1234 - Picker".into(),
            status: Some(Status::Active),
            ..Default::default()
        })
        .unwrap();
    for (i, branch) in branches.iter().enumerate() {
        store
            .add_slice(NewSlice {
                plan_id: plan.id,
                key: format!("PR{}", i + 1),
                title: format!("slice for {branch}"),
                branch: Some(branch.to_string()),
                ..Default::default()
            })
            .unwrap();
    }

    Fx {
        _dir: dir,
        store,
        ctx,
        plan_id: plan.id,
    }
}

#[test]
fn a_merged_branch_is_noticed_and_the_slice_is_marked_done() {
    let mut fx = fixture(&["feat/one", "feat/two"]);

    // Nothing has landed yet, so nothing is out of sync.
    assert!(fx
        .store
        .drift(fx.plan_id, &fx.ctx, false)
        .unwrap()
        .is_empty());

    git(
        &fx.ctx.worktree,
        &["merge", "-q", "--no-ff", "-m", "merge one", "feat/one"],
    );

    let findings = fx.store.drift(fx.plan_id, &fx.ctx, false).unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    match &findings[0] {
        Finding::Merged {
            slice,
            branch,
            into,
        } => {
            assert_eq!(slice, "PR1");
            assert_eq!(branch, "feat/one");
            assert_eq!(into, "main");
        }
        other => panic!("expected a merge finding, got {other:?}"),
    }
    assert!(findings[0].describe().contains("has landed"));
    assert_eq!(findings[0].remedy(), "aip slice set PR1 done");

    let report = fx.store.apply_drift(fx.plan_id, &findings).unwrap();
    assert_eq!(report.applied, 1);
    assert_eq!(
        fx.store.require_slice(fx.plan_id, "PR1").unwrap().status,
        Status::Done
    );
    // The untouched slice is left exactly as it was.
    assert_eq!(
        fx.store.require_slice(fx.plan_id, "PR2").unwrap().status,
        Status::Ready
    );

    // Reconciling again finds nothing, so it is safe to run on every turn.
    assert!(fx
        .store
        .drift(fx.plan_id, &fx.ctx, false)
        .unwrap()
        .is_empty());
}

#[test]
fn reconciling_writes_its_own_audit_entry() {
    let mut fx = fixture(&["feat/one"]);
    git(
        &fx.ctx.worktree,
        &["merge", "-q", "--no-ff", "-m", "merge", "feat/one"],
    );

    let findings = fx.store.drift(fx.plan_id, &fx.ctx, false).unwrap();
    fx.store.apply_drift(fx.plan_id, &findings).unwrap();

    let log = fx.store.log(fx.plan_id, None).unwrap();
    assert!(
        log.iter().any(|e| e.body.contains("aip sync reconciled")),
        "the reconciliation must be visible in the log: {log:?}"
    );
}

#[test]
fn a_deleted_branch_releases_a_claim_rather_than_losing_it() {
    let mut fx = fixture(&["feat/gone"]);
    let slice = fx.store.require_slice(fx.plan_id, "PR1").unwrap();
    fx.store
        .claim_slice(&slice, &fx.ctx.worktree_str(), Some("feat/gone"))
        .unwrap();

    // Still there: a claimed slice on a live branch is not drift.
    assert!(fx
        .store
        .drift(fx.plan_id, &fx.ctx, false)
        .unwrap()
        .is_empty());

    git(&fx.ctx.worktree, &["branch", "-q", "-D", "feat/gone"]);

    let findings = fx.store.drift(fx.plan_id, &fx.ctx, false).unwrap();
    assert!(
        matches!(&findings[0], Finding::BranchGone { slice, .. } if slice == "PR1"),
        "{findings:?}"
    );

    fx.store.apply_drift(fx.plan_id, &findings).unwrap();
    let fresh = fx.store.require_slice(fx.plan_id, "PR1").unwrap();
    assert!(fresh.claimed_by.is_none(), "the claim is released");
    // The work itself is not thrown away - only the claim.
    assert_eq!(fresh.status, Status::Active);
}

#[test]
fn a_slice_that_is_already_done_is_left_alone() {
    let mut fx = fixture(&["feat/one"]);
    let slice = fx.store.require_slice(fx.plan_id, "PR1").unwrap();
    fx.store
        .set_slice_status(&slice, Status::Done, None)
        .unwrap();
    git(
        &fx.ctx.worktree,
        &["merge", "-q", "--no-ff", "-m", "merge", "feat/one"],
    );

    // Reconciliation only moves a slice forward, so nothing is reported here.
    assert!(fx
        .store
        .drift(fx.plan_id, &fx.ctx, false)
        .unwrap()
        .is_empty());
}

#[test]
fn the_default_branch_is_read_from_the_repo_not_assumed() {
    // A repo on `master` must not be reconciled against a `main` that does not exist.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("widget");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "master"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("f.txt"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);

    let ctx = GitContext::detect(&repo).unwrap();
    assert_eq!(ctx.default_branch().as_deref(), Some("master"));
    assert!(ctx.branch_exists("master"));
    assert!(!ctx.branch_exists("main"));
}

#[test]
fn a_claim_with_no_progress_note_is_reported_once_per_state() {
    let mut fx = fixture(&["feat/one"]);
    let worktree = fx.ctx.worktree_str();
    let slice = fx.store.require_slice(fx.plan_id, "PR1").unwrap();
    fx.store
        .claim_slice(&slice, &worktree, Some("feat/one"))
        .unwrap();

    let unlogged = fx.store.unlogged_claims(fx.plan_id, &worktree).unwrap();
    assert_eq!(unlogged.len(), 1);
    assert_eq!(unlogged[0].key, "PR1");

    // The nudge is spoken once per distinct state, so a per-turn hook cannot spam.
    assert!(fx
        .store
        .take_nudge(&worktree, "stop", "PR1 has no note")
        .unwrap());
    assert!(!fx
        .store
        .take_nudge(&worktree, "stop", "PR1 has no note")
        .unwrap());
    assert!(fx
        .store
        .take_nudge(&worktree, "stop", "PR2 has no note either")
        .unwrap());

    // Writing the note clears the condition.
    fx.store
        .append_log(ai_planner_core::store::NewLog {
            plan_id: fx.plan_id,
            slice_id: Some(slice.id),
            body: "Built the thing.".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(fx
        .store
        .unlogged_claims(fx.plan_id, &worktree)
        .unwrap()
        .is_empty());
}
