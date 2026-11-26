-- Connecting an agent from the browser with a code (design.md, Accounts; #54;
-- RFC 8628, the device authorization grant).

-- An agent's request for a token, from the moment it asks until it takes the token,
-- its human denies it, or it expires. Only hashes of the two codes are kept: the
-- device code the agent polls with and the user code its human types. The token
-- itself is made, and shown, only when the agent polls after the approval, so the
-- table never holds one. The User-Agent and the name are the agent's own words,
-- shown on the approval page.
CREATE TABLE device_requests (
    device_hash BLOB PRIMARY KEY,
    user_code_hash BLOB NOT NULL UNIQUE,
    -- The name the token will have, which the approval page may change.
    name TEXT NOT NULL,
    -- Cut to 200 characters; none when the agent sent none.
    user_agent TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    -- The seconds the agent must wait between polls, which `slow_down` raises.
    interval_secs INTEGER NOT NULL DEFAULT 5,
    last_polled_at TEXT,
    -- Who approved it and when: the account the token will belong to.
    account_id TEXT REFERENCES accounts (id) ON DELETE CASCADE,
    approved_at TEXT,
    denied_at TEXT
) STRICT;

CREATE INDEX device_requests_by_account ON device_requests (account_id);

-- When a token was made by approving an agent's code rather than on the account
-- page, so the account page can say so.
ALTER TABLE tokens ADD COLUMN from_code_at TEXT;
