//! Seam: `embargo::release`, which reads `embargo_until` at a given time for an
//! entry first submitted at another (design.md, Embargo): the latest date is three
//! months after the first submission, and a date whose release time has come is
//! refused. The router calls it with the clock's time, so only here can a test pick
//! the days.

use filedrawer_server::embargo::{EmbargoError, Release, release};
use time::macros::{date, datetime};
use time::{Date, Duration, OffsetDateTime};

/// Asserts that an entry first submitted at noon UTC on `first` may, the day
/// after, be embargoed until `latest` and not until the day after it.
fn assert_latest(first: Date, latest: Date) {
    let first_submitted = first.with_hms(12, 0, 0).unwrap().assume_utc();
    let named_the_day_after = |date: Date| {
        release(
            Some(&date.to_string()),
            first_submitted + Duration::days(1),
            first_submitted,
        )
    };
    let too_late = latest.next_day().unwrap();

    assert_eq!(named_the_day_after(latest), Ok(Release::On(latest)));
    assert_eq!(
        named_the_day_after(too_late),
        Err(EmbargoError::TooLong {
            date: too_late,
            latest
        })
    );
}

#[test]
fn the_latest_date_is_the_same_day_three_months_on() {
    assert_latest(date!(2026 - 08 - 15), date!(2026 - 11 - 15));
}

#[test]
fn three_months_from_november_cross_the_year_and_stop_at_februarys_last_day() {
    assert_latest(date!(2026 - 11 - 30), date!(2027 - 02 - 28));
}

#[test]
fn three_months_from_december_end_in_march_of_the_next_year() {
    assert_latest(date!(2026 - 12 - 31), date!(2027 - 03 - 31));
}

#[test]
fn a_leap_years_february_gives_the_29th() {
    assert_latest(date!(2027 - 11 - 30), date!(2028 - 02 - 29));
}

#[test]
fn a_date_is_refused_from_its_release_time_2359_at_utc_minus_12() {
    let first_submitted: OffsetDateTime = datetime!(2026-11-30 12:00 UTC);
    let named = Some("2026-12-10");
    // 23:59 on the 10th at UTC−12 is 11:59 UTC on the 11th.
    let release_time = datetime!(2026-12-11 11:59 UTC);

    assert_eq!(
        release(named, release_time - Duration::seconds(1), first_submitted),
        Ok(Release::On(date!(2026 - 12 - 10)))
    );
    assert_eq!(
        release(named, release_time, first_submitted),
        Err(EmbargoError::Passed(date!(2026 - 12 - 10)))
    );
}
