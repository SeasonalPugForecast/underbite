//! Sun and moon times and the moon's phase.

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::time::Weekday;

use crate::model::{MoonDay, MoonPhase, Polar, SkyDay, SunTimes};
use crate::time::{LocalDate, UnixTime, day_number, local_date, zone};
use crate::view::format::Formatter;
use crate::view::strings::{Key, Strings};
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, Quantity, Row, Section, TimedLabel, ViewInput,
};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SunMoonCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: SunMoonFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MoonPhaseVm {
    pub phase: MoonPhase,
    /// "Waxing gibbous"
    pub label: String,
    /// The lit fraction as a percentage.
    pub illumination: Quantity,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MoonVm {
    pub phase: MoonPhaseVm,
    pub rise: Option<TimedLabel>,
    pub set: Option<TimedLabel>,
    pub upper_transit: Option<TimedLabel>,
    pub lower_transit: Option<TimedLabel>,
    /// The nearest new, quarter or full moon.
    pub nearest_principal: Option<TimedLabel>,
}

/// One local day.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SunMoonDayVm {
    /// "Sat Nov 14"
    pub date: String,
    /// Dawn, sunrise, golden hour, sunset, dusk in time order, only those that occur.
    pub sun: Vec<TimedLabel>,
    /// "10 h 12 min"
    pub day_length: Option<String>,
    /// "Sun stays up all day" or "Sun stays down all day".
    pub polar: Option<String>,
    pub moon: MoonVm,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SunMoonFull {
    /// Today first, then the days the snapshot has.
    pub days: Vec<SunMoonDayVm>,
    pub sections: Vec<Section>,
}

/// The credit for values computed on the device.
pub(crate) fn computed_credit(strings: Strings) -> CreditVm {
    CreditVm {
        source: strings.get(Key::ComputedOnDevice).to_owned(),
        text: strings.get(Key::SunMoonCredit).to_owned(),
        link: None,
        licence: None,
    }
}

/// "Sun Nov 8". The weekday comes from the day count, so it needs no zone.
pub(crate) fn day_label(f: &Formatter, date: LocalDate) -> String {
    // 1970-01-01 was a Thursday.
    let weekday = match (day_number(date) + 4).rem_euclid(7) {
        0 => Weekday::Sun,
        1 => Weekday::Mon,
        2 => Weekday::Tue,
        3 => Weekday::Wed,
        4 => Weekday::Thu,
        5 => Weekday::Fri,
        _ => Weekday::Sat,
    };
    format!("{} {}", f.strings.weekday(weekday), f.date(date))
}

/// The snapshot's sky days from the spot's local today on, in date order.
pub(crate) fn today_and_after<'a>(input: &ViewInput, days: &'a [SkyDay]) -> Vec<&'a SkyDay> {
    let today = zone(&input.snapshot.spot.time_zone)
        .ok()
        .and_then(|tz| local_date(input.now, &tz).ok())
        .map_or(i64::MIN, day_number);
    let mut out: Vec<&SkyDay> = days
        .iter()
        .filter(|d| day_number(d.date) >= today)
        .collect();
    out.sort_by_key(|d| day_number(d.date));
    out
}

fn timed(f: &Formatter, label: Key, t: UnixTime) -> TimedLabel {
    TimedLabel {
        label: f.strings.get(label).to_owned(),
        time: f.clock(t),
        t: t.0,
    }
}

/// The sun's events that occur, in time order.
fn sun_events(f: &Formatter, sun: &SunTimes) -> Vec<TimedLabel> {
    let mut out: Vec<TimedLabel> = [
        (Key::NauticalDawn, sun.nautical_dawn),
        (Key::CivilDawn, sun.civil_dawn),
        (Key::Sunrise, sun.sunrise),
        (Key::GoldenHourEnds, sun.golden_morning_end),
        (Key::GoldenHourStarts, sun.golden_evening_start),
        (Key::Sunset, sun.sunset),
        (Key::CivilDusk, sun.civil_dusk),
        (Key::NauticalDusk, sun.nautical_dusk),
    ]
    .into_iter()
    .filter_map(|(label, t)| t.map(|t| timed(f, label, t)))
    .collect();
    out.sort_by_key(|e| e.t);
    out
}

fn polar_text(f: &Formatter, polar: Polar) -> &'static str {
    f.strings.get(match polar {
        Polar::AlwaysUp => Key::SunStaysUp,
        Polar::AlwaysDown => Key::SunStaysDown,
    })
}

/// "10 h 12 min", rounded to the minute.
fn day_length(f: &Formatter, sun: &SunTimes) -> Option<String> {
    let (rise, set) = (sun.sunrise?, sun.sunset?);
    let minutes = (set.0 - rise.0 + 30).div_euclid(60);
    (minutes > 0).then(|| {
        f.strings.fmt(
            Key::DayLengthValue,
            &[
                ("hours", &(minutes / 60).to_string()),
                ("minutes", &format!("{:02}", minutes % 60)),
            ],
        )
    })
}

