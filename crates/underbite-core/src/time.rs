//! Instants, dates and offsets; local-time helpers over IANA zones and the time formats sources use.
//!
//! The time-zone database is bundled into the binary (jiff's `tzdb-bundle-always`) so lookups work
//! on iOS, where an app cannot rely on reading the system zoneinfo, as well as on Linux. jiff still
//! prefers the system database where it exists and falls back to the bundle.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use whether_core::time::{days_from_civil, days_in_month};

pub use whether_core::time::{LocalDate, UnixTime, UtcOffset, parse_rfc3339};

/// Seconds in a UTC day.
const DAY: i64 = 86_400;

/// A time or zone that could not be resolved or parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    /// The IANA zone name is unknown.
    #[error("unknown time zone {0:?}")]
    UnknownZone(String),
    /// A date or instant is outside the supported range.
    #[error("date out of range")]
    OutOfRange,
    /// A timestamp or duration string is malformed.
    #[error("unrecognised time {0:?}")]
    Parse(String),
}

fn parse_err(s: &str) -> TimeError {
    TimeError::Parse(s.to_owned())
}

/// Looks up an IANA zone by name, such as `America/New_York`.
///
/// # Errors
/// [`TimeError::UnknownZone`] when the name is not in the database.
pub fn zone(name: &str) -> Result<TimeZone, TimeError> {
    TimeZone::get(name).map_err(|_| TimeError::UnknownZone(name.to_owned()))
}

fn timestamp(t: UnixTime) -> Result<Timestamp, TimeError> {
    Timestamp::from_second(t.0).map_err(|_| TimeError::OutOfRange)
}

fn civil(d: LocalDate) -> Result<Date, TimeError> {
    let year = i16::try_from(d.year).map_err(|_| TimeError::OutOfRange)?;
    let month = i8::try_from(d.month).map_err(|_| TimeError::OutOfRange)?;
    let day = i8::try_from(d.day).map_err(|_| TimeError::OutOfRange)?;
    Date::new(year, month, day).map_err(|_| TimeError::OutOfRange)
}

fn local(d: Date) -> LocalDate {
    // INVARIANT: jiff dates have month 1..=12 and day 1..=31, which are positive.
    LocalDate {
        year: i32::from(d.year()),
        month: u8::try_from(d.month()).unwrap_or(1),
        day: u8::try_from(d.day()).unwrap_or(1),
    }
}

/// The calendar date of an instant in a zone.
///
/// # Errors
/// [`TimeError::OutOfRange`] for instants jiff cannot represent.
pub fn local_date(t: UnixTime, tz: &TimeZone) -> Result<LocalDate, TimeError> {
    Ok(local(timestamp(t)?.to_zoned(tz.clone()).date()))
}

/// The UTC bounds of a local date in a zone: midnight to the next midnight (start inclusive, end
/// exclusive). The span is 23 or 25 hours on a daylight-saving change.
///
/// # Errors
/// [`TimeError::OutOfRange`] for dates jiff cannot represent.
pub fn local_date_bounds(
    date: LocalDate,
    tz: &TimeZone,
) -> Result<(UnixTime, UnixTime), TimeError> {
    let d = civil(date)?;
    let next = d.tomorrow().map_err(|_| TimeError::OutOfRange)?;
    let start = d.to_zoned(tz.clone()).map_err(|_| TimeError::OutOfRange)?;
    let end = next
        .to_zoned(tz.clone())
        .map_err(|_| TimeError::OutOfRange)?;
    Ok((
        UnixTime(start.timestamp().as_second()),
        UnixTime(end.timestamp().as_second()),
    ))
}

/// The zone's standard-time offset on a date, ignoring daylight saving. USGS daily values are
/// means over the site's standard-time day, so a summer date still gives the winter offset.
///
/// When daylight time is in effect the nearest standard-time transition (before or after) gives the
/// offset, so southern-hemisphere zones work too. A zone with no standard-time period nearby falls
/// back to its offset on that date.
///
/// # Errors
/// [`TimeError::OutOfRange`] for dates jiff cannot represent.
pub fn standard_offset(tz: &TimeZone, date: LocalDate) -> Result<UtcOffset, TimeError> {
    let noon = civil(date)?
        .at(12, 0, 0, 0)
        .to_zoned(tz.clone())
        .map_err(|_| TimeError::OutOfRange)?
        .timestamp();
    let info = tz.to_offset_info(noon);
    let seconds = if info.dst().is_dst() {
        let before = tz.preceding(noon).take(4);
        let after = tz.following(noon).take(4);
        before
            .chain(after)
            .find(|t| !t.dst().is_dst())
            .map_or(info.offset().seconds(), |t| t.offset().seconds())
    } else {
        info.offset().seconds()
    };
    Ok(UtcOffset(seconds))
}

