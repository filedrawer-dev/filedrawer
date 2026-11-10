-- The owner's daily email (design.md, Admin): when it last ran, so a restart neither
-- runs it twice in a UTC day nor skips one, and the period each admin was last sent,
-- so a run that reached one admin and failed for another retries only the other.
CREATE TABLE daily_email_runs (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    -- UTC, RFC 3339: the end of the last period the job reported, or found quiet.
    last_run_at TEXT NOT NULL
) STRICT;

CREATE TABLE daily_email_sent (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id),
    -- The start of the last period this admin was sent.
    period_start TEXT NOT NULL
) STRICT;
