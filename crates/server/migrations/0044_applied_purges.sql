-- The lines of the purge log (`purges.log` beside the database) this database has
-- applied, by each line's SHA-256 (purge.rs): a database restored to an older point
-- lacks the newer ones, which the server applies when it starts.
CREATE TABLE applied_purges (
    line_sha256 TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL
) STRICT;
