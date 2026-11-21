-- What a set stores besides its files' text (design.md, Attached code).

-- The repos a set's files name, each once with its URL or `no remote`. Before this,
-- each file's row held a copy of its repo's URL (`attachment_files.repo_url`), which
-- rows from then keep; a newer row leaves it NULL and its origin's repo name finds
-- the URL here.
CREATE TABLE attachment_repos (
    set_seq INTEGER NOT NULL REFERENCES attachment_sets (seq) ON DELETE CASCADE,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    PRIMARY KEY (set_seq, name)
) STRICT;

-- The bytes of the set's origins, paths and repos, which count toward the site's
-- storage cap beside its files' bytes. `bytes` holds them too, with the files' text.
-- 0 for the sets stored before this.
ALTER TABLE attachment_sets ADD COLUMN metadata_bytes INTEGER NOT NULL DEFAULT 0;
