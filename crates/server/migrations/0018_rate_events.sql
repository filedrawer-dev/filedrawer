-- The hourly and daily rate limits' counts (design.md, Limits and abuse): one row
-- for each signup or account email, under each key it counts against, such as
-- `signup-ip:203.0.113.5` or `email-address:ada@example.com`. Rows past the longest
-- window, a day, are deleted by the hourly tidy.
CREATE TABLE rate_events (
    key TEXT NOT NULL,
    at TEXT NOT NULL
);

CREATE INDEX rate_events_by_key ON rate_events (key, at);
