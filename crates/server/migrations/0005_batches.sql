-- Batches, slugs and runs (design.md, Agent interface).

-- A batch's members are held and published together.
CREATE TABLE batches (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    created_at TEXT NOT NULL
) STRICT;

ALTER TABLE entries ADD COLUMN batch_seq INTEGER REFERENCES batches (seq);

-- A slug names a chain of versions, by its first entry, among one account's
-- entries. A new version keeps its chain's slug.
CREATE TABLE slugs (
    account_id TEXT NOT NULL REFERENCES accounts (id),
    slug TEXT NOT NULL,
    entry_seq INTEGER NOT NULL REFERENCES entries (seq),
    PRIMARY KEY (account_id, slug)
) STRICT;

-- A run, declared once by a batch, cited by ID afterwards.
CREATE TABLE runs (
    account_id TEXT NOT NULL REFERENCES accounts (id),
    id TEXT NOT NULL,
    -- The declaration as the batch sent it, a JSON object.
    declaration TEXT NOT NULL,
    PRIMARY KEY (account_id, id)
) STRICT;

CREATE TABLE entry_runs (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq),
    account_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    FOREIGN KEY (account_id, run_id) REFERENCES runs (account_id, id)
) STRICT;
