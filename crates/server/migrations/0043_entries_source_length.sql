-- Finds the entries whose text could equal a new submission's, for the refusal of
-- identical text from an unclaimed account (design.md, Limits and abuse; #130):
-- those of about its length, which the server then compares in full. The length is
-- in bytes, since `length()` of TEXT stops at a NUL character.
CREATE INDEX entries_source_length ON entries (length(CAST(source AS BLOB)));