/// The `MM-DD` day-of-year key of a date, including `02-29`.
pub fn day_key(date: LocalDate) -> String {
    format!("{:02}-{:02}", date.month, date.day)
}

/// Parses `YYYY-MM-DD`.
///
/// # Errors
/// [`TimeError::Parse`] when the string is not a valid calendar date.
pub fn parse_date(s: &str) -> Result<LocalDate, TimeError> {
    let err = || parse_err(s);
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(err());
    }
    let year: i32 = s[0..4].parse().map_err(|_| err())?;
    let month: u8 = s[5..7].parse().map_err(|_| err())?;
    let day: u8 = s[8..10].parse().map_err(|_| err())?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return Err(err());
    }
    Ok(LocalDate { year, month, day })
}

/// Parses `YYYY-MM-DDTHH:MM` or `YYYY-MM-DDTHH:MM:SS` without an offset (Open-Meteo), read at the
/// given offset; pass `UtcOffset(0)` when the request asked for UTC.
///
/// # Errors
/// [`TimeError::Parse`] when the string is malformed.
pub fn parse_local_iso(s: &str, offset: UtcOffset) -> Result<UnixTime, TimeError> {
    let full = match s.len() {
        16 => format!("{s}:00Z"),
        19 => format!("{s}Z"),
        _ => return Err(parse_err(s)),
    };
    let t = parse_rfc3339(&full).ok_or_else(|| parse_err(s))?;
    Ok(UnixTime(t.0 - i64::from(offset.0)))
}

/// Parses a CO-OPS time, `YYYY-MM-DD HH:MM` in UTC.
///
/// # Errors
/// [`TimeError::Parse`] when the string is malformed.
pub fn parse_coops(s: &str) -> Result<UnixTime, TimeError> {
    if s.len() != 16 || s.as_bytes()[10] != b' ' {
        return Err(parse_err(s));
    }
    let iso = format!("{}T{}", &s[..10], &s[11..]);
    parse_local_iso(&iso, UtcOffset(0)).map_err(|_| parse_err(s))
}

/// Parses NDBC's five whitespace-separated UTC columns, `YYYY MM DD hh mm`.
///
/// # Errors
/// [`TimeError::Parse`] when the columns are missing or invalid.
pub fn parse_ndbc(s: &str) -> Result<UnixTime, TimeError> {
    let err = || parse_err(s);
    let mut cols = s.split_whitespace();
    let year = cols.next().filter(|c| c.len() == 4).ok_or_else(err)?;
    let mut two = || cols.next().filter(|c| c.len() == 2).ok_or_else(err);
    let (month, day, hour, minute) = (two()?, two()?, two()?, two()?);
    if cols.next().is_some() {
        return Err(err());
    }
    parse_local_iso(
        &format!("{year}-{month}-{day}T{hour}:{minute}"),
        UtcOffset(0),
    )
    .map_err(|_| err())
}

/// Parses an NWS interval, `start/duration` such as `2026-11-08T14:00:00+00:00/PT1H`, into the
/// start instant and the length in seconds.
///
/// # Errors
/// [`TimeError::Parse`] when either half is malformed.
pub fn parse_interval(s: &str) -> Result<(UnixTime, i64), TimeError> {
    let (start, duration) = s.split_once('/').ok_or_else(|| parse_err(s))?;
    let start = parse_rfc3339(start).ok_or_else(|| parse_err(s))?;
    Ok((start, parse_duration(duration).ok_or_else(|| parse_err(s))?))
}

/// Parses an ISO 8601 duration of days, hours, minutes and seconds (`P1DT6H`, `PT30M`), in
/// seconds with a day counted as 24 hours.
fn parse_duration(s: &str) -> Option<i64> {
    let rest = s.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };
    if date_part.is_empty() && time_part.is_none_or(str::is_empty) {
        return None;
    }
    let mut total = 0i64;
    let mut take = |part: &str, units: &[(char, i64)]| -> Option<()> {
        let mut num = String::new();
        let mut last = i64::MAX;
        for c in part.chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else {
                let (_, mult) = units.iter().find(|(u, _)| *u == c)?;
                // Units must be in descending order and each carry a number.
                if *mult >= last || num.is_empty() {
                    return None;
                }
                last = *mult;
                total += num.parse::<i64>().ok()?.checked_mul(*mult)?;
                num.clear();
            }
        }
        num.is_empty().then_some(())
    };
    take(date_part, &[('D', DAY)])?;
    if let Some(t) = time_part {
        take(t, &[('H', 3_600), ('M', 60), ('S', 1)])?;
    }
    Some(total)
}

