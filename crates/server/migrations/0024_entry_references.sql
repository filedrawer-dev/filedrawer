-- What each entry's `replicates` and `reanalyzes` name, one row per item with a
-- key, for search's `cites` filter (design.md, Agent interface). The keys are the
-- analyzer's (`Reference::key`), so the server rebuilds the table when their format
-- changes, and fills it the first time for the entries stored before it.
CREATE TABLE entry_references (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq) ON DELETE CASCADE,
    relation TEXT NOT NULL CHECK (relation IN ('replicates', 'reanalyzes')),
    -- The item's place in its field, from 1, counting items with no key too.
    position INTEGER NOT NULL,
    key TEXT NOT NULL,
    PRIMARY KEY (entry_seq, relation, position)
) STRICT;

CREATE INDEX entry_references_key ON entry_references (key);

-- The format version of each table derived from entries' text whose format can
-- change without a migration: `entry_references` holds `REFERENCE_KEYS_VERSION`'s.
CREATE TABLE derived_versions (
    name TEXT PRIMARY KEY,
    version INTEGER NOT NULL
) STRICT;
