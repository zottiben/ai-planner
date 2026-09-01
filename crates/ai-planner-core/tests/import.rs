//! Import has to be lossless enough that the markdown files can be deleted (D3, D5).
//! The document below is a composite of every dialect found in the real plans.

use std::path::Path;

use ai_planner_core::import::{ImportOptions, Outcome};
use ai_planner_core::{render_plan, Status, Store};

const PLAN: &str = r#"# ACME-1234 - Reusable Date Range Picker

> Living plan. Local/untracked (git-excluded). Owner: Ben Zotti.
> Base branch: `master`.

**Ticket:** https://app.clickup.com/t/1234567/ACME-1234

---

## 1. What we are building

A reusable `DateRangePicker` with two presentation variants over one shared core.

### Variant `fields`

A read-only MUI `TextField` showing `DD/MM/YYYY - DD/MM/YYYY`.

## 2. Where it lands today

```php
$inMonth = $lastMonth ? Date::now()->subMonthNoOverflow() : Date::now();
# not a heading
```

Both are already `whereBetween`.

## 3. Design decisions

### D1 - The value is a *specification*, not two resolved dates

If the value were `{ start, end }`, a bookmarked "This month" freezes.

### D4 - Lean on MUI for the calendar, build only the chrome

Use `DateCalendar` with a custom `PickersDay` slot.

## 5. Delivery slices

Seven PRs plus one optional.

### PR1 - Shared core + config + `fields` variant (~40 files)

The core and the first variant.

**Demo:** Main Dashboard, pick "Last quarter" on Summary Panel.

### PR2 - `button` variant + first View More page (~36 files)  ✅ IN REVIEW

The drill-down shell.

**Demo:** Reports | Monthly, "Previous 3 months".

### PR3 - Relative date range builder (~16 files) - DELIVERED 2026-08-24

Shared `RelativeRangeBuilder`.

### PR7 - Migrate the dashboard charts off `lastMonth`  ⛔ BLOCKED

Waiting on the charts team.

### PR8 (optional) - `Exclude...` panels

Only if product confirms.

## 6. Open questions

- **Which variant on which surface.** Recommendation: `button` for page-level filters.
- **`Exclude...`** - build it or drop it.

## Progress log

- 2026-08-24 - PR3 built, gated and browser-verified, not pushed.
- 2026-08-19 - PR1 open as #412.
"#;

const HANDOFF: &str = r#"# HANDOFF - ACME-1234 Reusable Date Range Picker

## RESUME HERE

- Branch: `feat/date-range-picker`
- Head: `a1b2c3d4e5`

## Gotchas

### Verifying in the browser costs you the shared Herd symlink

`app.test` is one symlink shared by every worktree. Put it back.

### Test queries

While the panel is open MUI makes the rest of the page inert.
"#;

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

#[test]
fn a_plan_survives_the_round_trip_through_the_database() {
    let mut fx = fixture();
    let path = Path::new("/wt/3/widget/ACME-1234_BUILD_PLAN.md");
    let outcome = fx
        .store
        .import_file(fx.repo_id, path, PLAN, ImportOptions::default())
        .unwrap();

    let plan = match outcome {
        Outcome::Created { plan, .. } => plan,
        other => panic!("expected a new plan, got {other:?}"),
    };
    assert_eq!(plan.slug, "acme-1234");
    assert_eq!(plan.ticket_key.as_deref(), Some("ACME-1234"));
    assert_eq!(plan.base_branch.as_deref(), Some("master"));
    assert_eq!(plan.owner.as_deref(), Some("Ben Zotti"));
    assert_eq!(plan.status, Status::Active);

    let md = render_plan(&fx.store.bundle(plan.id).unwrap());

    // Every heading the author wrote is still a heading.
    for heading in [
        "## 1. What we are building",
        "## 2. Where it lands today",
        "## 3. Design decisions",
        "## 5. Delivery slices",
        "## 6. Open questions",
        "## Progress log",
    ] {
        assert!(md.contains(heading), "lost {heading}\n{md}");
    }
    // Including the prose sub-heading and the fenced block that looked like one.
    assert!(md.contains("### Variant `fields`"));
    assert!(md.contains("# not a heading"));

    for key in ["D1", "D4", "PR1", "PR2", "PR3", "PR7", "PR8"] {
        assert!(md.contains(key), "lost {key}");
    }
    assert!(md.contains("**Demo:** Main Dashboard, pick \"Last quarter\" on Summary Panel."));
    assert!(md.contains("Which variant on which surface"));
    assert!(md.contains("2026-08-24"));
    assert!(md.contains("PR3 built, gated and browser-verified, not pushed."));

    // Statuses came out of the markers, not out of the prose.
    let slices = fx.store.slices(plan.id).unwrap();
    let by_key = |k: &str| slices.iter().find(|s| s.key == k).unwrap().status;
    assert_eq!(by_key("PR1"), Status::Ready);
    assert_eq!(by_key("PR2"), Status::InReview);
    assert_eq!(by_key("PR3"), Status::Done);
    assert_eq!(by_key("PR7"), Status::Blocked);
    assert_eq!(by_key("PR8"), Status::Deferred);

    let pr1 = slices.iter().find(|s| s.key == "PR1").unwrap();
    assert_eq!(pr1.estimate_files, Some(40));
    assert_eq!(pr1.title, "Shared core + config + `fields` variant");

    // And the original file is still there byte for byte.
    assert_eq!(fx.store.raw_md(plan.id).unwrap().as_deref(), Some(PLAN));
}

