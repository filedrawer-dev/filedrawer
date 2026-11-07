-- The daily allowance (design.md, Limits and abuse): one row for each unit an
-- account's accepted submissions took, dated to the UTC day (YYYY-MM-DD) it counts
-- against. An entry isn't screened before the earliest day among its units. A unit
-- with no entry was taken by a series or a series version, which is screened at once.
CREATE TABLE allowance_units (
    seq INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    day TEXT NOT NULL,
    entry_seq INTEGER REFERENCES entries(seq)
);

CREATE INDEX allowance_units_by_day ON allowance_units (account_id, day);
CREATE INDEX allowance_units_by_entry ON allowance_units (entry_seq);

-- The day each entry's screening waits for: the earliest day among its units.
CREATE VIEW entry_screen_from AS
    SELECT entry_seq, MIN(day) AS day FROM allowance_units
    WHERE entry_seq IS NOT NULL GROUP BY entry_seq;
