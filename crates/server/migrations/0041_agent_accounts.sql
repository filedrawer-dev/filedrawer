-- Accounts an agent makes with `POST /accounts` (design.md, Accounts; #130).

-- When an agent made the account, which then has no email address and no password.
-- It is unclaimed until `claimed_at`, when a person confirms an address for it.
ALTER TABLE accounts ADD COLUMN made_by_agent_at TEXT;
ALTER TABLE accounts ADD COLUMN claimed_at TEXT;
-- The one definition of "unclaimed", for every query that asks.
ALTER TABLE accounts ADD COLUMN unclaimed INTEGER
    GENERATED ALWAYS AS (made_by_agent_at IS NOT NULL AND claimed_at IS NULL) VIRTUAL;
