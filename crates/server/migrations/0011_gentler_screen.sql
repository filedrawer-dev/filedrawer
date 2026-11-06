-- A gentler screen (the owner's decisions, 2026-11-06; design.md, Submission and
-- screening).

-- The screen's doubt that an entry holds a hazard, when it passed anyway: the
-- category and the quoted passage. Only the owner sees it, on /admin, until marked
-- seen; it never shows on the card.
ALTER TABLE entries ADD COLUMN hazard_doubt TEXT;
ALTER TABLE entries ADD COLUMN hazard_doubt_seen_at TEXT;

-- The review guard now trips when the owner holds an entry or rejects one in
-- review, not when an entry reaches review. Clear guards set the old way that no
-- held entry and no rejection in review supports.
UPDATE accounts SET review_guard_at = NULL
WHERE review_guard_at IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM entries e
      WHERE e.account_id = accounts.id
        AND (e.status = 'held' OR e.rejection_reasons LIKE '%rejected in review%')
  );
