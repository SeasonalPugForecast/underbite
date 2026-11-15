//! Solunar periods from the moon's transits, rise and set, and the sky for each local day.
//!
//! Solunar periods are a traditional fishing cue, not a prediction: this module only computes
//! where the bands fall.

use whether_core::sun::{CIVIL, GOLDEN, NAUTICAL, SunDay, sun_crossing, sun_day};

use crate::astro::moon::{MoonEvent, MoonEventKind, moon_day, moon_events};
use crate::model::{
    Polar, SkyConditions, SkyDay, SolunarCause, SolunarKind, SolunarPeriod, SunTimes,
};
use crate::time::{LocalDate, UnixTime};

/// How long the solunar periods are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolunarSettings {
    /// Length of a major period, centred on a transit.
    pub major_minutes: u32,
    /// Length of a minor period, centred on a moonrise or moonset.
    pub minor_minutes: u32,
}

impl Default for SolunarSettings {
    fn default() -> Self {
        Self {
            major_minutes: 120,
            minor_minutes: 60,
        }
    }
}

/// A local calendar date with its UTC bounds (start inclusive, end exclusive), as
/// [`crate::time::local_date_bounds`] returns them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DayBounds {
    pub date: LocalDate,
    pub start: UnixTime,
    pub end: UnixTime,
}

/// Majors centred on upper and lower transits, minors on moonrise and moonset, in start order.
/// Overlapping periods stay separate entries.
pub fn solunar_periods(events: &[MoonEvent], settings: SolunarSettings) -> Vec<SolunarPeriod> {
    let mut periods: Vec<SolunarPeriod> = events
        .iter()
        .map(|e| {
            let (kind, cause, minutes) = match e.kind {
                MoonEventKind::UpperTransit => (
                    SolunarKind::Major,
                    SolunarCause::UpperTransit,
                    settings.major_minutes,
                ),
                MoonEventKind::LowerTransit => (
                    SolunarKind::Major,
                    SolunarCause::LowerTransit,
                    settings.major_minutes,
                ),
                MoonEventKind::Rise => (
                    SolunarKind::Minor,
                    SolunarCause::Rise,
                    settings.minor_minutes,
                ),
                MoonEventKind::Set => (
                    SolunarKind::Minor,
                    SolunarCause::Set,
                    settings.minor_minutes,
                ),
            };
            let half = i64::from(minutes) * 30;
            SolunarPeriod {
                kind,
                start: UnixTime(e.time.0 - half),
                end: UnixTime(e.time.0 + half),
                cause,
            }
        })
        .collect();
    periods.sort_by_key(|p| (p.start, p.end));
    periods
}

fn crossing(
    date: LocalDate,
    lat: f64,
    lon: f64,
    altitude: f64,
) -> (Option<UnixTime>, Option<UnixTime>) {
    match sun_crossing(date, lat, lon, altitude) {
        SunDay::RiseSet { rise, set } => (Some(rise), Some(set)),
        SunDay::AlwaysUp | SunDay::AlwaysDown => (None, None),
    }
}

/// The sun's times on `date`; twilights and golden hours are absent where the sun never crosses
/// those altitudes.
pub fn sun_times(date: LocalDate, lat: f64, lon: f64) -> SunTimes {
    let (polar, sunrise, sunset) = match sun_day(date, lat, lon) {
        SunDay::RiseSet { rise, set } => (None, Some(rise), Some(set)),
        SunDay::AlwaysUp => (Some(Polar::AlwaysUp), None, None),
        SunDay::AlwaysDown => (Some(Polar::AlwaysDown), None, None),
    };
    let (civil_dawn, civil_dusk) = crossing(date, lat, lon, CIVIL);
    let (nautical_dawn, nautical_dusk) = crossing(date, lat, lon, NAUTICAL);
    let (golden_morning_end, golden_evening_start) = crossing(date, lat, lon, GOLDEN);
    SunTimes {
        polar,
        sunrise,
        sunset,
        civil_dawn,
        civil_dusk,
        nautical_dawn,
        nautical_dusk,
        golden_morning_end,
        golden_evening_start,
    }
}

