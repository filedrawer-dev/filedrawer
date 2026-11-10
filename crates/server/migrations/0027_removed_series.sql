-- The owner removing a series (design.md, Admin; #57): a removed series is served
-- nowhere, and its title and description show on no entry. NULL unless removed; a
-- restore clears it.
ALTER TABLE series ADD COLUMN removed_at TEXT;
