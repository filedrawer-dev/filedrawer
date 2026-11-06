-- The fair queue (design.md, Submission and screening): an entry's place in the
-- queue it waits in, set when it joins the screening queue and again when it joins
-- the review queue, and kept while it waits there. Waiting items sort by it, then by
-- submission time, so every account's first item goes ahead of anyone's second.
ALTER TABLE entries ADD COLUMN queue_position INTEGER;
