-- Screening (design.md, Submission and screening): each entry's verdict, the note
-- on its card, the reasons for a rejection, and every model call's cost.

-- 'pass', 'reject' or 'review' once screened. A batch member keeps its verdict while
-- it waits for the rest of its batch.
ALTER TABLE entries ADD COLUMN screen_verdict TEXT
    CHECK (screen_verdict IN ('pass', 'reject', 'review'));
-- The screen's doubt that the body matches `outcome`, shown on the card.
ALTER TABLE entries ADD COLUMN screening_note TEXT;
-- A rejection's reasons, a JSON array of strings, for the author.
ALTER TABLE entries ADD COLUMN rejection_reasons TEXT;
-- Calls that failed while screening it; at the limit it goes to review.
ALTER TABLE entries ADD COLUMN screen_attempts INTEGER NOT NULL DEFAULT 0;

-- One row per model call. `entry_seq` is empty for a series screened inline.
CREATE TABLE screenings (
    id INTEGER PRIMARY KEY,
    entry_seq INTEGER REFERENCES entries (seq),
    account_id TEXT NOT NULL REFERENCES accounts (id),
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    -- Billionths of a dollar.
    cost_nano_usd INTEGER NOT NULL,
    -- UTC, RFC 3339; the day's cap counts by its first 10 characters.
    at TEXT NOT NULL
) STRICT;

CREATE INDEX screenings_at ON screenings (at);
