-- The feedback limit counts an account's items in the last hour and day (#60),
-- inside the write transaction that stores the next one.
CREATE INDEX feedback_by_account ON feedback (account_id, created_at);
