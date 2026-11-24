-- Claiming an account an agent made (design.md, Accounts; #130).

-- An unclaimed account's pending claim, at most one: the address its agent named,
-- the hash of the code the agent showed its human, the hash of the link emailed to
-- the address, set when that email is written, and how many wrong codes the link's
-- page was given. A new request replaces the row, and it is deleted at the claim,
-- after 5 wrong codes, with the account, and once it has expired.
CREATE TABLE account_claims (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    -- Lower-cased and trimmed, as `accounts.email`.
    email TEXT NOT NULL,
    code_hash BLOB NOT NULL,
    link_hash BLOB,
    wrong_codes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
) STRICT;

-- The queue gains a kind: `claim`, the link to the address a claim names. SQLite
-- can't widen an unnamed CHECK, so the table is rebuilt with its rows, ids kept,
-- and its indexes with it.
CREATE TABLE outgoing_mail_new (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('verify', 'reset', 'taken', 'notice', 'change',
                                       'change_notice', 'claim')),
    -- A notice's own text; the other kinds are written when sent.
    subject TEXT,
    body TEXT,
    created_at TEXT NOT NULL,
    next_try_at TEXT NOT NULL,
    tries INTEGER NOT NULL DEFAULT 0,
    -- The class of the last failure, such as `connect` or `refused 550`, never
    -- the error's text, which can hold the address.
    last_error TEXT,
    dropped_at TEXT
);
INSERT INTO outgoing_mail_new (id, account_id, kind, subject, body, created_at,
                               next_try_at, tries, last_error, dropped_at)
SELECT id, account_id, kind, subject, body, created_at, next_try_at, tries, last_error,
       dropped_at
FROM outgoing_mail;
DROP TABLE outgoing_mail;
ALTER TABLE outgoing_mail_new RENAME TO outgoing_mail;
CREATE INDEX outgoing_mail_due ON outgoing_mail (dropped_at, next_try_at);
CREATE INDEX outgoing_mail_by_account ON outgoing_mail (account_id, kind);
