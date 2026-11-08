-- Deleting an entry or a batch with a token, and withdrawing one from the account
-- page (design.md, Accounts; Agent interface; Entry lifecycle).

-- The token that submitted the entry: a token deletes only the pending entries it
-- submitted itself. Entries stored before have none, so no token deletes them.
ALTER TABLE entries ADD COLUMN token_id TEXT REFERENCES tokens (id);

-- What took a unit: an entry, or a series or series version. A deleted entry's
-- unit stays, with no entry, and still counts as an entry's, never as a series'.
ALTER TABLE allowance_units ADD COLUMN kind TEXT NOT NULL DEFAULT 'entry'
    CHECK (kind IN ('entry', 'series'));
UPDATE allowance_units SET kind = 'series' WHERE entry_seq IS NULL;

-- Set when the entries that cited a set are deleted. Its files go; the row stays
-- until uncited sets expire, so its bytes still count toward the account's day.
ALTER TABLE attachment_sets ADD COLUMN deleted_at TEXT;

-- A deleted entry, so the account that deleted it is told when, and a retried
-- request that had worked is recognised. Nobody else learns anything from it.
CREATE TABLE deleted_entries (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    -- The batch it was deleted with, if any.
    batch_id TEXT,
    deleted_at TEXT NOT NULL
) STRICT;

CREATE INDEX deleted_entries_batch ON deleted_entries (batch_id)
    WHERE batch_id IS NOT NULL;
