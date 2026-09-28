-- Let a note outlive the slice it described.
--
-- `log.slice_id` has always been `ON DELETE SET NULL`: the schema's own answer to a
-- slice going away is to detach the notes written against it, not to destroy them.
-- But the append-only trigger fired on *any* update to a log row, including the one
-- SQLite generates to perform that detach - so deleting a slice was impossible, and
-- the only way to remove one was to take the whole plan with it.
--
-- Scoping the trigger to the columns that carry the note itself keeps the guarantee
-- that matters (a progress note is never rewritten or re-dated) and stops it from
-- standing in the way of the cascade the schema already asked for.

DROP TRIGGER log_is_append_only;

CREATE TRIGGER log_is_append_only
BEFORE UPDATE OF id, plan_id, at, actor, kind, branch, worktree_path, body ON log
BEGIN
    SELECT RAISE(ABORT, 'log is append-only');
END;
