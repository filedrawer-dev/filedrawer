-- no-transaction
-- Adds `pending`, the hour an accepted submission waits so its author can fix it
-- (design.md, Entry status), and `hold_until`, when that hour ends.
--
-- SQLite can't change a CHECK constraint in place, so `entries` is rebuilt. That
-- needs foreign keys off, which SQLite ignores inside a transaction, so this file
-- runs outside the migration transaction and opens its own.

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE entries_new (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    status TEXT NOT NULL CHECK (status IN (
        'pending', 'screening', 'review', 'held', 'frozen', 'embargoed',
        'public', 'rejected', 'withdrawn', 'removed'
    )),
    source TEXT NOT NULL,
    submitted_at TEXT NOT NULL,
    first_submitted_at TEXT NOT NULL,
    embargo_until TEXT,
    published_at TEXT,
    analyzer_version INTEGER NOT NULL,
    outcome TEXT,
    kind TEXT,
    stage TEXT,
    -- While `pending`: when the entry goes on to screening by itself.
    hold_until TEXT
) STRICT;

INSERT INTO entries_new (seq, id, account_id, status, source, submitted_at,
    first_submitted_at, embargo_until, published_at, analyzer_version, outcome, kind, stage)
SELECT seq, id, account_id, status, source, submitted_at,
    first_submitted_at, embargo_until, published_at, analyzer_version, outcome, kind, stage
FROM entries;

DROP TABLE entries;
ALTER TABLE entries_new RENAME TO entries;

CREATE INDEX entries_status ON entries (status);
CREATE INDEX entries_hold ON entries (hold_until) WHERE status = 'pending';

COMMIT;

PRAGMA foreign_keys = ON;
