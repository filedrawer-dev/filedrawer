-- Attached files' bytes move out of the database, onto disk beside it (design.md,
-- Attached code; the owner's decision, 2026-11-06, issue #12). Staging's few sets
-- were test data, so they go rather than being copied out (the owner's decision,
-- 2026-11-06); production held none.

UPDATE entries SET attachment_set_seq = NULL;
DELETE FROM attachment_files;
DELETE FROM attachment_sets;
DELETE FROM files;
ALTER TABLE files DROP COLUMN text;
