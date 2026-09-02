//! Behaviour that only shows up against a real file: concurrent writers, claim
//! races, and the conflict check. These are the guarantees that make parallel
//! worktrees safe (D4, D6).

use std::path::{Path, PathBuf};

use ai_planner_core::model::{DecisionStatus, Renders};
use ai_planner_core::store::{NewLog, NewPlan, NewSlice, SectionWrite};
use ai_planner_core::{Error, Status, Store};

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    repo_id: i64,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("planner.db");
        let store = Store::init(&path).unwrap();
        let repo_id = store
            .db()
            .conn()
            .query_row(
                "INSERT INTO repo (key, name, created_at)
                 VALUES ('github.com/acme/widget', 'widget', '2026-01-01T00:00:00Z')
                 RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        Fixture {
            _dir: dir,
            path,
            repo_id,
        }
    }

    fn store(&self) -> Store {
        Store::open(&self.path).unwrap()
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn seed_plan(store: &mut Store, repo_id: i64, title: &str) -> ai_planner_core::Plan {
    store
        .create_plan(NewPlan {
            repo_id,
            title: title.to_string(),
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn a_new_plan_is_keyed_by_its_ticket_and_starts_with_the_usual_spine() {
    let fx = Fixture::new();
    let mut store = fx.store();

    let plan = seed_plan(
        &mut store,
        fx.repo_id,
        "ACME-1234 - Reusable Date Range Picker",
    );

    assert_eq!(plan.slug, "acme-1234");
    assert_eq!(plan.ticket_key.as_deref(), Some("ACME-1234"));
    assert_eq!(plan.status, Status::Draft);

    let sections = store.sections(plan.id).unwrap();
    let keys: Vec<&str> = sections.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "outcome",
            "grounding",
            "sources",
            "decisions",
            "slices",
            "questions",
            "gotchas",
            "log"
        ]
    );
    assert_eq!(sections[4].renders, Renders::Slices);
}

#[test]
fn a_plan_is_findable_by_slug_ticket_id_and_title() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(
        &mut store,
        fx.repo_id,
        "ACME-1234 - Reusable Date Range Picker",
    );
    seed_plan(&mut store, fx.repo_id, "Canvas Editor");

    for needle in [
        "acme-1234",
        "ACME-1234",
        "ACME-1234 - Reusable Date Range Picker",
        "date range",
    ] {
        let found = store.find_plan(needle, Some(fx.repo_id)).unwrap();
        assert_eq!(found.id, plan.id, "looking up {needle:?}");
    }
    assert_eq!(
        store
            .find_plan(&plan.id.to_string(), Some(fx.repo_id))
            .unwrap()
            .id,
        plan.id
    );
}

#[test]
fn an_ambiguous_reference_is_an_error_rather_than_a_guess() {
    let fx = Fixture::new();
    let mut store = fx.store();
    seed_plan(&mut store, fx.repo_id, "Accounts V2 dashboard");
    seed_plan(&mut store, fx.repo_id, "Accounts V2 exports");

    let err = store.find_plan("accounts v2", Some(fx.repo_id));
    assert!(matches!(err, Err(Error::AmbiguousPlan(_, 2, _))), "{err:?}");
}

#[test]
fn two_plans_cannot_share_a_slug_in_one_repo() {
    let fx = Fixture::new();
    let mut store = fx.store();
    seed_plan(&mut store, fx.repo_id, "ACME-1234 - Picker");
    let err = store.create_plan(NewPlan {
        repo_id: fx.repo_id,
        title: "ACME-1234 - Picker again".into(),
        ..Default::default()
    });
    assert!(matches!(err, Err(Error::DuplicatePlan(_))), "{err:?}");
}

#[test]
fn finishing_a_slice_stamps_it_and_writes_its_own_history() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");
    let slice = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR1".into(),
            title: "Shared core".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(slice.status, Status::Ready);

    let active = store
        .set_slice_status(&slice, Status::Active, None)
        .unwrap();
    assert!(active.started_at.is_some());
    assert!(active.completed_at.is_none());

    let done = store.set_slice_status(&active, Status::Done, None).unwrap();
    assert!(done.completed_at.is_some());

    let history = store.slice_log(done.id, None).unwrap();
    let bodies: Vec<&str> = history.iter().map(|l| l.body.as_str()).collect();
    assert!(bodies.contains(&"PR1 ready -> active"), "{bodies:?}");
    assert!(bodies.contains(&"PR1 active -> done"), "{bodies:?}");
}

