-- The skill version and User-Agent of the request that wrote each entry's text
-- (#45), for the owner to read through SQL. Nothing shows them. NULL when the
-- request sent none, or a skill header not shaped `name/N`.
ALTER TABLE entries ADD COLUMN skill TEXT;
ALTER TABLE entries ADD COLUMN user_agent TEXT;