/// Sun, moon and solunar periods for each day, at `lat`, `lon` (degrees, east positive).
///
/// A solunar period belongs to every day it overlaps and is not clipped, so a period across
/// midnight appears whole in both days.
pub fn sky_conditions(
    lat: f64,
    lon: f64,
    days: &[DayBounds],
    settings: SolunarSettings,
) -> SkyConditions {
    // Events up to half a period outside the day can still have a period reaching into it.
    let reach = i64::from(settings.major_minutes.max(settings.minor_minutes)) * 30;
    let days = days
        .iter()
        .map(|d| {
            let events = moon_events(
                lat,
                lon,
                UnixTime(d.start.0 - reach),
                UnixTime(d.end.0 + reach),
            );
            let solunar = solunar_periods(&events, settings)
                .into_iter()
                .filter(|p| p.end > d.start && p.start < d.end)
                .collect();
            SkyDay {
                date: d.date,
                sun: sun_times(d.date, lat, lon),
                moon: moon_day(lat, lon, d.start, d.end),
                solunar,
            }
        })
        .collect();
    SkyConditions { days }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{local_date_bounds, zone};
    use whether_core::time::days_from_civil;

    const BATTERY: (f64, f64) = (40.7033, -74.0170);
    const ANTIPODE: (f64, f64) = (40.7033, 105.9830);
    const DAY_S: i64 = 86_400;

    fn usno(place: (f64, f64), day: u8) -> serde_json::Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/usno/usno-rstt-{:.4}_{:.4}-2026-11-{day:02}.json",
            env!("CARGO_MANIFEST_DIR"),
            place.0,
            place.1
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["properties"]["data"].clone()
    }

    /// Seconds into the day of USNO's moon phenomenon `name`.
    fn phen(data: &serde_json::Value, name: &str) -> Option<i64> {
        data["moondata"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["phen"] == name)
            .map(|p| {
                let (h, m) = p["time"].as_str().unwrap().split_once(':').unwrap();
                h.parse::<i64>().unwrap() * 3_600 + m.parse::<i64>().unwrap() * 60
            })
    }

    fn utc_day(day: u8) -> DayBounds {
        let start = UnixTime(days_from_civil(2026, 11, day) * DAY_S);
        DayBounds {
            date: LocalDate {
                year: 2026,
                month: 11,
                day,
            },
            start,
            end: UnixTime(start.0 + DAY_S),
        }
    }

    fn ny_day(day: u8) -> DayBounds {
        let date = LocalDate {
            year: 2026,
            month: 11,
            day,
        };
        let (start, end) = local_date_bounds(date, &zone("America/New_York").unwrap()).unwrap();
        DayBounds { date, start, end }
    }

    #[test]
    fn periods_are_centred_on_events_with_the_configured_lengths() {
        let events = [
            MoonEvent {
                kind: MoonEventKind::Set,
                time: UnixTime(10_000),
            },
            MoonEvent {
                kind: MoonEventKind::UpperTransit,
                time: UnixTime(10_600),
            },
            MoonEvent {
                kind: MoonEventKind::LowerTransit,
                time: UnixTime(60_000),
            },
            MoonEvent {
                kind: MoonEventKind::Rise,
                time: UnixTime(70_000),
            },
        ];
        let p = solunar_periods(&events, SolunarSettings::default());
        // Overlapping periods stay separate, in start order.
        assert_eq!(
            p.iter().map(|x| x.cause).collect::<Vec<_>>(),
            [
                SolunarCause::UpperTransit,
                SolunarCause::Set,
                SolunarCause::LowerTransit,
                SolunarCause::Rise
            ]
        );
        assert_eq!((p[0].start.0, p[0].end.0), (10_600 - 3_600, 10_600 + 3_600));
        assert_eq!(p[0].kind, SolunarKind::Major);
        assert_eq!((p[1].start.0, p[1].end.0), (10_000 - 1_800, 10_000 + 1_800));
        assert_eq!(p[1].kind, SolunarKind::Minor);
        let custom = solunar_periods(
            &events[..1],
            SolunarSettings {
                major_minutes: 90,
                minor_minutes: 30,
            },
        );
        assert_eq!(custom[0].end.0 - custom[0].start.0, 1_800);
    }

    #[test]
    fn periods_match_usno_events_at_the_battery() {
        let settings = SolunarSettings::default();
        let mut checked = 0;
        for day in 8..=22 {
            let d = utc_day(day);
            let sky = sky_conditions(BATTERY.0, BATTERY.1, &[d], settings);
            let periods = &sky.days[0].solunar;
            let here = usno(BATTERY, day);
            // The antipode's upper transit is the lower transit here.
            let anti = usno(ANTIPODE, day);
            let expected = [
                (SolunarCause::UpperTransit, phen(&here, "Upper Transit")),
                (SolunarCause::LowerTransit, phen(&anti, "Upper Transit")),
                (SolunarCause::Rise, phen(&here, "Rise")),
                (SolunarCause::Set, phen(&here, "Set")),
            ];
            for (cause, at) in expected {
                let Some(at) = at else { continue };
                let found: Vec<_> = periods.iter().filter(|p| p.cause == cause).collect();
                let centre = d.start.0 + at;
                let hit = found
                    .iter()
                    .find(|p| (i64::midpoint(p.start.0, p.end.0) - centre).abs() <= 150)
                    .unwrap_or_else(|| panic!("11-{day} {cause:?} near {at}: {found:?}"));
                let len = hit.end.0 - hit.start.0;
                match hit.kind {
                    SolunarKind::Major => assert_eq!(len, 7_200),
                    SolunarKind::Minor => assert_eq!(len, 3_600),
                }
                checked += 1;
            }
        }
        assert!(checked > 50, "{checked}");
    }

    #[test]
    fn a_period_across_local_midnight_belongs_to_both_days() {
        let settings = SolunarSettings::default();
        let days: Vec<DayBounds> = (8..=22).map(ny_day).collect();
        let sky = sky_conditions(BATTERY.0, BATTERY.1, &days, settings);
        let mut straddling = 0;
        for (skies, bounds) in sky.days.windows(2).zip(days.windows(2)) {
            let midnight = bounds[1].start;
            for p in &skies[0].solunar {
                if p.start < midnight && p.end > midnight {
                    straddling += 1;
                    assert!(skies[1].solunar.contains(p), "{p:?}");
                }
            }
        }
        assert!(straddling > 0, "no period crossed a local midnight");
        // Every listed period overlaps its day, and none that overlaps is missing.
        let all = solunar_periods(
            &moon_events(
                BATTERY.0,
                BATTERY.1,
                UnixTime(days[0].start.0 - DAY_S),
                UnixTime(days[14].end.0 + DAY_S),
            ),
            settings,
        );
        for (day, bounds) in sky.days.iter().zip(&days) {
            let want: Vec<_> = all
                .iter()
                .filter(|p| p.end > bounds.start && p.start < bounds.end)
                .copied()
                .collect();
            assert_eq!(day.solunar, want, "{:?}", day.date);
        }
    }

    #[test]
    fn sky_for_the_battery_has_sun_and_moon() {
        let sky = sky_conditions(
            BATTERY.0,
            BATTERY.1,
            &[ny_day(8)],
            SolunarSettings::default(),
        );
        let d = &sky.days[0];
        let s = d.sun;
        let (rise, set) = (s.sunrise.unwrap(), s.sunset.unwrap());
        // USNO: the sun rises 11:35Z and sets 21:45Z on 2026-11-08.
        let midnight = days_from_civil(2026, 11, 8) * DAY_S;
        assert!((rise.0 - (midnight + 11 * 3_600 + 35 * 60)).abs() <= 120);
        assert!((set.0 - (midnight + 21 * 3_600 + 45 * 60)).abs() <= 120);
        assert!(s.nautical_dawn.unwrap() < s.civil_dawn.unwrap());
        assert!(s.civil_dawn.unwrap() < rise);
        assert!(rise < s.golden_morning_end.unwrap());
        assert!(s.golden_evening_start.unwrap() < set);
        assert!(set < s.civil_dusk.unwrap() && s.civil_dusk.unwrap() < s.nautical_dusk.unwrap());
        assert_eq!(s.polar, None);
        assert!(d.moon.upper_transit.is_some());
    }

    #[test]
    fn polar_days_and_missing_twilight_are_optional() {
        let summer = LocalDate {
            year: 2026,
            month: 6,
            day: 21,
        };
        let t = sun_times(summer, 78.2, 15.6);
        assert_eq!(t.polar, Some(Polar::AlwaysUp));
        assert_eq!((t.sunrise, t.sunset), (None, None));
        let winter = LocalDate {
            year: 2026,
            month: 12,
            day: 21,
        };
        assert_eq!(sun_times(winter, 78.2, 15.6).polar, Some(Polar::AlwaysDown));
        // Oslo at midsummer: the sun sets, civil twilight exists, nautical twilight does not.
        let oslo = sun_times(summer, 59.91, 10.75);
        assert!(oslo.sunrise.is_some() && oslo.polar.is_none());
        assert_eq!(oslo.nautical_dawn, None);
        assert!(oslo.civil_dawn.is_some());
    }
}
