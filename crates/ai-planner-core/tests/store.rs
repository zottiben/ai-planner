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

    /// Register another repo, and say its id.
    fn repo(&self, key: &str) -> i64 {
        let name = key.rsplit('/').next().unwrap();
        self.store()
            .db()
            .conn()
            .query_row(
                "INSERT INTO repo (key, name, created_at)
                 VALUES (?1, ?2, '2026-01-01T00:00:00Z')
                 RETURNING id",
                [key, name],
                |r| r.get(0),
            )
            .unwrap()
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
fn a_plans_exact_name_finds_it_even_when_another_plans_name_contains_it() {
    // Neither has a ticket key. "shout" is one plan's slug and a prefix of the other's, and
    // the plan that is exactly called that is the one meant - from inside the repo and
    // from outside it.
    let fx = Fixture::new();
    let mut store = fx.store();
    let shout = seed_plan(&mut store, fx.repo_id, "Shout");
    seed_plan(&mut store, fx.repo_id, "Shout greetings");

    for repo in [Some(fx.repo_id), None] {
        let found = store.find_plan("shout", repo).unwrap();
        assert_eq!(
            found.id, shout.id,
            "looking up \"shout\" with repo {repo:?}"
        );
    }
    // A reference that is nobody's exact name is still a question, not a guess.
    let err = store.find_plan("sho", Some(fx.repo_id));
    assert!(matches!(err, Err(Error::AmbiguousPlan(_, 2, _))), "{err:?}");

    // Asked from another repo with no plan of that name, the one plan that has it is
    // still meant - more than one that merely starts with it.
    let other = fx.repo("github.com/acme/other");
    assert_eq!(store.find_plan("shout", Some(other)).unwrap().id, shout.id);
    // Once that repo has its own, its own is the one meant there...
    let theirs = seed_plan(&mut store, other, "Shout");
    assert_eq!(store.find_plan("shout", Some(other)).unwrap().id, theirs.id);
    assert_eq!(
        store.find_plan("shout", Some(fx.repo_id)).unwrap().id,
        shout.id
    );
    // ...and from a third repo, two plans of that name is a question again.
    let third = fx.repo("github.com/acme/third");
    let err = store.find_plan("shout", Some(third));
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
    // Finishing gives the worktree back, so a done slice never looks like work in
    // progress - but where it was built stays readable.
    assert!(done.claimed_by.is_none());
    assert!(done.worktree_path.is_some() || active.worktree_path.is_none());

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
fn claiming_fills_the_branch_in_but_never_overwrites_the_planned_one() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "Picker");

    // A slice the plan says is built on a particular branch.
    let planned = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR1".into(),
            title: "Core".into(),
            branch: Some("feat/picker".into()),
            ..Default::default()
        })
        .unwrap();
    // Claimed from somewhere else entirely - which must not rewrite the plan, or the
    // slice stops pointing at the work and drift detection goes blind.
    let claimed = store.claim_slice(&planned, "/wt/1", Some("main")).unwrap();
    assert_eq!(claimed.branch.as_deref(), Some("feat/picker"));

    // A slice with no branch yet takes the one it was claimed on.
    let blank = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR2".into(),
            title: "Variant".into(),
            ..Default::default()
        })
        .unwrap();
    let claimed = store
        .claim_slice(&blank, "/wt/1", Some("feat/variant"))
        .unwrap();
    assert_eq!(claimed.branch.as_deref(), Some("feat/variant"));

    // Changing it is possible, but only on purpose.
    let moved = store
        .update_slice(
            &claimed,
            ai_planner_core::SliceUpdate {
                branch: Some("feat/renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(moved.branch.as_deref(), Some("feat/renamed"));
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

/// Deleting has to take the derived rows with it, or a gone plan keeps answering
/// searches and keeps resolving in the worktree it was built in.
#[test]
fn deleting_a_plan_leaves_nothing_of_it_behind_and_spares_its_neighbour() {
    let fx = Fixture::new();
    let mut store = fx.store();

    let doomed = seed_plan(&mut store, fx.repo_id, "ACME-1234 - Picker");
    let keeper = seed_plan(&mut store, fx.repo_id, "ACME-9999 - Canvas");

    let slice = store
        .add_slice(NewSlice {
            plan_id: doomed.id,
            key: "PR1".into(),
            title: "Shared core".into(),
            ..Default::default()
        })
        .unwrap();
    store
        .add_decision(ai_planner_core::store::NewDecision {
            plan_id: doomed.id,
            title: "The value is a specification".into(),
            body: "Not two resolved dates.".into(),
            ..Default::default()
        })
        .unwrap();
    store
        .add_gotcha(doomed.id, "The Herd symlink is shared", "Put it back.")
        .unwrap();
    store
        .add_question(doomed.id, Some(slice.id), "past or both?")
        .unwrap();
    store
        .append_log(NewLog {
            plan_id: doomed.id,
            slice_id: Some(slice.id),
            body: "Core landed.".into(),
            ..Default::default()
        })
        .unwrap();
    store
        .write_handoff(ai_planner_core::NewHandoff {
            plan_id: doomed.id,
            worktree_path: "/wt/1".into(),
            branch: Some("feat/core".into()),
            head_sha: None,
            gates: Vec::new(),
            resume_md: "Where it got to.".into(),
            next_md: "PR2".into(),
        })
        .unwrap();
    store
        .record_affinity(doomed.id, fx.repo_id, Some("feat/core"), "/wt/1")
        .unwrap();
    store.reindex().unwrap();

    let before = store.search_rows().unwrap();
    assert!(
        before > 0,
        "the fixture has to be indexed to prove anything"
    );

    let removal = store.delete_plan(&doomed, false, Some("/wt/1")).unwrap();
    assert_eq!(removal.slug, "acme-1234");
    assert_eq!(removal.slices, 1);
    assert_eq!(removal.decisions, 1);
    assert_eq!(removal.gotchas, 1);
    assert_eq!(removal.questions, 1);
    assert_eq!(removal.handoffs, 1);
    // One progress note, plus the status rows the claim and the slice wrote.
    assert!(removal.log_entries >= 1, "{removal:?}");

    assert!(matches!(
        store.get_plan(doomed.id),
        Err(Error::NoSuchPlan(_))
    ));
    let conn_counts = |table: &str| -> i64 {
        store
            .db()
            .conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE plan_id = ?1"),
                [doomed.id],
                |r| r.get(0),
            )
            .unwrap()
    };
    for table in [
        "plan_section",
        "slice",
        "decision",
        "question",
        "gotcha",
        "log",
        "handoff",
        "plan_affinity",
        "search",
    ] {
        assert_eq!(conn_counts(table), 0, "{table} still has rows");
    }

    // The counter the search and doctor paths read has to match the index itself.
    let indexed: i64 = store
        .db()
        .conn()
        .query_row("SELECT COUNT(*) FROM search", [], |r| r.get(0))
        .unwrap();
    assert_eq!(store.search_rows().unwrap(), indexed);
    assert!(indexed > 0, "the surviving plan is still indexed");
    assert!(indexed < before);

    // The neighbour is untouched, including its place in the index.
    let kept = store.get_plan(keeper.id).unwrap();
    assert_eq!(kept.slug, "acme-9999");
    let opts = ai_planner_core::SearchOptions {
        limit: 20,
        ..Default::default()
    };
    let hits = store.search("Canvas", &opts).unwrap();
    assert!(hits.iter().all(|h| h.plan.id == keeper.id), "{hits:?}");
    // And the deleted plan answers nothing at all.
    assert!(
        store.search("Herd symlink", &opts).unwrap().is_empty(),
        "a deleted plan is still in the index"
    );
}

/// A plan is shared. Deleting one somebody else is mid-way through throws their work
/// away with it, so the claim has to be released or the delete forced.
#[test]
fn a_plan_another_worktree_is_holding_is_not_deleted_by_accident() {
    let fx = Fixture::new();
    let mut store = fx.store();
    let plan = seed_plan(&mut store, fx.repo_id, "ACME-1234 - Picker");
    let slice = store
        .add_slice(NewSlice {
            plan_id: plan.id,
            key: "PR1".into(),
            title: "Shared core".into(),
            ..Default::default()
        })
        .unwrap();
    store.claim_slice(&slice, "/wt/other", None).unwrap();

    let refused = store.delete_plan(&plan, false, Some("/wt/mine"));
    assert!(
        matches!(&refused, Err(Error::PlanIsHeld(slug, 1, detail))
            if slug == "acme-1234" && detail.contains("/wt/other")),
        "{refused:?}"
    );
    assert!(store.get_plan(plan.id).is_ok(), "nothing was deleted");

    // Work this worktree holds itself is its own to throw away.
    let mine = seed_plan(&mut store, fx.repo_id, "ACME-4321 - Mine");
    let slice = store
        .add_slice(NewSlice {
            plan_id: mine.id,
            key: "PR1".into(),
            title: "Mine".into(),
            ..Default::default()
        })
        .unwrap();
    store.claim_slice(&slice, "/wt/mine", None).unwrap();
    store.delete_plan(&mine, false, Some("/wt/mine")).unwrap();
    assert!(matches!(store.get_plan(mine.id), Err(Error::NoSuchPlan(_))));

    // And forcing it goes through, reporting what it took.
    let removal = store.delete_plan(&plan, true, Some("/wt/mine")).unwrap();
    assert_eq!(removal.held.len(), 1);
    assert_eq!(removal.held[0].worktree_path, "/wt/other");
    assert!(matches!(store.get_plan(plan.id), Err(Error::NoSuchPlan(_))));
}
