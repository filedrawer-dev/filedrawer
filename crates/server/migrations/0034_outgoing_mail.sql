-- Email waiting to be sent, retried until it goes or a day passes (#103).
-- A verify or reset row holds no link: the sender mints one when it sends, and
-- retires it when the send fails. No row holds an address; the sender reads the
-- account's.
CREATE TABLE outgoing_mail (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('verify', 'reset', 'taken', 'notice')),
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
CREATE INDEX outgoing_mail_due ON outgoing_mail (dropped_at, next_try_at);
CREATE INDEX outgoing_mail_by_account ON outgoing_mail (account_id, kind);
