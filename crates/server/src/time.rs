//! Times and dates as the database stores them: UTC, to the second, always the same
//! width, so stored times compare correctly as text.

use time::OffsetDateTime;
use time::macros::format_description;

/// `at` as the database stores it, such as `2026-11-05T22:58:52Z`.
pub fn stamp(at: OffsetDateTime) -> String {
    at.to_offset(time::UtcOffset::UTC)
        .format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
        ))
        .expect("every UTC time formats")
}

/// A time the database stored with [`stamp`], or `None` if `text` isn't one.
pub fn parse(text: &str) -> Option<OffsetDateTime> {
    time::PrimitiveDateTime::parse(
        text,
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z"),
    )
    .ok()
    .map(time::PrimitiveDateTime::assume_utc)
}

/// Now, as the database stores it.
pub fn now() -> String {
    stamp(now_utc())
}

/// The current time in UTC.
pub fn now_utc() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}
