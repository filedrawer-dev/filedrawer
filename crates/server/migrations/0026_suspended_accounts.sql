-- Set while the owner suspends the account (design.md, Admin): it can't submit, and
-- its entries were removed when the suspension began. Lifting it clears this only.
ALTER TABLE accounts ADD COLUMN suspended_at TEXT;

-- A suspended account's token writes are refused before they start (`auth.rs`); this
-- stops one that passed that check before the suspension committed, since every
-- insert waits for the suspension's write lock. Any path that adds an entry is
-- covered, including ones added later.
CREATE TRIGGER entries_not_from_suspended BEFORE INSERT ON entries
WHEN (SELECT suspended_at FROM accounts WHERE id = NEW.account_id) IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'the account is suspended');
END;