#[test]
fn reopening_a_done_slice_clears_the_completion_stamp() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");
    let slice = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR1".into(),
            title: "Shared core".into(),
            ..Default::default()
        })
        .unwrap();
    let done = store.set_slice_status(&slice, Status::Done, None).unwrap();
    let reopened = store.set_slice_status(&done, Status::Active, None).unwrap();
    assert!(reopened.completed_at.is_none());
}

#[test]
fn blocking_a_slice_keeps_the_reason_and_dropping_the_block_clears_it() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Canvas Editor");
    let slice = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "S7".into(),
            title: "Shared canvas for a group".into(),
            ..Default::default()
        })
        .unwrap();

    let blocked = store
        .set_slice_status(
            &slice,
            Status::Blocked,
            Some("waiting on the upstream project"),
        )
        .unwrap();
    assert_eq!(
        blocked.blocked_reason.as_deref(),
        Some("waiting on the upstream project")
    );

    let ready = store
        .set_slice_status(&blocked, Status::Ready, None)
        .unwrap();
    assert_eq!(ready.blocked_reason, None);
}

#[test]
fn only_one_of_two_racing_agents_wins_a_claim() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");
    let slice = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR2".into(),
            title: "Button variant".into(),
            ..Default::default()
        })
        .unwrap();
    drop(store);

    // Deliberately the *same* actor in two worktrees - the case a naive "is it me?"
    // check would wave through, and the one that actually happens when four worktrees
    // run the same harness.
    let path = fx.path().to_path_buf();
    let slice_a = slice.clone();
    let slice_b = slice.clone();

    let a = std::thread::spawn(move || {
        let mut s = Store::open(&path).unwrap();
        s.set_actor("claude");
        s.claim_slice(&slice_a, "/wt/3", Some("feat/button"))
    });
    let path2 = fx.path().to_path_buf();
    let b = std::thread::spawn(move || {
        let mut s = Store::open(&path2).unwrap();
        s.set_actor("claude");
        s.claim_slice(&slice_b, "/wt/4", Some("feat/button-2"))
    });

    let results = [a.join().unwrap(), b.join().unwrap()];
    let wins = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(wins, 1, "exactly one claim must win: {results:?}");
    assert!(results
        .iter()
        .any(|r| matches!(r, Err(Error::AlreadyClaimed(..)))));

    let store = fx.store();
    let fresh = store.require_slice(plan.id, "PR2").unwrap();
    assert!(fresh.claimed_by.is_some());
    assert!(fresh.worktree_path.is_some());
    // Claiming starts the work, so the slice is no longer merely "ready".
    assert_eq!(fresh.status, Status::Active);
}

#[test]
fn reclaiming_your_own_slice_in_the_same_worktree_is_a_no_op() {
    let fx = Fixture::new();
    let mut store = fx.store();
    store.set_actor("agent-a");
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");
    let slice = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR1".into(),
            title: "Core".into(),
            ..Default::default()
        })
        .unwrap();

    let first = store.claim_slice(&slice, "/wt/1", None).unwrap();
    let second = store.claim_slice(&first, "/wt/1", None).unwrap();
    assert_eq!(second.claimed_by.as_deref(), Some("agent-a"));

    let released = store.release_slice(&second).unwrap();
    assert!(released.claimed_by.is_none());
    // Releasing gives the slice up without pretending the work never started.
    assert!(released.worktree_path.is_some());
}

