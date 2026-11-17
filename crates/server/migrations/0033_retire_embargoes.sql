-- no-transaction
-- Embargoes and hidden series are retired (design.md, Rejected; the owner's
-- decision, 2026-11-17). Only staging ever held any.
--
-- Every `embargoed` and `waiting_on_series` entry is deleted, with its later
-- versions, which a chain held back with it, and with what was read from them: the
-- cost log and the allowance unit stay with no entry, and the runs and batches only
-- they used go. Their attachment sets are left for the hourly tidier, which expires
-- uncited sets, and no `deleted_entries` record is written, so their pages answer
-- 404 rather than 410. An entry the owner removed from either status reads as
-- removed from `withdrawn`. Every hidden series no one could read yet goes too, and
-- the entries citing it lose the cite; one the owner removed stays, as removed data
-- does, and is public if restored.
--
-- `entries` is rebuilt without the two statuses and `embargo_until`, as in 0006,
-- outside the migration transaction with foreign keys off, keeping every other
-- column, index, foreign key and the trigger from 0026. `series.public_at` goes.

PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TEMP TABLE doomed_entries AS
    WITH RECURSIVE held (seq) AS (
        SELECT seq FROM entries WHERE status IN ('embargoed', 'waiting_on_series')
        UNION
        SELECT e.seq FROM entries e JOIN held h ON e.previous_seq = h.seq
    )
    SELECT seq, batch_seq FROM entries WHERE seq IN (SELECT seq FROM held);

CREATE TEMP TABLE doomed_runs AS
    SELECT DISTINCT account_id, run_id FROM entry_runs
    WHERE entry_seq IN (SELECT seq FROM doomed_entries);

DELETE FROM entry_models WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM entry_tags WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM entry_rows WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM entry_references WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM entry_unknown_fields WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM entry_runs WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM slugs WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM link_checks WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM entries_fts WHERE rowid IN (SELECT seq FROM doomed_entries);
UPDATE screenings SET entry_seq = NULL WHERE entry_seq IN (SELECT seq FROM doomed_entries);
UPDATE allowance_units SET entry_seq = NULL WHERE entry_seq IN (SELECT seq FROM doomed_entries);
DELETE FROM entries WHERE seq IN (SELECT seq FROM doomed_entries);

DELETE FROM runs WHERE EXISTS (
    SELECT 1 FROM doomed_runs d WHERE d.account_id = runs.account_id AND d.run_id = runs.id)
  AND NOT EXISTS (
    SELECT 1 FROM entry_runs er WHERE er.account_id = runs.account_id AND er.run_id = runs.id);
DELETE FROM batches WHERE seq IN (SELECT batch_seq FROM doomed_entries)
  AND NOT EXISTS (SELECT 1 FROM entries e WHERE e.batch_seq = batches.seq);

UPDATE entries SET removed_from = 'withdrawn'
WHERE removed_from IN ('embargoed', 'waiting_on_series');

CREATE TEMP TABLE doomed_series AS
    SELECT seq FROM series s
    WHERE s.public_at IS NULL AND s.removed_at IS NULL
      AND NOT EXISTS (
          SELECT 1 FROM series_versions v
          WHERE v.series_seq = s.seq AND v.readable_at IS NOT NULL);

UPDATE entries SET series_seq = NULL, series_version = NULL, series_section = NULL,
    series_removed_at = NULL
WHERE series_seq IN (SELECT seq FROM doomed_series);
DELETE FROM series_members WHERE series_seq IN (SELECT seq FROM doomed_series);
DELETE FROM series_versions WHERE series_seq IN (SELECT seq FROM doomed_series);
DELETE FROM series WHERE seq IN (SELECT seq FROM doomed_series);

DROP TABLE doomed_entries;
DROP TABLE doomed_runs;
DROP TABLE doomed_series;

ALTER TABLE series DROP COLUMN public_at;

CREATE TABLE entries_new (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    status TEXT NOT NULL CHECK (status IN (
        'pending', 'screening', 'review', 'held', 'frozen', 'public', 'rejected',
        'withdrawn', 'removed'
    )),
    source TEXT NOT NULL,
    submitted_at TEXT NOT NULL,
    first_submitted_at TEXT NOT NULL,
    published_at TEXT,
    analyzer_version INTEGER NOT NULL,
    outcome TEXT,
    kind TEXT,
    stage TEXT,
    hold_until TEXT,
    previous_seq INTEGER REFERENCES entries (seq),
    batch_seq INTEGER REFERENCES batches (seq),
    -- The series version the entry cites, kept in its own record (design.md, Series).
    series_seq INTEGER REFERENCES series (seq),
    series_version INTEGER,
    series_section TEXT,
    series_removed_at TEXT,
    screen_verdict TEXT CHECK (screen_verdict IN ('pass', 'reject', 'review')),
    screening_note TEXT,
    rejection_reasons TEXT,
    screen_attempts INTEGER NOT NULL DEFAULT 0,
    queue_position INTEGER,
    hazard_doubt TEXT,
    hazard_doubt_seen_at TEXT,
    attachment_set_seq INTEGER REFERENCES attachment_sets (seq),
    review_reason TEXT,
    token_id TEXT REFERENCES tokens (id),
    removed_from TEXT,
    removed_at TEXT,
    skill TEXT,
    user_agent TEXT,
    FOREIGN KEY (series_seq, series_version) REFERENCES series_versions (series_seq, version)
) STRICT;

INSERT INTO entries_new (seq, id, account_id, status, source, submitted_at,
    first_submitted_at, published_at, analyzer_version, outcome, kind, stage, hold_until,
    previous_seq, batch_seq, series_seq, series_version, series_section, series_removed_at,
    screen_verdict, screening_note, rejection_reasons, screen_attempts, queue_position,
    hazard_doubt, hazard_doubt_seen_at, attachment_set_seq, review_reason, token_id,
    removed_from, removed_at, skill, user_agent)
SELECT seq, id, account_id, status, source, submitted_at,
    first_submitted_at, published_at, analyzer_version, outcome, kind, stage, hold_until,
    previous_seq, batch_seq, series_seq, series_version, series_section, series_removed_at,
    screen_verdict, screening_note, rejection_reasons, screen_attempts, queue_position,
    hazard_doubt, hazard_doubt_seen_at, attachment_set_seq, review_reason, token_id,
    removed_from, removed_at, skill, user_agent
FROM entries;

DROP TABLE entries;
ALTER TABLE entries_new RENAME TO entries;

CREATE INDEX entries_status ON entries (status);
CREATE INDEX entries_hold ON entries (hold_until) WHERE status = 'pending';
CREATE UNIQUE INDEX entries_one_successor ON entries (previous_seq)
    WHERE previous_seq IS NOT NULL
      AND status NOT IN ('rejected', 'withdrawn', 'removed');
CREATE INDEX entries_series ON entries (series_seq, series_version)
    WHERE series_seq IS NOT NULL;

CREATE TRIGGER entries_not_from_suspended BEFORE INSERT ON entries
WHEN (SELECT suspended_at FROM accounts WHERE id = NEW.account_id) IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'the account is suspended');
END;

COMMIT;

PRAGMA foreign_keys = ON;
