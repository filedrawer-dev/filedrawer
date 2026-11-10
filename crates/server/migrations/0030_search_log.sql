-- Each search that ran (#45): its query, how many public entries matched, and the
-- UTC day, with no IP, account or token. The hourly tidy deletes rows older than
-- 90 days, and a day takes at most 10,000.
CREATE TABLE search_log (
    -- YYYY-MM-DD, UTC.
    day TEXT NOT NULL,
    -- The filters, form-encoded in a fixed order, cut to 500 characters.
    query TEXT NOT NULL,
    hits INTEGER NOT NULL
) STRICT;

CREATE INDEX search_log_day ON search_log (day);
