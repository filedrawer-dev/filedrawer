-- Invite codes (design.md, Limits and abuse): the owner makes one with a label, a
-- number of uses and an expiry, and an account that signs up with it skips the
-- per-IP signup and email limits and the new-account allowance. Kept in plain, since
-- the owner reads them off the admin page to share them.
CREATE TABLE invite_codes (
    seq INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    max_uses INTEGER NOT NULL CHECK (max_uses > 0),
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);

-- The code an account signed up with, if any.
ALTER TABLE accounts ADD COLUMN invite_seq INTEGER REFERENCES invite_codes(seq);
CREATE INDEX accounts_by_invite ON accounts (invite_seq);
