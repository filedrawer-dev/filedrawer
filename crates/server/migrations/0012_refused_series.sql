-- Series the screen refused as a likely hazard, kept for the owner (the owner's
-- decision, 2026-11-06; design.md, Series). The owner confirms one, which puts the
-- account under the review guard, or dismisses it; either takes it off /admin.
CREATE TABLE refused_series (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id),
    -- The series it would have versioned, for a refused new version.
    series_id TEXT,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    -- The screen's reason: the hazard's kind, never the passage.
    why TEXT NOT NULL,
    refused_at TEXT NOT NULL,
    decision TEXT CHECK (decision IN ('confirmed', 'dismissed')),
    decided_at TEXT
) STRICT;