fn moon_vm(f: &Formatter, moon: &MoonDay) -> MoonVm {
    MoonVm {
        phase: MoonPhaseVm {
            phase: moon.phase,
            label: f.strings.moon_phase(moon.phase).to_owned(),
            illumination: f.percent(Some(moon.illuminated_fraction * 100.0)),
        },
        rise: moon.rise.map(|t| timed(f, Key::Moonrise, t)),
        set: moon.set.map(|t| timed(f, Key::Moonset, t)),
        upper_transit: moon.upper_transit.map(|t| timed(f, Key::MoonOverhead, t)),
        lower_transit: moon.lower_transit.map(|t| timed(f, Key::MoonUnderfoot, t)),
        nearest_principal: moon.nearest_principal.map(|p| TimedLabel {
            label: f.strings.principal_phase(p.phase).to_owned(),
            time: format!("{} {}", f.date_of(p.time), f.clock(p.time)),
            t: p.time.0,
        }),
    }
}

fn day_vm(f: &Formatter, d: &SkyDay) -> SunMoonDayVm {
    SunMoonDayVm {
        date: day_label(f, d.date),
        sun: sun_events(f, &d.sun),
        day_length: day_length(f, &d.sun),
        polar: d.sun.polar.map(|p| polar_text(f, p).to_owned()),
        moon: moon_vm(f, &d.moon),
    }
}

fn row(label: &str, value: String) -> Row {
    Row {
        label: label.to_owned(),
        value,
        detail: None,
    }
}

/// Today's sun and moon as label and value rows; events that do not happen say so.
fn sections_for(f: &Formatter, day: &SunMoonDayVm) -> Vec<Section> {
    let s = f.strings;
    let mut sun: Vec<Row> = day
        .sun
        .iter()
        .map(|e| row(&e.label, e.time.clone()))
        .collect();
    if let Some(p) = &day.polar {
        sun.push(row(s.get(Key::Sun), p.clone()));
    }
    if let Some(l) = &day.day_length {
        sun.push(row(s.get(Key::DayLength), l.clone()));
    }
    let m = &day.moon;
    let event = |label: &str, missing: &str, e: &Option<TimedLabel>| {
        row(
            label,
            e.as_ref()
                .map_or_else(|| missing.to_owned(), |e| e.time.clone()),
        )
    };
    let mut moon = vec![
        row(
            &m.phase.label,
            s.fmt(Key::Lit, &[("value", &m.phase.illumination.full())]),
        ),
        event(s.get(Key::Moonrise), s.get(Key::NoMoonrise), &m.rise),
        event(s.get(Key::Moonset), s.get(Key::NoMoonset), &m.set),
        event(
            s.get(Key::MoonOverhead),
            s.get(Key::NotToday),
            &m.upper_transit,
        ),
        event(
            s.get(Key::MoonUnderfoot),
            s.get(Key::NotToday),
            &m.lower_transit,
        ),
    ];
    if let Some(p) = &m.nearest_principal {
        moon.push(row(&p.label, p.time.clone()));
    }
    vec![
        Section {
            title: s.get(Key::Sun).to_owned(),
            rows: sun,
        },
        Section {
            title: s.get(Key::Moon).to_owned(),
            rows: moon,
        },
    ]
}