#[test]
fn the_same_file_in_four_worktrees_imports_once() {
    let mut fx = fixture();
    let first = fx
        .store
        .import_file(
            fx.repo_id,
            Path::new("/wt/1/repo/CANVAS_EDITOR_BUILD_PLAN.md"),
            PLAN,
            ImportOptions::default(),
        )
        .unwrap();
    assert!(matches!(first, Outcome::Created { .. }));

    for wt in ["/wt/2/repo", "/wt/3/repo"] {
        let again = fx
            .store
            .import_file(
                fx.repo_id,
                &Path::new(wt).join("CANVAS_EDITOR_BUILD_PLAN.md"),
                PLAN,
                ImportOptions::default(),
            )
            .unwrap();
        match again {
            Outcome::AlreadyImported { first_seen, .. } => {
                assert_eq!(first_seen, "/wt/1/repo/CANVAS_EDITOR_BUILD_PLAN.md")
            }
            other => panic!("expected a duplicate, got {other:?}"),
        }
    }

    let plans = fx.store.list_plans(&Default::default()).unwrap();
    assert_eq!(plans.len(), 1);
    // Every copy is still recorded, so you can see where they all were.
    assert_eq!(fx.store.import_sources(plans[0].id).unwrap().len(), 3);
}

#[test]
fn two_copies_that_drifted_apart_are_reported_not_merged() {
    let mut fx = fixture();
    fx.store
        .import_file(
            fx.repo_id,
            Path::new("/wt/3/repo/ACME-1234_BUILD_PLAN.md"),
            PLAN,
            ImportOptions::default(),
        )
        .unwrap();

    let drifted = PLAN.replace("Seven PRs plus one optional.", "Eight PRs now.");
    let outcome = fx
        .store
        .import_file(
            fx.repo_id,
            Path::new("/wt/4/repo/ACME-1234_BUILD_PLAN.md"),
            &drifted,
            ImportOptions::default(),
        )
        .unwrap();
    match outcome {
        Outcome::Conflict {
            existing_sources, ..
        } => assert_eq!(
            existing_sources,
            vec!["/wt/3/repo/ACME-1234_BUILD_PLAN.md"]
        ),
        other => panic!("expected a conflict, got {other:?}"),
    }

    // Nothing changed until it was asked for explicitly.
    let plan = fx
        .store
        .plan_by_slug(fx.repo_id, "acme-1234")
        .unwrap()
        .unwrap();
    assert!(
        render_plan(&fx.store.bundle(plan.id).unwrap()).contains("Seven PRs plus one optional.")
    );

    let replaced = fx
        .store
        .import_file(
            fx.repo_id,
            Path::new("/wt/4/repo/ACME-1234_BUILD_PLAN.md"),
            &drifted,
            ImportOptions {
                replace: true,
                dry_run: false,
            },
        )
        .unwrap();
    assert!(matches!(replaced, Outcome::Replaced { .. }));
    let md = render_plan(&fx.store.bundle(plan.id).unwrap());
    assert!(md.contains("Eight PRs now."));
    assert!(!md.contains("Seven PRs plus one optional."));
    // Replacing rebuilds the content without duplicating the slices.
    assert_eq!(fx.store.slices(plan.id).unwrap().len(), 5);
}

#[test]
fn a_handoff_attaches_to_its_plan_and_leaves_its_gotchas_behind() {
    let mut fx = fixture();
    fx.store
        .import_file(
            fx.repo_id,
            Path::new("/wt/3/repo/ACME-1234_BUILD_PLAN.md"),
            PLAN,
            ImportOptions::default(),
        )
        .unwrap();

    let outcome = fx
        .store
        .import_file(
            fx.repo_id,
            Path::new("/wt/3/repo/HANDOFF-ACME-1234.md"),
            HANDOFF,
            ImportOptions::default(),
        )
        .unwrap();
    let plan = match outcome {
        Outcome::HandoffAttached { plan, .. } => plan,
        other => panic!("expected a handoff, got {other:?}"),
    };
    assert_eq!(plan.slug, "acme-1234");

    let gotchas = fx.store.gotchas(plan.id).unwrap();
    assert_eq!(gotchas.len(), 2);
    assert!(gotchas[0].title.starts_with("Verifying in the browser"));
    assert!(gotchas[0].body.contains("Put it back."));

    // It did not become a second plan.
    assert_eq!(fx.store.list_plans(&Default::default()).unwrap().len(), 1);
}

#[test]
fn a_dry_run_writes_nothing() {
    let mut fx = fixture();
    let outcome = fx
        .store
        .import_file(
            fx.repo_id,
            Path::new("/wt/3/repo/ACME-1234_BUILD_PLAN.md"),
            PLAN,
            ImportOptions {
                replace: false,
                dry_run: true,
            },
        )
        .unwrap();
    match outcome {
        Outcome::Planned { slug, slices, .. } => {
            assert_eq!(slug, "acme-1234");
            assert_eq!(slices, 5);
        }
        other => panic!("expected a preview, got {other:?}"),
    }
    assert!(fx.store.list_plans(&Default::default()).unwrap().is_empty());
}