#[test]
fn a_stale_rev_is_refused_instead_of_overwriting_another_agents_edit() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");

    let read = store.section(plan.id, "outcome").unwrap().unwrap();
    store
        .set_section(
            plan.id,
            SectionWrite {
                key: "outcome",
                body: "agent A's text",
                expect_rev: Some(read.rev),
                ..Default::default()
            },
        )
        .unwrap();

    // Agent B still holds the rev it read before A wrote.
    let err = store.set_section(
        plan.id,
        SectionWrite {
            key: "outcome",
            body: "agent B's text",
            expect_rev: Some(read.rev),
            ..Default::default()
        },
    );
    assert!(matches!(err, Err(Error::Conflict(_, _, _))), "{err:?}");

    let current = store.section(plan.id, "outcome").unwrap().unwrap();
    assert_eq!(current.body, "agent A's text");

    // Re-reading and retrying succeeds, which is the intended recovery.
    store
        .set_section(
            plan.id,
            SectionWrite {
                key: "outcome",
                body: "agent B's merge",
                expect_rev: Some(current.rev),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        store.section(plan.id, "outcome").unwrap().unwrap().body,
        "agent B's merge"
    );
}

#[test]
fn parallel_progress_notes_all_survive() {
    // The failure this project exists to remove: two worktrees writing progress into
    // two copies of one plan, and one copy winning.
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");
    drop(store);

    let mut handles = Vec::new();
    for agent in 0..4 {
        let path = fx.path().to_path_buf();
        let plan_id = plan.id;
        handles.push(std::thread::spawn(move || {
            let mut s = Store::open(&path).unwrap();
            for i in 0..10 {
                s.append_log(NewLog {
                    plan_id,
                    body: format!("agent {agent} note {i}"),
                    ..Default::default()
                })
                .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let store = fx.store();
    let log = store.log(plan.id, None).unwrap();
    assert_eq!(
        log.len(),
        40,
        "every note must survive four concurrent writers"
    );
}

#[test]
fn decisions_number_themselves_and_supersede_in_place() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");

    let d1 = store
        .add_decision(ai_planner_core::NewDecision {
            plan_id: plan.id,
            title: "The value is a specification".into(),
            body: "Not two resolved dates.".into(),
            ..Default::default()
        })
        .unwrap();
    let d2 = store
        .add_decision(ai_planner_core::NewDecision {
            plan_id: plan.id,
            title: "Lean on MUI for the calendar".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(d1.key, "D1");
    assert_eq!(d2.key, "D2");

    let superseded = store
        .supersede_decision(
            plan.id,
            "D2",
            "D3",
            Some("DateCalendar has no controlled month"),
        )
        .unwrap();
    assert_eq!(superseded.status, DecisionStatus::Superseded);
    assert_eq!(superseded.superseded_by.as_deref(), Some("D3"));
    assert_eq!(
        superseded.supersede_note.as_deref(),
        Some("DateCalendar has no controlled month")
    );
    // The original reasoning is annotated, never edited.
    assert_eq!(superseded.body, d2.body);
}

#[test]
fn the_bundle_carries_the_whole_document() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "ACME-1234 - Picker");
    store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR1".into(),
            title: "Core".into(),
            demo_md: Some("Pick Last quarter.".into()),
            ..Default::default()
        })
        .unwrap();
    store
        .add_question(plan.id, None, "range on Summary Panel?")
        .unwrap();
    store
        .add_gotcha(plan.id, "The Herd symlink is shared", "Put it back.")
        .unwrap();
    store
        .append_log(NewLog {
            plan_id: plan.id,
            body: "Grounded and planned.".into(),
            ..Default::default()
        })
        .unwrap();

    let bundle = store.bundle(plan.id).unwrap();
    let md = ai_planner_core::render_plan(&bundle);

    assert!(md.contains("### PR1 - Core"));
    assert!(md.contains("**Demo:** Pick Last quarter."));
    assert!(md.contains("- [ ] range on Summary Panel?"));
    assert!(md.contains("### The Herd symlink is shared"));
    assert!(md.contains("Grounded and planned."));
}
