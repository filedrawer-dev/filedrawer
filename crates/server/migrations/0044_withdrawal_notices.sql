-- A public version its author withdrew (design.md, Entry lifecycle; #31): the entry
-- keeps its row, as `withdrawn`, and its page shows this notice in its place. The
-- reason shows once the screen passes it. `screen` is NULL until it is screened,
-- then `pass`; `reject`, with `screen_reasons` for the author; or `review`, held
-- for the archive's owner, with why in `screen_reasons`. `screen_attempts` counts
-- the failed calls that count against it, as `entries.screen_attempts` does.
CREATE TABLE withdrawal_notices (
    entry_seq INTEGER PRIMARY KEY REFERENCES entries (seq) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    withdrawn_at TEXT NOT NULL,
    screen TEXT CHECK (screen IN ('pass', 'reject', 'review')),
    screen_reasons TEXT,
    screened_at TEXT,
    screen_attempts INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX withdrawal_notices_unscreened ON withdrawal_notices (withdrawn_at)
    WHERE screen IS NULL;

-- A model call on a notice's reason, which the cost log names with the notice's
-- entry, is marked so, apart from calls on the entry's text.
ALTER TABLE screenings ADD COLUMN notice INTEGER NOT NULL DEFAULT 0;