pub fn build(input: &ViewInput) -> Option<SunMoonCard> {
    let days = today_and_after(input, &input.snapshot.sky.days);
    if days.is_empty() {
        return None;
    }
    let f = Formatter::for_input(input);
    let vms: Vec<SunMoonDayVm> = days.iter().map(|d| day_vm(&f, d)).collect();
    let first = &vms[0];

    let sun_line = if let Some(p) = &first.polar {
        p.clone()
    } else {
        let find = |key: Key| {
            let label = f.strings.get(key);
            first.sun.iter().find(|e| e.label == label)
        };
        let (rise, set) = (f.strings.get(Key::Sunrise), f.strings.get(Key::Sunset));
        match (find(Key::Sunrise), find(Key::Sunset)) {
            (Some(r), Some(s)) => format!("{rise} {} · {set} {}", r.time, s.time),
            (Some(r), None) => format!("{rise} {}", r.time),
            (None, Some(s)) => format!("{set} {}", s.time),
            (None, None) => f.strings.get(Key::NoSunriseOrSunset).to_owned(),
        }
    };
    let moon = &first.moon;
    let moon_line = match (&moon.rise, &moon.set) {
        (Some(r), _) => format!(
            "{} · {} {}",
            moon.phase.label,
            f.strings.get(Key::Moonrise),
            r.time
        ),
        (None, Some(s)) => format!(
            "{} · {} {}",
            moon.phase.label,
            f.strings.get(Key::Moonset),
            s.time
        ),
        (None, None) => format!("{} · {}", moon.phase.label, f.strings.get(Key::NoMoonrise)),
    };
    let accessibility = format!(
        "{}. {}, {}.",
        sun_line.replace(" · ", ", "),
        moon.phase.label,
        f.strings
            .fmt(Key::Lit, &[("value", &moon.phase.illumination.full())])
    );
    let headline = moon.phase.illumination.clone();
    let sections = sections_for(&f, first);
    Some(SunMoonCard {
        chrome: CardChrome {
            id: CardId::SunMoon,
            title: f.strings.card_title(CardId::SunMoon).to_owned(),
            footnote: None,
            credits: vec![computed_credit(f.strings)],
            state: CardState::Fresh,
            notices: Vec::new(),
            accessibility,
        },
        compact: Compact {
            headline,
            lines: vec![sun_line, moon_line],
            spark: Vec::new(),
        },
        full: SunMoonFull {
            days: vms,
            sections,
        },
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::astro::solunar::{DayBounds, SolunarSettings, sky_conditions};
    use crate::time::{LocalDate, UnixTime, local_date_bounds, zone};
    use crate::view::ViewInput;
    use crate::view::tests::sample_input;

    /// The Battery's sky for 2026-11-08 and the two days after, from the astronomy module, at
    /// 8:00 AM EST on Nov 8.
    pub(crate) fn battery_input() -> ViewInput {
        let mut input = sample_input();
        let tz = zone("America/New_York").unwrap();
        let days: Vec<DayBounds> = (8..=10)
            .map(|day| {
                let date = LocalDate {
                    year: 2026,
                    month: 11,
                    day,
                };
                let (start, end) = local_date_bounds(date, &tz).unwrap();
                DayBounds { date, start, end }
            })
            .collect();
        input.snapshot.sky = sky_conditions(40.7033, -74.0170, &days, SolunarSettings::default());
        input.now = UnixTime(days[0].start.0 + 8 * 3_600);
        input
    }
}

#[cfg(test)]
mod tests {
    use super::testing::battery_input;
    use super::*;
    use crate::view::Voice;
    use whether_core::presentation::TimeFormat;

    #[test]
    fn today_first_with_sun_and_moon_events_in_time_order() {
        let card = build(&battery_input()).unwrap();
        assert_eq!(card.full.days.len(), 3);
        let day = &card.full.days[0];
        assert_eq!(day.date, "Sun Nov 8");
        let labels: Vec<_> = day.sun.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Nautical dawn",
                "Civil dawn",
                "Sunrise",
                "Golden hour ends",
                "Golden hour starts",
                "Sunset",
                "Civil dusk",
                "Nautical dusk"
            ]
        );
        assert!(day.sun.windows(2).all(|w| w[0].t < w[1].t));
        // Sunrise at the Battery on Nov 8 is about 6:25 EST.
        let rise = day.sun.iter().find(|e| e.label == "Sunrise").unwrap();
        assert!(
            rise.time.starts_with("6:") && rise.time.ends_with("AM"),
            "{}",
            rise.time
        );
        assert!(day.day_length.as_deref().unwrap().starts_with("10 h"));
        assert!(day.polar.is_none());
        assert_eq!(card.chrome.notices.len(), 0);
        assert_eq!(card.compact.lines.len(), 2);
    }

    #[test]
    fn a_day_without_moonrise_says_so() {
        let mut input = battery_input();
        input.snapshot.sky.days[0].moon.rise = None;
        input.snapshot.sky.days[0].moon.set = None;
        let card = build(&input).unwrap();
        assert!(card.compact.lines[1].contains("No moonrise today"));
        let moon = &card.full.sections[1];
        let rise = moon.rows.iter().find(|r| r.label == "Moonrise").unwrap();
        assert_eq!(rise.value, "No moonrise today");
        assert!(card.full.days[0].moon.rise.is_none());
    }

    #[test]
    fn polar_days_say_so_and_have_no_sunrise() {
        let mut input = battery_input();
        input.snapshot.sky.days[0].sun = SunTimes {
            polar: Some(Polar::AlwaysUp),
            ..SunTimes::default()
        };
        let card = build(&input).unwrap();
        let day = &card.full.days[0];
        assert_eq!(day.polar.as_deref(), Some("Sun stays up all day"));
        assert!(day.sun.is_empty() && day.day_length.is_none());
        assert_eq!(card.compact.lines[0], "Sun stays up all day");
    }

    #[test]
    fn no_sky_is_no_card_and_past_days_are_dropped() {
        let mut input = battery_input();
        input.now = UnixTime(input.now.0 + 2 * 86_400);
        assert_eq!(build(&input).unwrap().full.days.len(), 1);
        input.snapshot.sky.days.clear();
        assert!(build(&input).is_none());
    }

    #[test]
    fn snapshots_in_both_voices() {
        let mut input = battery_input();
        insta::assert_json_snapshot!("sun_moon_luna", build(&input).unwrap());
        input.voice = Voice::Plain;
        input.clock = TimeFormat::H24;
        insta::assert_json_snapshot!("sun_moon_plain_24h", build(&input).unwrap());
    }
}
