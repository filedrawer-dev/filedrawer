-- Changing an account's email address (design.md, Accounts; #125).

-- An account's pending change, at most one: the address it asked to change to, and
-- the hash of the link emailed there, set when that email is written. A new request
-- replaces the row, and it is deleted at the swap, on cancel, by a password reset,
-- with the account, and once it has expired.
CREATE TABLE email_changes (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    -- Lower-cased and trimmed, as `accounts.email`.
    new_email TEXT NOT NULL,
    link_hash BLOB,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
) STRICT;

-- The queue gains two kinds: `change`, the link to the new address, and
-- `change_notice`, the note to the current one. SQLite can't widen an unnamed CHECK,
-- so the table is rebuilt with its rows, ids kept, and its indexes with it.
CREATE TABLE outgoing_mail_new (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('verify', 'reset', 'taken', 'notice', 'change',
                                       'change_notice')),
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
