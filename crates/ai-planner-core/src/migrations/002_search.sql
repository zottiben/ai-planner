-- Full-text search over everything a plan says.
--
-- Deliberately AI-free and deterministic (D8): FTS5 with BM25, re-ranked by recency
-- and by whether the hit is in the repo you are standing in. A local embedding model
-- is an opt-in addition later, not a prerequisite for finding a plan.

CREATE VIRTUAL TABLE search USING fts5(
    body,
    plan_id     UNINDEXED,
    kind        UNINDEXED,
    ref         UNINDEXED,
    title,
    tokenize    = "unicode61 remove_diacritics 2"
);

-- The index is rebuilt from the tables it summarises rather than maintained by
-- triggers: the write paths are many, a stale index is worse than a rebuilt one, and
-- rebuilding the whole thing takes milliseconds at this scale.
CREATE TABLE search_state (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    rebuilt_at   TEXT NOT NULL,
    rows         INTEGER NOT NULL
);
