-- The owner removing an entry (design.md, Admin): a removed entry is served nowhere
-- and stays for analysis. `removed_from` is the status it had, which a restore puts
-- back; both columns are NULL unless the status is `removed`.
ALTER TABLE entries ADD COLUMN removed_from TEXT;
ALTER TABLE entries ADD COLUMN removed_at TEXT;
