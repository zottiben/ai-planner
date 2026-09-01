-- Optional local semantic search.
--
-- Vectors live in an ordinary table rather than a `sqlite-vec` virtual one. At this
-- scale - thousands of rows, not millions - a brute-force cosine scan is faster than
-- the query planner overhead, and it keeps the file free of the shadow tables a vec0
-- index creates, which matters because TablePlus is a first-class client (D13).
--
-- Rows are keyed by what they describe rather than by the FTS rowid, so rebuilding the
-- lexical index does not throw the embeddings away.

CREATE TABLE embedding (
    id       INTEGER PRIMARY KEY,
    plan_id  INTEGER NOT NULL REFERENCES plan(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL,
    ref      TEXT NOT NULL,
    -- sha256 of the embedded text: unchanged text is never re-embedded.
    sha      TEXT NOT NULL,
    model    TEXT NOT NULL,
    dims     INTEGER NOT NULL,
    vector   BLOB NOT NULL,
    title    TEXT NOT NULL DEFAULT '',
    snippet  TEXT NOT NULL DEFAULT '',
    built_at TEXT NOT NULL,
    UNIQUE (plan_id, kind, ref, model)
);

CREATE INDEX embedding_plan ON embedding(plan_id);
CREATE INDEX embedding_model ON embedding(model);

CREATE TABLE embedding_state (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    model    TEXT NOT NULL,
    dims     INTEGER NOT NULL,
    rows     INTEGER NOT NULL,
    built_at TEXT NOT NULL
);
