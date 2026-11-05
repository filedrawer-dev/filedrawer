-- Accounts, tokens, entries and their search index, and private feedback.
-- Times are UTC in RFC 3339; dates are YYYY-MM-DD.

CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 50),
    created_at TEXT NOT NULL
) STRICT;

-- Only the SHA-256 of a token is kept; the token itself is shown once.
CREATE TABLE tokens (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    name TEXT NOT NULL,
    hash BLOB NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    revoked_at TEXT
) STRICT;

-- One row per entry. `source` is the stored text (analyzer::stored_text); the
-- columns after it copy what search filters on, so reads never parse YAML to filter.
CREATE TABLE entries (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    status TEXT NOT NULL CHECK (status IN (
        'screening', 'review', 'held', 'frozen', 'embargoed',
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
    stage TEXT
) STRICT;

CREATE INDEX entries_status ON entries (status);

CREATE TABLE entry_models (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq),
    name TEXT NOT NULL,
    role TEXT
) STRICT;

CREATE INDEX entry_models_name ON entry_models (name);

CREATE TABLE entry_tags (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq),
    tag TEXT NOT NULL
) STRICT;

CREATE INDEX entry_tags_tag ON entry_tags (tag);

-- Each Results row's model and claim, for "rows for model M that claim no_effect".
CREATE TABLE entry_rows (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq),
    position INTEGER NOT NULL,
    model TEXT NOT NULL,
    claim TEXT
) STRICT;

CREATE INDEX entry_rows_model ON entry_rows (model);

-- Front-matter fields the server doesn't know, for the admin report.
CREATE TABLE entry_unknown_fields (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq),
    name TEXT NOT NULL
) STRICT;

-- Holds public entries only, so embargoed text can never match a search. Its rowid
-- is entries.seq.
CREATE VIRTUAL TABLE entries_fts USING fts5 (
    title, hypothesis, safety_motivation, body,
    tokenize = 'porter unicode61'
);

CREATE TABLE feedback (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    experiment TEXT NOT NULL,
    wanted TEXT NOT NULL,
    workaround TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
