-- Why the screen sent an entry to the owner's review, for the owner's page: a
-- hazard's kind, a limit an attached file passed, or the review guard. Never the
-- hazard's passage. NULL for entries that went to review before reasons were kept.
ALTER TABLE entries ADD COLUMN review_reason TEXT;
