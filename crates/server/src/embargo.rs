//! Reading `embargo_until` at submission (design.md, Embargo).

use time::{Date, Month, macros::format_description};

/// What an entry's `embargo_until` means for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// No embargo, or a date that has come: public once it passes screening.
    Now,
    /// Held until this date.
    On(Date),
}

/// Reads `embargo_until` against `today`. Refuses a value that isn't a YYYY-MM-DD
/// date, since publishing an entry whose embargo the server misread can't be undone,
/// and a date more than 3 months ahead.
pub fn release(embargo_until: Option<&str>, today: Date) -> Result<Release, String> {
    let Some(text) = embargo_until else {
        return Ok(Release::Now);
    };
    let date = Date::parse(text.trim(), format_description!("[year]-[month]-[day]"))
        .map_err(|_| format!("`embargo_until: {text}` is not a date written YYYY-MM-DD"))?;
    let latest = months_later(today, 3);
    if date > latest {
        return Err(format!(
            "`embargo_until: {text}` is more than 3 months away; the latest a first embargo may run is {latest}"
        ));
    }
    Ok(if date > today {
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
