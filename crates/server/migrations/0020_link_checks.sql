-- The link checks (design.md, Submission and screening, step 2): what each `code` and
-- `logs` link of an entry answered when last checked, and, for a `code` link on
-- GitHub or Zenodo, whether its pin was confirmed. The card's flags come from these
-- rows, and the weekly recheck picks the ones that aren't live and confirmed. A
-- link on an unlisted host has no row: the server never fetches it. Rows go with
-- their entry.
CREATE TABLE link_checks (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq) ON DELETE CASCADE,
    field TEXT NOT NULL CHECK (field IN ('code', 'logs')),
    url TEXT NOT NULL,
    -- `retry`: it answered 408, 429 or 5xx, or not at all; not dead.
    reach TEXT NOT NULL CHECK (reach IN ('live', 'dead', 'retry')),
    -- What it answered, for the author's warnings, when not `live`.
    reach_detail TEXT,
    pin TEXT CHECK (pin IN ('verified', 'not_verified', 'failed')),
    pin_detail TEXT,
    checked_at TEXT NOT NULL,
    PRIMARY KEY (entry_seq, field, url)
);
CREATE INDEX link_checks_flagged ON link_checks (checked_at)
    WHERE reach != 'live' OR pin IN ('not_verified', 'failed');

-- Commits GitHub's API confirmed are on a branch of the repository a link names, so
-- a resubmission or a recheck spends no API calls on them. A commit found on no
-- branch isn't kept: its author may push it to one and submit again.
CREATE TABLE github_commits_on_branches (
    -- `<owner>/<repo>`, lowercased.
    repo TEXT NOT NULL,
    sha TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    PRIMARY KEY (repo, sha)
);

-- The card's flags for each entry that has link checks, one row per entry, which
-- the public reads join.
CREATE VIEW link_flags AS
SELECT entry_seq,
       MAX(field = 'code' AND reach = 'dead') AS code_link_dead,
       MAX(field = 'logs' AND reach = 'dead') AS logs_link_dead,
       MAX(IFNULL(pin, '') = 'not_verified') AS pin_not_verified,
       MAX(IFNULL(pin, '') = 'failed') AS pin_failed
FROM link_checks
GROUP BY entry_seq;
