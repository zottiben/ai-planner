-- ai-planner core schema.
--
-- Timestamps are ISO-8601 UTC TEXT and statuses are lowercase words on purpose:
-- this file is browsed directly in TablePlus (D13), so rows have to be readable
-- without a decoder ring.

CREATE TABLE repo (
    id          INTEGER PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    remote_url  TEXT,
    main_path   TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE plan (
    id          INTEGER PRIMARY KEY,
    repo_id     INTEGER NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    slug        TEXT NOT NULL,
    title       TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'draft'
                CHECK (status IN ('draft','ready','active','in_review','blocked','done','deferred')),
    summary     TEXT,
    ticket_key  TEXT,
    ticket_url  TEXT,
    base_branch TEXT,
    owner       TEXT,
    raw_md      TEXT,
    source_path TEXT,
    rev         INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE (repo_id, slug)
);

CREATE INDEX plan_repo_status ON plan(repo_id, status);
CREATE INDEX plan_ticket ON plan(ticket_key);

CREATE TABLE plan_source (
    id          INTEGER PRIMARY KEY,
    plan_id     INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    ref         TEXT NOT NULL,
    note        TEXT,
    created_at  TEXT NOT NULL
);

CREATE INDEX plan_source_plan ON plan_source(plan_id);

-- A section is a narrative block of the plan document. `renders` names a built-in
-- block to emit at that position instead of (or after) the body, which is what makes
-- a rendered plan come back out in its original shape (D3).
CREATE TABLE plan_section (
    id          INTEGER PRIMARY KEY,
    plan_id     INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    ord         INTEGER NOT NULL,
    key         TEXT NOT NULL,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '',
    renders     TEXT NOT NULL DEFAULT 'body'
                CHECK (renders IN ('body','sources','decisions','slices','questions','gotchas','log')),
    rev         INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE (plan_id, key)
);

CREATE INDEX plan_section_ord ON plan_section(plan_id, ord);

CREATE TABLE decision (
    id            INTEGER PRIMARY KEY,
    plan_id       INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    ord           INTEGER NOT NULL,
    key           TEXT NOT NULL,
    title         TEXT NOT NULL,
    body          TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL DEFAULT 'agreed'
                  CHECK (status IN ('proposed','agreed','superseded','rejected')),
    superseded_by TEXT,
    -- Why it was superseded. Kept apart from `body` so the original reasoning is
    -- never edited, only annotated.
    supersede_note TEXT,
    rev           INTEGER NOT NULL DEFAULT 1,
    decided_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    UNIQUE (plan_id, key)
);

CREATE INDEX decision_ord ON decision(plan_id, ord);

CREATE TABLE slice (
    id             INTEGER PRIMARY KEY,
    plan_id        INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    ord            INTEGER NOT NULL,
    key            TEXT NOT NULL,
    title          TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'ready'
                   CHECK (status IN ('draft','ready','active','in_review','blocked','done','deferred')),
    scope_md       TEXT NOT NULL DEFAULT '',
    demo_md        TEXT,
    estimate_files INTEGER,
    branch         TEXT,
    base_branch    TEXT,
    pr_url         TEXT,
    worktree_path  TEXT,
    claimed_by     TEXT,
    claimed_at     TEXT,
    blocked_reason TEXT,
    started_at     TEXT,
    completed_at   TEXT,
    rev            INTEGER NOT NULL DEFAULT 1,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    UNIQUE (plan_id, key)
);

CREATE INDEX slice_ord ON slice(plan_id, ord);
CREATE INDEX slice_branch ON slice(branch);
CREATE INDEX slice_worktree ON slice(worktree_path);

CREATE TABLE slice_dep (
    slice_id      INTEGER NOT NULL REFERENCES slice(id) ON DELETE CASCADE,
    depends_on_id INTEGER NOT NULL REFERENCES slice(id) ON DELETE CASCADE,
    PRIMARY KEY (slice_id, depends_on_id)
);

CREATE TABLE question (
    id          INTEGER PRIMARY KEY,
    plan_id     INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    slice_id    INTEGER REFERENCES slice(id) ON DELETE SET NULL,
    body        TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'open'
                CHECK (status IN ('open','answered','dropped')),
    answer      TEXT,
    asked_at    TEXT NOT NULL,
    answered_at TEXT
);

CREATE INDEX question_plan ON question(plan_id, status);

CREATE TABLE gotcha (
    id         INTEGER PRIMARY KEY,
    plan_id    INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    body       TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    UNIQUE (plan_id, title)
);

-- Insert-only. Progress is the highest-frequency write and the one that got
-- clobbered when two worktrees held two copies of one plan, so it is modelled as
-- an append-only event stream that cannot conflict (D4).
CREATE TABLE log (
    id       INTEGER PRIMARY KEY,
    plan_id  INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    slice_id INTEGER REFERENCES slice(id) ON DELETE SET NULL,
    at       TEXT NOT NULL,
    actor    TEXT,
    kind     TEXT NOT NULL DEFAULT 'progress'
             CHECK (kind IN ('progress','status','decision','gotcha','verification','blocker','handoff')),
    branch   TEXT,
    worktree_path TEXT,
    body     TEXT NOT NULL
);

CREATE INDEX log_plan_at ON log(plan_id, at DESC);

-- Updates are refused outright. Deletes are deliberately left alone so that
-- dropping a plan still cascades; no command ever deletes a log row on its own.
CREATE TRIGGER log_is_append_only
BEFORE UPDATE ON log
BEGIN
    SELECT RAISE(ABORT, 'log is append-only');
END;

CREATE TABLE handoff (
    id            INTEGER PRIMARY KEY,
    plan_id       INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    worktree_path TEXT NOT NULL,
    branch        TEXT,
    head_sha      TEXT,
    gates_json    TEXT,
    resume_md     TEXT NOT NULL DEFAULT '',
    next_md       TEXT NOT NULL DEFAULT '',
    actor         TEXT,
    at            TEXT NOT NULL
);

CREATE INDEX handoff_lookup ON handoff(plan_id, worktree_path, at DESC);

-- The learned (repo, branch, worktree) -> plan association. A counter beats a model
-- for this question because a wrong answer sends progress into the wrong plan (D9).
CREATE TABLE plan_affinity (
    id            INTEGER PRIMARY KEY,
    plan_id       INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    repo_id       INTEGER NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    branch        TEXT NOT NULL DEFAULT '',
    worktree_path TEXT NOT NULL DEFAULT '',
    hits          INTEGER NOT NULL DEFAULT 0,
    last_at       TEXT NOT NULL,
    UNIQUE (plan_id, repo_id, branch, worktree_path)
);

CREATE INDEX plan_affinity_lookup ON plan_affinity(repo_id, branch, worktree_path);

CREATE TABLE plan_import (
    id          INTEGER PRIMARY KEY,
    plan_id     INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    source_path TEXT NOT NULL,
    sha256      TEXT NOT NULL,
    bytes       INTEGER NOT NULL,
    imported_at TEXT NOT NULL
);

CREATE INDEX plan_import_sha ON plan_import(sha256);

-- Views exist so that opening the file in TablePlus answers "what is going on"
-- with no query written (D13).

CREATE VIEW v_plans AS
SELECT
    p.id,
    r.name                                    AS repo,
    p.slug,
    p.title,
    p.status,
    p.ticket_key,
    (SELECT COUNT(*) FROM slice s WHERE s.plan_id = p.id)                        AS slices,
    (SELECT COUNT(*) FROM slice s WHERE s.plan_id = p.id AND s.status = 'done')  AS done,
    CASE
        WHEN (SELECT COUNT(*) FROM slice s WHERE s.plan_id = p.id) = 0 THEN NULL
        ELSE CAST(ROUND(
            100.0 * (SELECT COUNT(*) FROM slice s WHERE s.plan_id = p.id AND s.status = 'done')
                  / (SELECT COUNT(*) FROM slice s WHERE s.plan_id = p.id)) AS INTEGER)
    END                                                                          AS percent,
    (SELECT COUNT(*) FROM question q WHERE q.plan_id = p.id AND q.status = 'open') AS open_questions,
    (SELECT MAX(l.at) FROM log l WHERE l.plan_id = p.id)                         AS last_activity,
    p.updated_at,
    p.created_at
FROM plan p
JOIN repo r ON r.id = p.repo_id;

CREATE VIEW v_slices AS
SELECT
    s.id,
    r.name  AS repo,
    p.slug  AS plan,
    s.ord,
    s.key,
    s.title,
    s.status,
    s.branch,
    s.pr_url,
    s.claimed_by,
    s.worktree_path,
    s.claimed_at,
    s.completed_at,
    s.updated_at
FROM slice s
JOIN plan p ON p.id = s.plan_id
JOIN repo r ON r.id = p.repo_id;

CREATE VIEW v_log AS
SELECT
    l.id,
    l.at,
    r.name AS repo,
    p.slug AS plan,
    s.key  AS slice,
    l.kind,
    l.actor,
    l.branch,
    l.body
FROM log l
JOIN plan p ON p.id = l.plan_id
JOIN repo r ON r.id = p.repo_id
LEFT JOIN slice s ON s.id = l.slice_id;

CREATE VIEW v_open_questions AS
SELECT
    q.id,
    r.name AS repo,
    p.slug AS plan,
    s.key  AS slice,
    q.body,
    q.asked_at
FROM question q
JOIN plan p ON p.id = q.plan_id
JOIN repo r ON r.id = p.repo_id
LEFT JOIN slice s ON s.id = q.slice_id
WHERE q.status = 'open';

CREATE VIEW v_worktrees AS
SELECT
    s.worktree_path,
    r.name AS repo,
    p.slug AS plan,
    s.key  AS slice,
    s.status,
    s.branch,
    s.claimed_by,
    s.claimed_at
FROM slice s
JOIN plan p ON p.id = s.plan_id
JOIN repo r ON r.id = p.repo_id
WHERE s.worktree_path IS NOT NULL;
