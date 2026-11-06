-- The owner's review queue (design.md, Admin; Submission and screening).

-- 1 for the owner's account, set by `filedrawer admin grant` from a shell on the
-- server. No token carries it: admin pages read only the browser session.
ALTER TABLE accounts ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0
    CHECK (is_admin IN (0, 1));

-- When one of the account's entries first reached `review`: the review guard. Only
-- the owner clears it; rejecting or withdrawing the entry leaves it.
ALTER TABLE accounts ADD COLUMN review_guard_at TEXT;

-- An account that already has an entry in review or held is guarded from the
-- earliest such entry's submission; the day it reached review wasn't recorded.
UPDATE accounts SET review_guard_at = (
    SELECT min(submitted_at) FROM entries
    WHERE entries.account_id = accounts.id AND status IN ('review', 'held')
);
