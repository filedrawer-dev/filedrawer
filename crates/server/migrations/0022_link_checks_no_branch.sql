-- A GitHub commit on no branch or tag of the repository it names is flagged, not
-- refused (design.md, Submission and screening, step 2): `pin` gains `no_branch`.
-- SQLite can't change a CHECK constraint, so the table is rebuilt with its rows,
-- and the view over it with them.
DROP VIEW link_flags;

CREATE TABLE link_checks_new (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq) ON DELETE CASCADE,
    field TEXT NOT NULL CHECK (field IN ('code', 'logs')),
    url TEXT NOT NULL,
    reach TEXT NOT NULL CHECK (reach IN ('live', 'dead', 'retry')),
    reach_detail TEXT,
    pin TEXT CHECK (pin IN ('verified', 'not_verified', 'failed', 'no_branch')),
    pin_detail TEXT,
    checked_at TEXT NOT NULL,
    next_check_at TEXT,
    retries INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (entry_seq, field, url)
);
-- The rowid too: rows read back in the order they were stored.
INSERT INTO link_checks_new (rowid, entry_seq, field, url, reach, reach_detail, pin,
                             pin_detail, checked_at, next_check_at, retries)
SELECT rowid, entry_seq, field, url, reach, reach_detail, pin, pin_detail, checked_at,
       next_check_at, retries
FROM link_checks;
DROP TABLE link_checks;
ALTER TABLE link_checks_new RENAME TO link_checks;
CREATE INDEX link_checks_due ON link_checks (next_check_at)
    WHERE next_check_at IS NOT NULL;

-- A GitHub commit an earlier check called a bad pin, on no branch, is checked again
-- at once, so its card takes the new flag and its warning the new advice.
UPDATE link_checks SET next_check_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE field = 'code' AND pin = 'failed' AND lower(url) LIKE '%://github.com/%';

-- The card's flags for each entry that has link checks, one row per entry, which
-- the public reads join. An entry has one `code` link, so one pin state at most:
-- `pin_flag` is it, unless it is `verified`.
CREATE VIEW link_flags AS
SELECT entry_seq,
       MAX(field = 'code' AND reach = 'dead') AS code_link_dead,
       MAX(field = 'logs' AND reach = 'dead') AS logs_link_dead,
       MAX(CASE WHEN pin != 'verified' THEN pin END) AS pin_flag
FROM link_checks
GROUP BY entry_seq;

-- The commits GitHub's API confirmed now include those at a tag.
ALTER TABLE github_commits_on_branches RENAME TO github_commits_confirmed;
