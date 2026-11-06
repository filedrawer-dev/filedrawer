-- no-transaction
-- Series (design.md, Series): a series, its versions and its members, the version
-- each entry cites, and `waiting_on_series`, the status of an entry that passed
-- screening while its series, or a series another member of its batch cites, is
-- still hidden.
--
-- `entries` is rebuilt for its CHECK constraint, as in 0003, outside the migration
-- transaction with foreign keys off.

PRAGMA foreign_keys = OFF;

BEGIN;

-- The creator alone makes versions and makes the series public. Only the SHA-256
-- of the join key is kept; the key itself is shown once.
CREATE TABLE series (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    key_hash BLOB NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    -- Set once, from the creator's browser session; never unset.
    public_at TEXT
) STRICT;

-- Every version stays as it was written. Versions count from 1.
CREATE TABLE series_versions (
    series_seq INTEGER NOT NULL REFERENCES series (seq),
    version INTEGER NOT NULL CHECK (version >= 1),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- When anyone could first read it: the series was public, a public entry cited
    -- it, and no citing entry was still unpublished. Never unset, so a readable
    -- version never changes back.
    readable_at TEXT,
    PRIMARY KEY (series_seq, version)
) STRICT;

CREATE TABLE series_members (
    series_seq INTEGER NOT NULL REFERENCES series (seq),
    account_id TEXT NOT NULL REFERENCES accounts (id),
    joined_at TEXT NOT NULL,
    PRIMARY KEY (series_seq, account_id)
) STRICT;

CREATE TABLE entries_new (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    status TEXT NOT NULL CHECK (status IN (
        'pending', 'screening', 'review', 'held', 'frozen', 'embargoed',
        'waiting_on_series', 'public', 'rejected', 'withdrawn', 'removed'
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
    hold_until TEXT,
    previous_seq INTEGER REFERENCES entries (seq),
    batch_seq INTEGER REFERENCES batches (seq),
    -- The series version the entry cites, kept in its own record (design.md, Series).
    series_seq INTEGER REFERENCES series (seq),
    series_version INTEGER,
    series_section TEXT,
    FOREIGN KEY (series_seq, series_version) REFERENCES series_versions (series_seq, version)
) STRICT;

INSERT INTO entries_new (seq, id, account_id, status, source, submitted_at,
    first_submitted_at, embargo_until, published_at, analyzer_version, outcome, kind, stage,
    hold_until, previous_seq, batch_seq)
SELECT seq, id, account_id, status, source, submitted_at,
    first_submitted_at, embargo_until, published_at, analyzer_version, outcome, kind, stage,
    hold_until, previous_seq, batch_seq
FROM entries;

DROP TABLE entries;
ALTER TABLE entries_new RENAME TO entries;

CREATE INDEX entries_status ON entries (status);
CREATE INDEX entries_hold ON entries (hold_until) WHERE status = 'pending';
CREATE UNIQUE INDEX entries_one_successor ON entries (previous_seq)
    WHERE previous_seq IS NOT NULL
      AND status NOT IN ('rejected', 'withdrawn', 'removed');
CREATE INDEX entries_series ON entries (series_seq, series_version)
    WHERE series_seq IS NOT NULL;

COMMIT;

PRAGMA foreign_keys = ON;
