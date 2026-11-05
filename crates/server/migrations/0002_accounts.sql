-- Email and password sign-in, sessions, and the one-time tokens emails carry
-- (design.md, Accounts). Accounts made by `filedrawer admin create-account` have no
-- email or password and count as verified: the owner made them.

ALTER TABLE accounts ADD COLUMN email TEXT;
ALTER TABLE accounts ADD COLUMN password_hash TEXT;
ALTER TABLE accounts ADD COLUMN email_verified_at TEXT;

UPDATE accounts SET email_verified_at = created_at WHERE email IS NULL;

-- Emails are stored lower-cased and trimmed.
CREATE UNIQUE INDEX accounts_email ON accounts (email) WHERE email IS NOT NULL;

-- Only the SHA-256 of a session ID is kept; the ID itself lives in the cookie.
CREATE TABLE sessions (
    hash BLOB PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
) STRICT;

CREATE INDEX sessions_account ON sessions (account_id);

-- Links in verification and password-reset emails. Used once, valid for 24 hours.
CREATE TABLE email_tokens (
    hash BLOB PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    purpose TEXT NOT NULL CHECK (purpose IN ('verify', 'reset')),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at TEXT
) STRICT;
