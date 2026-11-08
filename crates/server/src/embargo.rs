//! Reading `embargo_until` at submission (design.md, Embargo).

use analyzer::Refusal;
use time::{Date, Month, OffsetDateTime, Time, UtcOffset, macros::format_description};

/// Whether the server takes embargoes (design.md, Embargo: paused, the owner's
/// decision of 2026-11-08). Every real configuration is `Paused`; the tests build
/// `Open`, so the dormant code keeps its tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Embargoes {
    /// `embargo_until` is refused, and new series start public.
    Paused,
    /// Embargoes and hidden series work as design.md describes.
    Open,
}

impl Embargoes {
    /// Whether the server takes embargoes and hidden series.
    pub fn taken(self) -> bool {
        self == Embargoes::Open
    }
}

/// Whether `named`, a version's `embargo_until`, repeats the date its previous
/// version's text names. The text an agent edits still carries that date, so it asks
/// for no new embargo.
pub fn repeats(previous: Option<&str>, named: Option<&str>) -> bool {
    previous.is_some() && parse_date(previous) == parse_date(named)
}

/// The analyzer's refusals that stand for one submission (entry-format.md, Accept,
/// warn, and re-render: refused while embargoes are paused).
pub struct Standing<'r> {
    pub refusals: Vec<&'r Refusal>,
    /// The paused refusal is among them, so the embargo rules' own reasons, which
    /// would contradict it, aren't added.
    pub refused_as_paused: bool,
}

impl<'r> Standing<'r> {
    /// While embargoes are paused, a key asking for one stands unless it is a
    /// `repeated` date, and it replaces an unreadable date's refusal. While they are
    /// taken, it never stands.
    pub fn of(refusals: &'r [Refusal], embargoes: Embargoes, repeated: bool) -> Standing<'r> {
        let asks_for_embargo = |refusal: &Refusal| {
            matches!(refusal, Refusal::EmbargoPaused { field }
                if !(repeated && field == "embargo_until"))
        };
        let refused_as_paused = !embargoes.taken() && refusals.iter().any(asks_for_embargo);
        let refusals = refusals
            .iter()
            .filter(|refusal| match refusal {
                Refusal::EmbargoPaused { .. } => !embargoes.taken() && asks_for_embargo(refusal),
                Refusal::EmbargoUnreadable(_) => !refused_as_paused,
                _ => true,
            })
            .collect();
        Standing {
            refusals,
            refused_as_paused,
        }
    }
}

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
        "`embargo_until: {named}` isn't this chain's date: a new version keeps its chain's embargo, until {date}, so leave `embargo_until` out or set it to {date}"
    )]
    NotTheChainDate { named: String, date: Date },
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

/// A chain's stored release date (`entries::chain_embargo`), read once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainDate {
    /// No embargo.
    Unset,
    On(Date),
    /// Stored, but not a date, which only a hand-edited row can be.
    Unreadable,
}

impl ChainDate {
    pub fn read(stored: Option<&str>) -> ChainDate {
        match stored {
            None => ChainDate::Unset,
            Some(text) => parse_date(Some(text)).map_or(ChainDate::Unreadable, ChainDate::On),
        }
    }

    /// Whether it still holds the chain back at `now`. An unreadable date holds it,
    /// since a release can't be undone.
    pub fn holds(self, now: OffsetDateTime) -> bool {
        match self {
            ChainDate::Unset => false,
            ChainDate::On(date) => now < release_time(date),
            ChainDate::Unreadable => true,
        }
    }
}

/// The date a version bound by the embargo `in_force` keeps, given the
/// `embargo_until` it `named` (design.md, Embargo: "A new version of an embargoed
/// entry keeps its chain's date"): leaving the field out, or repeating the date,
/// keeps it, and any other date is refused. Reason: a token moves an embargo only
/// through its own route, so an agent resending old text can't undo a move.
pub fn kept_date(in_force: Date, named: Option<&str>) -> Result<Date, EmbargoError> {
    match named {
        None => Ok(in_force),
        Some(text) if parse_date(Some(text)) == Some(in_force) => Ok(in_force),
        Some(text) => Err(EmbargoError::NotTheChainDate {
            named: text.trim().to_string(),
            date: in_force,
        }),
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
