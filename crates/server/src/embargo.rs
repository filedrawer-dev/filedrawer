//! Reading `embargo_until` at submission (design.md, Embargo).

use time::{Date, Month, OffsetDateTime, Time, UtcOffset, macros::format_description};

/// What an entry's `embargo_until` means for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// No embargo: public once it passes screening.
    Now,
    /// Held until this date's release time.
    On(Date),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmbargoError {
    /// The analyzer refuses anything that isn't a YYYY-MM-DD date first; this guards
    /// against the two disagreeing.
    #[error("`embargo_until: {0}` is not a date written YYYY-MM-DD")]
    NotADate(String),
    #[error(
        "`embargo_until: {date}` is more than 3 months away; the latest a first embargo may run is {latest}"
    )]
    TooLong { date: Date, latest: Date },
    #[error(
        "a token can't remove an entry's embargo; release it early from a browser session once that exists"
    )]
    Dropped,
    #[error(
        "a token can move an embargo earlier only to {earliest} or later, 7 days from now; `embargo_until: {date}` is sooner"
    )]
    TooSoon { date: Date, earliest: Date },
    /// Most likely a typo in the date, and publishing early can't be undone.
    #[error("`embargo_until: {0}` has passed; to publish at once, leave embargo_until out")]
    Passed(Date),
}

/// When an embargo dated `date` ends: 23:59 at UTC−12 on that date, when the day has
/// ended everywhere on earth.
pub fn release_time(date: Date) -> OffsetDateTime {
    let utc_minus_12 = UtcOffset::from_hms(-12, 0, 0).expect("−12 hours is a valid offset");
    date.with_time(Time::from_hms(23, 59, 0).expect("23:59 is a time"))
        .assume_offset(utc_minus_12)
}

/// When the 12-month cap ends for a chain first submitted at `first_submitted`: the
/// release time of the date 12 months later (UTC), so the cap counts as an embargo
/// date does (design.md, Embargo; Series).
pub fn cap_time(first_submitted: OffsetDateTime) -> OffsetDateTime {
    release_time(months_later(
        first_submitted.to_offset(UtcOffset::UTC).date(),
        12,
    ))
}

/// Reads `embargo_until` at `now`, for an entry first submitted at `first_submitted`.
/// A date more than 3 months after the first submission (UTC), or one whose release
/// time has passed, is refused. Reason for counting from the first submission:
/// resubmitting mustn't stretch the cap (design.md, Entry lifecycle).
pub fn release(
    embargo_until: Option<&str>,
    now: OffsetDateTime,
    first_submitted: OffsetDateTime,
) -> Result<Release, EmbargoError> {
    let Some(text) = embargo_until else {
        return Ok(Release::Now);
    };
    let date = Date::parse(text.trim(), format_description!("[year]-[month]-[day]"))
        .map_err(|_| EmbargoError::NotADate(text.to_string()))?;
    let latest = months_later(first_submitted.to_offset(UtcOffset::UTC).date(), 3);
    if date > latest {
        return Err(EmbargoError::TooLong { date, latest });
    }
    if now >= release_time(date) {
        return Err(EmbargoError::Passed(date));
    }
    Ok(Release::On(date))
}

/// `embargo_until` as a date, if it is one. Responses print this, never the author's
/// text, above the author-text delimiter.
pub fn parse_date(embargo_until: Option<&str>) -> Option<Date> {
    Date::parse(
        embargo_until?.trim(),
        format_description!("[year]-[month]-[day]"),
    )
    .ok()
}

/// The date an entry stays embargoed until at `now`, if its `embargo_until` is a date
/// whose release time is still ahead. For an entry already accepted, whose date was
/// checked at submission.
pub fn embargoed_until(embargo_until: Option<&str>, now: OffsetDateTime) -> Option<Date> {
    let date = parse_date(embargo_until)?;
    (now < release_time(date)).then_some(date)
}

/// How soon a token may bring an embargo forward (design.md, Embargo).
const TOKEN_SHORTEN_DAYS: i64 = 7;

/// Whether a token may change an entry's embargo from `old` to `new` (design.md,
/// Accounts and Agent interface): it may add or extend one, and shorten one to no
/// sooner than 7 days from today, but never remove one. Reason: entry text can tell
/// a reading agent to release its embargoes.
pub fn token_may_change(
    old: Option<&str>,
    new: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), EmbargoError> {
    let Some(old) = parse_date(old) else {
        return Ok(());
    };
    let Some(new) = parse_date(new) else {
        return Err(EmbargoError::Dropped);
    };
    let earliest = now.to_offset(UtcOffset::UTC).date() + time::Duration::days(TOKEN_SHORTEN_DAYS);
    if new >= old || new >= earliest {
        Ok(())
    } else {
        Err(EmbargoError::TooSoon {
            date: new,
            earliest,
        })
    }
}

/// The same day `months` later, or the month's last day when it is shorter.
fn months_later(date: Date, months: u8) -> Date {
    let zero_based = u32::from(u8::from(date.month())) - 1 + u32::from(months);
    let year = date.year() + i32::try_from(zero_based / 12).expect("a few months fit in an i32");
    let month = Month::try_from(u8::try_from(zero_based % 12 + 1).expect("1 to 12 fit in a u8"))
        .expect("1 to 12 is a month");
    let day = date.day().min(time::util::days_in_month(month, year));
    Date::from_calendar_date(year, month, day).expect("the day was clamped to the month")
}
