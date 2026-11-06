-- Attached code (design.md, Attached code; the owner's decisions, 2026-11-06).

-- Each file once, by the SHA-256 of its text, with the screen's verdict on it, so a
-- file attached to many entries is stored and screened once.
CREATE TABLE files (
    sha256 TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    screen_verdict TEXT CHECK (screen_verdict IN ('pass', 'reject', 'review')),
    -- Why the screen rejected it, or sent it to review: the kind, never a passage.
    screen_reason TEXT
) STRICT;

-- A set of files uploaded together and cited by entries as `attachments: as-...`.
-- A set never changes.
CREATE TABLE attachment_sets (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    -- The set's files' bytes in all, counted toward the account's day.
    bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE attachment_files (
    set_seq INTEGER NOT NULL REFERENCES attachment_sets (seq),
    path TEXT NOT NULL,
    -- `<repo>@<commit>:<path>` or `author:<path>`, as sent.
    origin TEXT NOT NULL,
    -- The repo's URL, or `no remote`, when `<repo>` was a name the set declared.
    repo_url TEXT,
    sha256 TEXT NOT NULL REFERENCES files (sha256),
    PRIMARY KEY (set_seq, path)
) STRICT;

CREATE INDEX attachment_files_sha ON attachment_files (sha256);
CREATE INDEX attachment_sets_account ON attachment_sets (account_id, created_at);

ALTER TABLE entries ADD COLUMN attachment_set_seq INTEGER REFERENCES attachment_sets (seq);

-- 1 for a call that screened attached code, which may spend at most half the cap.
ALTER TABLE screenings ADD COLUMN attachment INTEGER NOT NULL DEFAULT 0
    CHECK (attachment IN (0, 1));

-- Notices sent to the owner, at most one of each kind a day, such as attachment
-- storage being full.
CREATE TABLE owner_notices (
    kind TEXT NOT NULL,
    day TEXT NOT NULL,
    PRIMARY KEY (kind, day)
) STRICT;
