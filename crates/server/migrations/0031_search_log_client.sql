-- The User-Agent of a search sent by File Drawer's own skills, which name the
-- agent's harness and model (design.md, Skills): stored only when it starts with
-- `filedrawer-skill`, so a browser's identifying string never sits beside a query.
ALTER TABLE search_log ADD COLUMN client TEXT;
