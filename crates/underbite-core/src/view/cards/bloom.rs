//! The bloom card: satellite cyanobacteria counts and harmful algal bloom forecasts near the spot.
//!
//! Its words are safety text: plain in every voice. A stale or missing satellite composite says
//! "no recent satellite data", never "no bloom".

use schemars::JsonSchema;
use serde::Serialize;
use whether_core::time::days_from_civil;

use crate::model::{BloomInfo, HabForecast, HabLevel, Measurement};
use crate::provenance::Provenance;
use crate::series::Series;
use crate::time::{LocalDate, UnixTime};
use crate::view::cards::flow::{LinkVm, capitalize, card_state, credits, row};
use crate::view::cards::wind::latest_at;
use crate::view::format::Formatter;
use crate::view::strings::{Key, SafetyKey};
use crate::view::{
    CardChrome, CardId, CardState, Compact, NoticeKind, Quantity, Row, Section, ViewInput,
};

const DAY_S: i64 = 86_400;
/// A satellite composite older than this is not a current picture of the lake.
const CYAN_RECENT_S: i64 = 14 * DAY_S;
/// A forecast valid longer ago than this is not a current forecast.
const HAB_RECENT_S: i64 = 36 * 3_600;

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct BloomCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: BloomFull,
}

/// The satellite cyanobacteria estimate of the spot's lake.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CyanVm {
    /// False when the composite is too old or has no value: the display says so and shows no
    /// count.
    pub recent: bool,
    /// The highest cell concentration, only while `recent`.
    pub max: Option<Quantity>,
    /// The composite's date, when known.
    pub image_date: Option<String>,
    /// The line to show: the estimate, or "No recent satellite data".
    pub text: String,
    /// EPA's description of the product as experimental.
    pub experimental: String,
}

/// The NOAA respiratory-irritation forecast of the nearest beach.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct HabVm {
    pub recent: bool,
    /// Only while `recent`.
    pub level: Option<HabLevel>,
    /// The line to show: the forecast, or "No recent bloom forecast".
    pub text: String,
    /// The source's weighting, "0.9 of 1".
    pub confidence: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct BloomFull {
    pub cyan: Option<CyanVm>,
    pub hab: Option<HabVm>,
    /// The health advice about people and dogs: shown with every bloom card.
    pub advisory: String,
    pub sections: Vec<Section>,
    /// The regional pages (bulletins, viewers).
    pub links: Vec<LinkVm>,
}

fn level_key(level: HabLevel) -> SafetyKey {
    match level {
        HabLevel::VeryLow => SafetyKey::BloomLevelVeryLow,
        HabLevel::Low => SafetyKey::BloomLevelLow,
        HabLevel::Moderate => SafetyKey::BloomLevelModerate,
        HabLevel::High => SafetyKey::BloomLevelHigh,
        HabLevel::Unknown => SafetyKey::BloomLevelUnknown,
    }
}

fn date_start(d: LocalDate) -> UnixTime {
    UnixTime(days_from_civil(d.year, d.month, d.day) * DAY_S)
}

/// The UTC calendar date of an instant.
fn day_of(t: UnixTime) -> LocalDate {
    let z = t.0.div_euclid(DAY_S) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // INVARIANT: month is 1..=12 and day 1..=31 by construction of the algorithm.
    LocalDate {
        year: i32::try_from(year).unwrap_or(1970),
        month: u8::try_from(month).unwrap_or(1),
        day: u8::try_from(day).unwrap_or(1),
    }
}

/// The satellite part: the highest count when the composite is recent.
fn cyan_vm(f: &Formatter, bloom: &BloomInfo, max: Option<&Series<f32>>, now: UnixTime) -> CyanVm {
    let latest = max.and_then(|s| latest_at(s, now));
    let date = bloom
        .cyan_image_date
        .or_else(|| latest.map(|l| day_of(l.time)));
    let recent = latest.is_some()
        && date.is_some_and(|d| (0..=CYAN_RECENT_S).contains(&(now.0 - date_start(d).0)));
    let date_text = date.map(|d| f.date(d));
    let max_q = latest.filter(|_| recent).map(|l| Quantity {
        value: Some(l.value),
        text: f.strings.grouped(l.value, 0),
        unit: f.strings.get(Key::UnitCellsPerMl).to_owned(),
    });
    let text = match (&max_q, &date_text) {
        (Some(q), Some(d)) => f.strings.safety_fmt(
            SafetyKey::BloomCyanMax,
            &[("value", &q.full()), ("date", d)],
        ),
        (_, Some(d)) => f
            .strings
            .safety_fmt(SafetyKey::BloomNoRecentSatelliteLast, &[("date", d)]),
        _ => f
            .strings
            .safety(SafetyKey::BloomNoRecentSatellite)
            .to_owned(),
    };
    CyanVm {
        recent,
        max: max_q,
        image_date: date_text,
        text,
        experimental: f
            .strings
            .safety(SafetyKey::BloomCyanExperimental)
            .to_owned(),
    }
}

