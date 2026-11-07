-- When each link is checked again (design.md, Submission and screening, step 2): a
-- dead link or a pin found bad in a week; a link that said "retry later", or a pin
-- the server couldn't check, an hour after the check, doubling with each retry in
-- a row (`retries`), at most a week; a live link with its pin verified, never.
ALTER TABLE link_checks ADD COLUMN next_check_at TEXT;
ALTER TABLE link_checks ADD COLUMN retries INTEGER NOT NULL DEFAULT 0;

UPDATE link_checks SET next_check_at = CASE
    WHEN reach = 'dead' OR pin = 'failed'
        THEN strftime('%Y-%m-%dT%H:%M:%SZ', checked_at, '+7 days')
    WHEN reach = 'retry' OR pin = 'not_verified'
        THEN strftime('%Y-%m-%dT%H:%M:%SZ', checked_at, '+1 hour')
    END;

DROP INDEX link_checks_flagged;
CREATE INDEX link_checks_due ON link_checks (next_check_at)
    WHERE next_check_at IS NOT NULL;
