-- Account deletion (design.md, Accounts) and taking an entry out of its series
-- (design.md, Series).

-- Set when the account is deleted. Its email, login and name are gone by then; the
-- row stays because its public entries still name the account ID.
ALTER TABLE accounts ADD COLUMN deleted_at TEXT;

-- Set when the series' creator takes the entry out. The entry keeps the version it
-- cites, so its page still shows that description.
ALTER TABLE entries ADD COLUMN series_removed_at TEXT;

-- The name to show for each account: none once it is deleted. Every read that shows
-- a name goes through this view, so none can forget the check.
CREATE VIEW authors AS
SELECT id, CASE WHEN deleted_at IS NULL THEN display_name END AS display_name
FROM accounts;