fn hab_vm(f: &Formatter, h: &HabForecast, now: UnixTime) -> HabVm {
    let recent = now.0 - h.valid_at.0 <= HAB_RECENT_S;
    if !recent {
        return HabVm {
            recent,
            level: None,
            text: f
                .strings
                .safety(SafetyKey::BloomNoRecentForecast)
                .to_owned(),
            confidence: None,
        };
    }
    HabVm {
        recent,
        level: Some(h.level),
        text: f.strings.safety_fmt(
            SafetyKey::BloomForecastLine,
            &[
                ("level", f.strings.safety(level_key(h.level))),
                ("beach", &h.location),
                ("distance", &f.distance(Some(h.distance_m)).full()),
            ],
        ),
        confidence: h.confidence.map(|c| {
            f.strings.safety_fmt(
                SafetyKey::BloomConfidence,
                &[("value", &f.strings.number(c, 1))],
            )
        }),
    }
}

#[allow(clippy::too_many_lines)]
pub fn build(input: &ViewInput) -> Option<BloomCard> {
    let bloom = input.snapshot.water.bloom.as_ref()?;
    let f = Formatter::for_input(input);
    let now = input.now;

    let max = bloom.cyan_max_cells_ml.as_ref().filter(|s| !s.is_empty());
    let has_cyan =
        max.is_some() || bloom.cyan_image_date.is_some() || bloom.cyan_valid_cells.is_some();
    // A bloom record with nothing in it still says the satellite has nothing recent.
    let cyan = (has_cyan || bloom.hab.is_none()).then(|| cyan_vm(&f, bloom, max, now));
    let hab = bloom.hab.as_ref().map(|h| hab_vm(&f, h, now));
    let cyan_recent = cyan.as_ref().is_some_and(|c| c.recent);
    let hab_recent = hab.as_ref().is_some_and(|h| h.recent);

    // Sections.
    let mut sections = Vec::new();
    if let Some(c) = &cyan {
        let mut rows: Vec<Row> = Vec::new();
        if let Some(q) = &c.max {
            rows.push(row(f.strings.get(Key::BloomRowMax), &q.full()));
        }
        if let Some(d) = &c.image_date {
            rows.push(row(f.strings.get(Key::BloomRowComposite), d));
        }
        if let Some(n) = bloom.cyan_valid_cells.filter(|_| c.recent) {
            rows.push(row(
                f.strings.get(Key::BloomRowCells),
                &f.strings.number(n as f32, 0),
            ));
        }
        rows.push(row("", &c.experimental));
        sections.push(Section {
            title: f.strings.get(Key::BloomSectionSatellite).to_owned(),
            rows,
        });
    }
    if let (Some(h), Some(src)) = (&hab, &bloom.hab) {
        let mut rows = vec![
            row(f.strings.get(Key::BloomRowBeach), &src.location),
            row(
                f.strings.get(Key::BloomRowDistance),
                &f.distance(Some(src.distance_m)).full(),
            ),
        ];
        if let Some(l) = h.level {
            rows.push(row(
                f.strings.get(Key::BloomRowLevel),
                &capitalize(f.strings.safety(level_key(l))),
            ));
        }
        if let Some(c) = &h.confidence {
            rows.push(row(f.strings.get(Key::BloomRowConfidence), c));
        }
        rows.push(row(
            f.strings.get(Key::BloomRowValid),
            &format!("{}, {}", f.date_of(src.valid_at), f.clock(src.valid_at)),
        ));
        if !h.recent {
            rows.push(row("", &h.text));
        }
        sections.push(Section {
            title: f.strings.get(Key::BloomSectionForecast).to_owned(),
            rows,
        });
    }

    // Headline and compact lines: a count, else the forecast level, else nothing.
    let headline = match (&cyan, &hab) {
        (Some(CyanVm { max: Some(q), .. }), _) => q.clone(),
        (_, Some(HabVm { level: Some(l), .. })) => Quantity {
            value: None,
            text: capitalize(f.strings.safety(level_key(*l))),
            unit: String::new(),
        },
        _ => Quantity {
            value: None,
            text: f.strings.get(Key::Missing).to_owned(),
            unit: String::new(),
        },
    };
    let mut lines: Vec<String> = Vec::new();
    lines.extend(cyan.as_ref().map(|c| c.text.clone()));
    lines.extend(hab.as_ref().map(|h| h.text.clone()));
    lines.truncate(2);

    // Provenance: the satellite series, then the forecast.
    let provs: Vec<&Provenance> = max
        .map(Series::provenance)
        .into_iter()
        .chain(bloom.hab.as_ref().map(|h| &h.provenance))
        .collect();
    let main = provs.first().copied();
    let latest = max.and_then(|s| latest_at(s, now).map(|l| l.time));
    let shown_at = latest.or_else(|| bloom.hab.as_ref().map(|h| h.valid_at));

    let title = f.strings.card_title(CardId::Bloom).to_owned();
    let state = if !cyan_recent && !hab_recent {
        CardState::Stale {
            reason: f
                .strings
                .safety(SafetyKey::BloomNoRecentSatellite)
                .to_owned(),
        }
    } else {
        match main {
            Some(p) => card_state(input, &f, Measurement::Cyanobacteria, p, latest),
            None => CardState::Fresh,
        }
    };
    let mut notices = Vec::new();
    if cyan_recent {
        notices.push(f.notice(NoticeKind::Approximate));
    }
    if hab_recent {
        notices.push(f.notice(NoticeKind::Modelled));
    }
    let advisory = f.strings.safety(SafetyKey::BloomAdvisory).to_owned();
    let first = lines.first().cloned().unwrap_or_default();
    let accessibility = format!("{title}: {first}. {advisory}");

    Some(BloomCard {
        chrome: CardChrome {
            id: CardId::Bloom,
            title,
            footnote: main.map(|p| f.footnote_aged(&first, p, now, shown_at)),
            credits: credits(input, &provs),
            state,
            notices,
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark: Vec::new(),
        },
        full: BloomFull {
            cyan,
            hab,
            advisory,
            sections,
            links: bloom
                .links
                .iter()
                .map(|l| LinkVm {
                    label: l.label.clone(),
                    url: l.url.clone(),
                })
                .collect(),
        },
    })
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::model::ExternalLink;
    use crate::provenance::{Quality, SourceId, StationKind, ValueKind};
    use crate::view::cards::flow::fixtures::{NOW, input, metric, prov, run, us};
    use crate::view::{DisplayUnits, Voice};

    fn cyan_prov() -> Provenance {
        prov(
            SourceId::CYAN,
            "oneida",
            "Oneida Lake",
            StationKind::Buoy,
            ValueKind::Satellite,
            Quality::Provisional,
        )
    }

    fn date(month: u8, day: u8) -> LocalDate {
        LocalDate {
            year: 2026,
            month,
            day,
        }
    }

    pub(in crate::view) fn lake(units: DisplayUnits, info: BloomInfo) -> ViewInput {
        let mut input = input(units, Voice::Luna);
        input.snapshot.spot.name = "Oneida Lake".into();
        input.snapshot.water.bloom = Some(info);
        input
            .snapshot
            .attribution
            .push(crate::provenance::Attribution {
                source: SourceId::CYAN,
                credit: "U.S. EPA Cyanobacteria Assessment Network (CyAN)".into(),
                url: Some("https://www.epa.gov/cyanohabs".into()),
            });
        input
    }

    pub(in crate::view) fn recent_composite() -> BloomInfo {
        BloomInfo {
            cyan_max_cells_ml: Some(run(cyan_prov(), NOW - 3 * DAY_S, 900, &[108_642.6])),
            cyan_valid_cells: Some(6),
            cyan_image_date: Some(date(11, 2)),
            hab: None,
            links: vec![ExternalLink {
                label: "NOAA bloom forecasts".into(),
                url: "https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/".into(),
            }],
        }
    }

    pub(in crate::view) fn stale_composite() -> BloomInfo {
        BloomInfo {
            cyan_max_cells_ml: Some(run(cyan_prov(), NOW - 225 * DAY_S, 900, &[9_000.0])),
            cyan_valid_cells: Some(4),
            cyan_image_date: Some(date(3, 28)),
            ..BloomInfo::default()
        }
    }

    pub(in crate::view) fn forecast(level: HabLevel, valid_at: i64) -> HabForecast {
        HabForecast {
            location: "Clearwater Beach".into(),
            level,
            confidence: Some(0.9),
            valid_at: UnixTime(valid_at),
            distance_m: 3_200.0,
            provenance: prov(
                SourceId::NOAA_HAB,
                "rif",
                "Clearwater Beach",
                StationKind::Buoy,
                ValueKind::Modelled,
                Quality::Approved,
            ),
        }
    }

    fn card(input: &ViewInput) -> BloomCard {
        build(input).expect("a bloom card")
    }

    #[test]
    fn a_recent_composite_shows_the_count_labelled_experimental() {
        let c = card(&lake(us(), recent_composite()));
        assert_eq!(c.compact.headline.full(), "108,643 cells/mL");
        assert!(c.compact.lines[0].starts_with("Satellite estimate up to 108,643 cells/mL"));
        let cyan = c.full.cyan.as_ref().unwrap();
        assert!(cyan.recent);
        assert!(cyan.experimental.starts_with("Experimental:"));
        assert!(c.full.advisory.contains("people and dogs"));
        assert_eq!(c.chrome.credits.len(), 1);
        assert_eq!(c.full.links.len(), 1);
        assert!(matches!(c.chrome.state, CardState::Fresh));
    }

    #[test]
    fn a_stale_composite_says_no_recent_satellite_data_never_no_bloom() {
        let c = card(&lake(us(), stale_composite()));
        let cyan = c.full.cyan.as_ref().unwrap();
        assert!(!cyan.recent);
        assert!(cyan.max.is_none());
        assert_eq!(
            c.compact.lines[0],
            "No recent satellite data (last composite Mar 28)"
        );
        assert!(c.compact.headline.value.is_none());
        assert!(matches!(c.chrome.state, CardState::Stale { .. }));
        let json = serde_json::to_string(&c).unwrap().to_lowercase();
        assert!(!json.contains("no bloom"), "{json}");
        assert!(!json.contains("9,000"), "a stale count is not shown");
    }

    #[test]
    fn an_empty_bloom_record_has_no_recent_satellite_data() {
        let c = card(&lake(us(), BloomInfo::default()));
        assert_eq!(c.compact.lines[0], "No recent satellite data");
        assert!(matches!(c.chrome.state, CardState::Stale { .. }));
        assert!(c.chrome.footnote.is_none());
    }

    #[test]
    fn no_bloom_data_no_card() {
        assert!(build(&input(us(), Voice::Luna)).is_none());
    }

    #[test]
    fn a_forecast_gives_its_level_and_the_modelled_notice() {
        let info = BloomInfo {
            hab: Some(forecast(HabLevel::High, NOW)),
            ..BloomInfo::default()
        };
        let c = card(&lake(us(), info));
        assert_eq!(c.compact.headline.text, "High");
        assert_eq!(
            c.compact.lines,
            ["Respiratory irritation high at Clearwater Beach, 2.0 mi"]
        );
        assert!(c.full.cyan.is_none());
        let hab = c.full.hab.as_ref().unwrap();
        assert_eq!(hab.confidence.as_deref(), Some("0.9 of 1"));
        assert_eq!(c.chrome.notices.len(), 1);
        assert!(matches!(c.chrome.state, CardState::Fresh));
    }

    #[test]
    fn an_old_forecast_is_no_recent_forecast_without_a_level() {
        let info = BloomInfo {
            hab: Some(forecast(HabLevel::High, NOW - 3 * DAY_S)),
            ..BloomInfo::default()
        };
        let c = card(&lake(us(), info));
        let hab = c.full.hab.as_ref().unwrap();
        assert!(!hab.recent);
        assert!(hab.level.is_none());
        assert_eq!(c.compact.lines, ["No recent bloom forecast"]);
        assert!(matches!(c.chrome.state, CardState::Stale { .. }));
    }

    #[test]
    fn the_words_are_the_same_in_both_voices() {
        let luna = lake(us(), recent_composite());
        let mut plain = luna.clone();
        plain.voice = Voice::Plain;
        let (a, b) = (card(&luna), card(&plain));
        assert_eq!(a.compact, b.compact);
        assert_eq!(a.full, b.full);
    }

    #[test]
    fn dates_round_trip_through_the_day_count() {
        let d = LocalDate {
            year: 2026,
            month: 11,
            day: 2,
        };
        assert_eq!(day_of(date_start(d)), d);
        assert_eq!(day_of(UnixTime(-1)).day, 31);
    }

    #[test]
    fn snapshots_of_a_recent_and_a_stale_composite_and_a_forecast() {
        insta::assert_json_snapshot!("recent_us", card(&lake(us(), recent_composite())));
        insta::assert_json_snapshot!("stale_metric", card(&lake(metric(), stale_composite())));
        let info = BloomInfo {
            hab: Some(forecast(HabLevel::Low, NOW)),
            ..BloomInfo::default()
        };
        insta::assert_json_snapshot!("forecast_us", card(&lake(us(), info)));
    }
}
