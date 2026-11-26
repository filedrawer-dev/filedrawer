-- The display name a screen last passed with an account's entry or series (design.md,
-- Accounts; #59). Until it equals `display_name`, the name rides along with the
-- account's next screen, so a name changed since, as when an unclaimed account is
-- claimed, is screened again.
ALTER TABLE accounts ADD COLUMN display_name_screened TEXT;

-- The names stored before the screen read names were public through the beta, as the
-- owner decided (2026-11-09), so they count as screened.
UPDATE accounts SET display_name_screened = display_name WHERE deleted_at IS NULL;

-- Public reads show the name a screen passed, and an account whose name no screen has
-- passed yet shows an empty one, which cards print as "(no name)". `unclaimed` is the
-- server's own word, shown as it is.
DROP VIEW authors;
CREATE VIEW authors AS
SELECT id,
       CASE
           WHEN deleted_at IS NOT NULL THEN NULL
           WHEN display_name = 'unclaimed' THEN display_name
           ELSE IFNULL(display_name_screened, '')
       END AS display_name
FROM accounts;
