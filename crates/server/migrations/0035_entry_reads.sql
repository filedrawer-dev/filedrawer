-- The read counter behind the home page's Most read tab (design.md, Agent interface;
-- #107). A reader of an entry, on one UTC day, as a hash salted with a key that
-- lives only in the server's memory, until the hourly tidier closes the day.
CREATE TABLE entry_readers (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq) ON DELETE CASCADE,
    day TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('agent', 'page')),
    reader BLOB NOT NULL,
    PRIMARY KEY (entry_seq, day, kind, reader)
) WITHOUT ROWID;

CREATE INDEX entry_readers_by_day ON entry_readers (day);

-- Each entry's readers of each kind on a closed day, for the days Most read ranks by.
CREATE TABLE entry_reads_daily (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq) ON DELETE CASCADE,
    day TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('agent', 'page')),
    readers INTEGER NOT NULL,
    PRIMARY KEY (entry_seq, day, kind)
) WITHOUT ROWID;

CREATE INDEX entry_reads_daily_by_day ON entry_reads_daily (day);

-- The same per calendar month (YYYY-MM), once a day leaves Most read's window; kept
-- for the owner to read through SQL.
CREATE TABLE entry_reads_monthly (
    entry_seq INTEGER NOT NULL REFERENCES entries (seq) ON DELETE CASCADE,
    month TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('agent', 'page')),
    readers INTEGER NOT NULL,
    PRIMARY KEY (entry_seq, month, kind)
) WITHOUT ROWID;

-- Each UTC day's totals across entries: the readers counted, once the day is closed,
-- and the reads past the daily cap that went uncounted, for the owner's daily email.
CREATE TABLE read_days (
    day TEXT PRIMARY KEY,
    readers INTEGER NOT NULL DEFAULT 0,
    dropped INTEGER NOT NULL DEFAULT 0
);
