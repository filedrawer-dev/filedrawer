-- The owner's removals of attached files (design.md, Attached code): a removed
-- file is served for no entry, and its bytes and record stay for analysis.
ALTER TABLE files ADD COLUMN removed_at TEXT;

-- Hashes the owner removed as a hazard: refused at upload and when an entry cites a
-- set holding one, and never served. Kept apart from `files`, so a ban outlives the
-- file's record.
CREATE TABLE banned_files (
    sha256 TEXT PRIMARY KEY,
    banned_at TEXT NOT NULL
) STRICT;
