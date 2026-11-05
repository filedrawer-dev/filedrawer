//! Reading `embargo_until` at submission (design.md, Embargo).

use time::{Date, Month, OffsetDateTime, Time, UtcOffset, macros::format_description};

/// What an entry's `embargo_until` means for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// No embargo, or one whose release time has passed: public once it passes
    /// screening.
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
}

/// When an embargo dated `date` ends: 23:59 at UTC−12 on that date, when the day has
/// ended everywhere on earth.
pub fn release_time(date: Date) -> OffsetDateTime {
    let utc_minus_12 = UtcOffset::from_hms(-12, 0, 0).expect("−12 hours is a valid offset");
    date.with_time(Time::from_hms(23, 59, 0).expect("23:59 is a time"))
        .assume_offset(utc_minus_12)
}

/// Reads `embargo_until` at `now`. A date more than 3 months after today (UTC) is
/// refused.
pub fn release(embargo_until: Option<&str>, now: OffsetDateTime) -> Result<Release, EmbargoError> {
    let Some(text) = embargo_until else {
        return Ok(Release::Now);
    };
    let date = Date::parse(text.trim(), format_description!("[year]-[month]-[day]"))
        .map_err(|_| EmbargoError::NotADate(text.to_string()))?;
    let today = now.to_offset(UtcOffset::UTC).date();
    let latest = months_later(today, 3);
    if date > latest {
        return Err(EmbargoError::TooLong { date, latest });
    }
    Ok(if now < release_time(date) {
        Release::On(date)
    } else {
        Release::Now
    })
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
