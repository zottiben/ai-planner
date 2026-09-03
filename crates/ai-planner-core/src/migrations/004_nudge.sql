-- Nudges the agent has already been given.
--
-- A hook that fires on every turn has to stay silent once it has said its piece, or it
-- becomes noise the model learns to skip. A nudge is keyed by a fingerprint of the
-- state that provoked it, so the same complaint is made once and only repeats when the
-- underlying state actually changes.

CREATE TABLE nudge (
    id            INTEGER PRIMARY KEY,
    worktree_path TEXT NOT NULL,
    kind          TEXT NOT NULL,
    fingerprint   TEXT NOT NULL,
    at            TEXT NOT NULL,
    UNIQUE (worktree_path, kind, fingerprint)
);

CREATE INDEX nudge_lookup ON nudge(worktree_path, kind);
