-- An account with no password confirms a change of its email address with a fresh
-- GitHub round trip (design.md, Accounts; #125): `purpose` gains `change_email`,
-- whose start holds the address asked for until GitHub's answer comes back.
-- SQLite can't widen an unnamed CHECK, so the table is rebuilt with its rows, which
-- live 10 minutes, and its index.
CREATE TABLE github_starts_new (
    state_hash BLOB PRIMARY KEY,
    browser_hash BLOB NOT NULL,
    verifier TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('sign_in', 'link', 'change_email')),
    -- The session's account, for a link or a change of address.
    account_id TEXT REFERENCES accounts (id) ON DELETE CASCADE,
    next TEXT,
    -- The client's limits key, as a SHA-256, to cap the starts in progress per IP.
    client_hash BLOB NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    -- The address a change of address asked for, lower-cased and trimmed.
    new_email TEXT
) STRICT;
INSERT INTO github_starts_new (state_hash, browser_hash, verifier, purpose, account_id,
                               next, client_hash, created_at, expires_at)
SELECT state_hash, browser_hash, verifier, purpose, account_id, next, client_hash,
       created_at, expires_at
FROM github_starts;
DROP TABLE github_starts;
ALTER TABLE github_starts_new RENAME TO github_starts;
CREATE INDEX github_starts_by_client ON github_starts (client_hash, expires_at);
