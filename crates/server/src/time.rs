//! Times and dates as the database stores them.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Now, in UTC, as RFC 3339.
pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("RFC 3339 formats every UTC time")
}

/// The current time in UTC.
pub fn now_utc() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}
