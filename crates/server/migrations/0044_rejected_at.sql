-- When an entry was rejected, so the hourly tidy deletes it 60 days later
-- (design.md, Entry lifecycle; #132). Entries already rejected count from now, so
-- the first tidy after the deploy deletes none of them. One an older binary rejects
-- after a rollback has none, and its latest submission stands in.
ALTER TABLE entries ADD COLUMN rejected_at TEXT;
UPDATE entries SET rejected_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE status = 'rejected';
