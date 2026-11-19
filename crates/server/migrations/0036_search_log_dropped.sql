-- The searches past the search log's daily cap that went unlogged, per UTC day, for
-- the owner's daily email (#107).
CREATE TABLE search_log_dropped (
    day TEXT PRIMARY KEY,
    dropped INTEGER NOT NULL DEFAULT 0
);
