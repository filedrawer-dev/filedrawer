-- GitHub sign-in (design.md, Accounts; #119). The server keeps GitHub's numeric user
-- ID and the account's current login, never a GitHub token.

-- An account's linked GitHub account: at most one each way.
CREATE TABLE github_links (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    github_id INTEGER NOT NULL UNIQUE,
    login TEXT NOT NULL,
    -- When the GitHub account was made, as GitHub said at linking; NULL when its
    -- answer had none the server could read, which grants no exemption.
    github_created_at TEXT,
    linked_at TEXT NOT NULL,
    -- The last answer GitHub gave about the account, and its ETag, for the refresh.
    refreshed_at TEXT NOT NULL,
    etag TEXT,
    -- Set while GitHub answers 404 for the ID, which also happens to live accounts,
    -- so the handle hides until a later answer brings it back.
    hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1))
) STRICT;

-- The account each GitHub ID first linked: only that account skips the new-account
-- limit for it (design.md, Limits and abuse). Unlinking leaves the row; deleting the
-- account clears `account_id`, so the bare ID still refuses a second exemption.
CREATE TABLE github_exemptions (
    github_id INTEGER PRIMARY KEY,
    account_id TEXT REFERENCES accounts (id) ON DELETE CASCADE
) STRICT;

-- A sign-in or link under way: the browser holds the secret whose hash is
-- `browser_hash` in a cookie, and GitHub sends `state` back. Each is used once and
-- lives 10 minutes, as GitHub's codes do.
CREATE TABLE github_starts (
    state_hash BLOB PRIMARY KEY,
    browser_hash BLOB NOT NULL,
    verifier TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('sign_in', 'link')),
    -- The session's account, for a link.
    account_id TEXT REFERENCES accounts (id) ON DELETE CASCADE,
    next TEXT,
    -- The client's limits key, as a SHA-256, to cap the starts in progress per IP.
    client_hash BLOB NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
) STRICT;
CREATE INDEX github_starts_by_client ON github_starts (client_hash, expires_at);

-- A GitHub sign-in that found no account, waiting for its finish-signup form: what
-- GitHub said, kept on the server so the form can't change it. The browser holds
-- the key in a cookie.
CREATE TABLE github_signups (
    key_hash BLOB PRIMARY KEY,
    github_id INTEGER NOT NULL,
    login TEXT NOT NULL,
    name TEXT,
    github_created_at TEXT,
    -- The verified addresses GitHub listed that can get mail, as a JSON array of
    -- strings, primary first.
    emails TEXT NOT NULL,
    next TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
) STRICT;

-- The GitHub login to show for each account that has one: none once the account is
-- deleted, none while GitHub's answer hides it, and none when GitHub last confirmed
-- it more than a day ago, since a login renamed away can be claimed by someone else
-- at once. Every read that shows a handle goes through this view, as names go
-- through `authors`.
CREATE VIEW github_handles AS
SELECT l.account_id, l.login
FROM github_links l JOIN accounts a ON a.id = l.account_id
WHERE a.deleted_at IS NULL AND l.hidden = 0
  AND l.refreshed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 day');
