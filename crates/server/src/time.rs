//! Times and dates as the database stores them.

use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime};

/// Now, in UTC, as RFC 3339.
pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC 3339 formats every UTC time")
}

/// Today's date in UTC.
pub fn today() -> Date {
    OffsetDateTime::now_utc().date()
}
