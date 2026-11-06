-- Versions (design.md, Entry lifecycle): a new version is a new entry that names the
-- entry it replaces. Each entry has at most one live successor; a rejected or
-- withdrawn one doesn't count, so its author can try again.

ALTER TABLE entries ADD COLUMN previous_seq INTEGER REFERENCES entries (seq);

CREATE UNIQUE INDEX entries_one_successor ON entries (previous_seq)
    WHERE previous_seq IS NOT NULL
      AND status NOT IN ('rejected', 'withdrawn', 'removed');