/// Days from 1970-01-01 to a local date; handy for sorting and day arithmetic.
pub fn day_number(date: LocalDate) -> i64 {
    days_from_civil(date.year, date.month, date.day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ny() -> TimeZone {
        zone("America/New_York").unwrap()
    }

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    #[test]
    fn new_york_standard_offset_is_minus_five_all_year() {
        for d in [date(2026, 7, 7), date(2026, 11, 8), date(2026, 1, 15)] {
            assert_eq!(standard_offset(&ny(), d), Ok(UtcOffset(-5 * 3600)), "{d:?}");
        }
    }

    #[test]
    fn southern_and_fixed_zones() {
        // Sydney observes daylight time in January; its standard offset is +10.
        let sydney = zone("Australia/Sydney").unwrap();
        assert_eq!(
            standard_offset(&sydney, date(2026, 1, 15)),
            Ok(UtcOffset(10 * 3600))
        );
        assert_eq!(
            standard_offset(&sydney, date(2026, 7, 15)),
            Ok(UtcOffset(10 * 3600))
        );
        let phoenix = zone("America/Phoenix").unwrap();
        assert_eq!(
            standard_offset(&phoenix, date(2026, 7, 7)),
            Ok(UtcOffset(-7 * 3600))
        );
    }

    #[test]
    fn local_date_across_midnight() {
        let t = parse_rfc3339("2026-11-09T03:00:00Z").unwrap();
        assert_eq!(local_date(t, &ny()), Ok(date(2026, 11, 8)));
    }

    #[test]
    fn date_bounds_follow_the_zone() {
        let (s, e) = local_date_bounds(date(2026, 7, 7), &ny()).unwrap();
        assert_eq!(s, parse_rfc3339("2026-07-07T04:00:00Z").unwrap());
        assert_eq!(e.0 - s.0, DAY);
        // The day daylight time ends is 25 hours long; the day it starts is 23.
        let (s, e) = local_date_bounds(date(2026, 11, 1), &ny()).unwrap();
        assert_eq!(e.0 - s.0, 25 * 3600);
        let (s, e) = local_date_bounds(date(2026, 3, 8), &ny()).unwrap();
        assert_eq!(e.0 - s.0, 23 * 3600);
    }

    #[test]
    fn unknown_zone_is_an_error() {
        assert_eq!(
            zone("Nowhere/Land").unwrap_err(),
            TimeError::UnknownZone("Nowhere/Land".into())
        );
    }

    #[test]
    fn day_keys() {
        assert_eq!(day_key(date(2028, 2, 29)), "02-29");
        assert_eq!(day_key(date(2026, 11, 8)), "11-08");
    }

    #[test]
    fn source_formats() {
        let t = parse_rfc3339("2026-11-08T14:30:00Z").unwrap();
        assert_eq!(parse_coops("2026-11-08 14:30"), Ok(t));
        assert_eq!(parse_ndbc("2026 11 08 14 30"), Ok(t));
        assert_eq!(parse_local_iso("2026-11-08T14:30", UtcOffset(0)), Ok(t));
        assert_eq!(
            parse_local_iso("2026-11-08T09:30", UtcOffset(-5 * 3600)),
            Ok(t)
        );
        assert_eq!(
            parse_rfc3339("2026-11-08T05:45:00+00:00").map(|t| t.0 % 3600),
            Some(2700)
        );
        assert_eq!(parse_date("2026-11-08"), Ok(date(2026, 11, 8)));
        assert!(parse_date("2026-02-30").is_err());
        assert!(parse_coops("2026-11-08T14:30").is_err());
        assert!(parse_ndbc("2026 11 08 14").is_err());
    }

    #[test]
    fn nws_intervals() {
        let start = parse_rfc3339("2026-11-08T14:00:00-05:00").unwrap();
        assert_eq!(
            parse_interval("2026-11-08T14:00:00-05:00/PT1H"),
            Ok((start, 3600))
        );
        assert_eq!(
            parse_interval("2026-11-08T14:00:00-05:00/P1DT6H"),
            Ok((start, 30 * 3600))
        );
        assert_eq!(
            parse_interval("2026-11-08T14:00:00-05:00/PT30M").map(|r| r.1),
            Ok(1800)
        );
        for bad in [
            "2026-11-08T14:00:00-05:00",
            "x/PT1H",
            "2026-11-08T14:00:00Z/P",
            "2026-11-08T14:00:00Z/PT1X",
        ] {
            assert!(parse_interval(bad).is_err(), "{bad}");
        }
    }
}
