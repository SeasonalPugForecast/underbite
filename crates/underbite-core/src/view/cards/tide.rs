//! Tide heights, highs and lows, and the stage now.

use schemars::JsonSchema;
use serde::Serialize;

use crate::datum::{Height, VerticalDatum};
use crate::derive::tide_stage::{TideDirection, tide_stage};
use crate::model::{
    Measurement, MoonPhase, SkyConditions, SpotSnapshot, TideEvent, TideKind, TideSeries,
    TideSourceKind,
};
use crate::provenance::{Provenance, Quality, SourceId};
use crate::series::Series;
use crate::time::{UnixTime, day_number};
use crate::view::cards::sun_moon::{MoonPhaseVm, day_label};
use crate::view::chart::{Zoned, grid, level_at, ticks, window};
use crate::view::format::Formatter;
use crate::view::strings::{Key, source_name};
use crate::view::{
    BandKind, CardChrome, CardId, CardState, ChartBand, ChartMarker, ChartPoint, ChartSeries,
    ChartVm, Compact, CreditVm, MarkerKind, NoticeKind, Quantity, Row, Section, SeriesRole,
    SeriesStyle, TimedLabel, ViewInput,
};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: TideFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TideStage {
    Rising,
    Falling,
    HighSlack,
    LowSlack,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TideEventKind {
    High,
    Low,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideEventVm {
    /// Unix seconds.
    pub t: i64,
    pub time: String,
    pub kind: TideEventKind,
    /// "High" or "Low".
    pub label: String,
    /// None when the source gives times only (modelled sea level).
    pub height: Option<Quantity>,
}

/// One local day of the tide table.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideDayVm {
    /// "Sat Nov 14"
    pub date: String,
    pub events: Vec<TideEventVm>,
    pub sunrise: Option<TimedLabel>,
    pub sunset: Option<TimedLabel>,
    pub moon: Option<MoonPhaseVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideFull {
    pub stage: TideStage,
    /// "Rising", "Falling", ...
    pub stage_label: String,
    /// The height now, interpolated; None without heights.
    pub height_now: Option<Quantity>,
    /// The next high and the next low, soonest first.
    pub next: Vec<TideEventVm>,
    /// "above MLLW"; None when heights are not shown.
    pub datum_label: Option<String>,
    /// False for modelled sea level: relative curve, no heights.
    pub heights_shown: bool,
    /// Prediction with the observed level over it, sunrise and sunset shading, moon phase.
    pub chart: Option<ChartVm>,
    pub days: Vec<TideDayVm>,
    pub sections: Vec<Section>,
}

/// Within this many minutes of a high or low the water is called slack.
const SLACK_MINUTES: f64 = 15.0;

/// The credits owed for a series: the source's, and for on-device harmonics also the data's.
pub(super) fn credits(snapshot: &SpotSnapshot, source: &SourceId) -> Vec<CreditVm> {
    let ids = if *source == SourceId::COOPS_HARMONICS {
        vec![SourceId::COOPS, SourceId::COOPS_HARMONICS]
    } else {
        vec![source.clone()]
    };
    ids.iter()
        .map(|id| {
            let found = snapshot.attribution.iter().find(|a| a.source == *id);
            CreditVm {
                source: source_name(id),
                text: found.map_or_else(|| default_credit(id), |a| a.credit.clone()),
                link: found.and_then(|a| a.url.clone()),
                licence: None,
            }
        })
        .collect()
}

fn default_credit(id: &SourceId) -> String {
    match id.as_str() {
        "coops" => "Predictions and observations: NOAA CO-OPS".to_owned(),
        "coops-harmonics" => "Computed from NOAA harmonic constants".to_owned(),
        "open-meteo-marine" => "Sea level model: Open-Meteo".to_owned(),
        _ => source_name(id),
    }
}

/// Fresh, or far when the station lies beyond the user's distance limit.
pub(super) fn state(
    input: &ViewInput,
    f: &Formatter,
    p: &Provenance,
    measurement: Measurement,
) -> CardState {
    let limit = input.limit(measurement).and_then(|l| l.max_distance_m);
    match (p.distance_m, limit) {
        (Some(d), Some(max)) if d > max => CardState::Far {
            reason: f.far_reason(d, max),
        },
        _ => CardState::Fresh,
    }
}

fn moon_label(phase: MoonPhase) -> &'static str {
    match phase {
        MoonPhase::New => "New moon",
        MoonPhase::WaxingCrescent => "Waxing crescent",
        MoonPhase::FirstQuarter => "First quarter",
        MoonPhase::WaxingGibbous => "Waxing gibbous",
        MoonPhase::Full => "Full moon",
        MoonPhase::WaningGibbous => "Waning gibbous",
        MoonPhase::LastQuarter => "Last quarter",
        MoonPhase::WaningCrescent => "Waning crescent",
    }
}

fn kind_label(kind: TideKind) -> &'static str {
    match kind {
        TideKind::High => "High",
        TideKind::Low => "Low",
    }
}

fn event_vm(f: &Formatter, tide: &TideSeries, shown: bool, e: &TideEvent) -> TideEventVm {
    TideEventVm {
        t: e.time.0,
        time: f.clock(e.time),
        kind: match e.kind {
            TideKind::High => TideEventKind::High,
            TideKind::Low => TideEventKind::Low,
        },
        label: kind_label(e.kind).to_owned(),
        height: shown.then(|| {
            f.height(e.height_m.map(|metres| Height {
                datum: tide.datum,
                metres,
            }))
        }),
    }
}

/// Night as bands between each sunset and the next sunrise, clipped to the window.
fn night_bands(sky: &SkyConditions, start: i64, end: i64) -> Vec<ChartBand> {
    let mut rises: Vec<i64> = sky
        .days
        .iter()
        .filter_map(|d| d.sun.sunrise)
        .map(|t| t.0)
        .collect();
    let mut sets: Vec<i64> = sky
        .days
        .iter()
        .filter_map(|d| d.sun.sunset)
        .map(|t| t.0)
        .collect();
    rises.sort_unstable();
    sets.sort_unstable();
    let mut spans = Vec::new();
    if let (Some(&rise), Some(&set)) = (rises.first(), sets.first())
        && rise < set
    {
        spans.push((start, rise));
    }
    for &set in &sets {
        let rise = rises.iter().copied().find(|r| *r > set).unwrap_or(end);
        spans.push((set, rise));
    }
    spans
        .into_iter()
        .map(|(a, b)| (a.max(start), b.min(end)))
        .filter(|(a, b)| a < b)
        .map(|(start_t, end_t)| ChartBand {
            kind: BandKind::Night,
            start_t,
            end_t,
            label: "Night".to_owned(),
        })
        .collect()
}

/// Observed samples inside the window, one per grid step.
fn thinned(series: &Series<f32>, start: i64, end: i64, step_minutes: u16) -> Vec<(i64, f32)> {
    let step = i64::from(step_minutes.max(1)) * 60;
    let mut last = None;
    let mut out = Vec::new();
    for s in series.window(UnixTime(start), UnixTime(end)) {
        let bucket = s.time.0.div_euclid(step);
        if last != Some(bucket) {
            last = Some(bucket);
            out.push((s.time.0, s.value));
        }
    }
    out
}

fn point(z: &Zoned, f: &Formatter, t: i64, metres: f32) -> ChartPoint {
    let q = f.length(Some(metres));
    ChartPoint {
        t,
        value: q.value.unwrap_or_default(),
        label: q.full(),
        major: z.on_the_hour(t),
    }
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn build(input: &ViewInput) -> Option<TideCard> {
    let snapshot = &input.snapshot;
    let tide = snapshot.marine.tide.as_ref()?;
    if tide.events.is_empty() && tide.curve.is_none() {
        return None;
    }
    let f = Formatter::for_input(input);
    let z = Zoned::new(&snapshot.spot.time_zone);
    let now = input.now;
    let model = tide.source_kind == TideSourceKind::ModelMean;
    let shown = !model && tide.datum != VerticalDatum::ModelMean;
    let height = |metres: Option<f32>| {
        f.height(metres.map(|metres| Height {
            datum: tide.datum,
            metres,
        }))
    };

    let mut events = tide.events.clone();
    events.sort_by_key(|e| e.time);
    let upcoming: Vec<&TideEvent> = events.iter().filter(|e| e.time > now).collect();
    let mut next: Vec<TideEventVm> = [TideKind::High, TideKind::Low]
        .iter()
        .filter_map(|k| upcoming.iter().find(|e| e.kind == *k))
        .map(|e| event_vm(&f, tide, shown, e))
        .collect();
    next.sort_by_key(|e| e.t);

    let stage = tide_stage(tide, now);
    let (stage_kind, stage_text) = match &stage {
        None => (TideStage::Unknown, f.strings.get(Key::Missing)),
        Some(s) if !s.approximate_times && s.near_turn(SLACK_MINUTES) => {
            let nearest = if s.elapsed_s <= s.to_next_s {
                s.previous.kind
            } else {
                s.next.kind
            };
            let kind = match nearest {
                TideKind::High => TideStage::HighSlack,
                TideKind::Low => TideStage::LowSlack,
            };
            (kind, f.strings.get(Key::Slack))
        }
        Some(s) if s.direction == TideDirection::Rising => {
            (TideStage::Rising, f.strings.get(Key::Rising))
        }
        Some(_) => (TideStage::Falling, f.strings.get(Key::Falling)),
    };
    let stage_label = stage_text.to_owned();
    let height_now = shown
        .then(|| level_at(tide, &events, now))
        .flatten()
        .map(|m| height(Some(m)));

    let (start, end) = window(input);
    let step = input.timeline_span.step_minutes;
    let levels: Vec<(i64, f32)> = grid(start, end, step)
        .into_iter()
        .filter_map(|t| Some((t, level_at(tide, &events, UnixTime(t))?)))
        .collect();
    let (lo, hi) = levels
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), (_, v)| {
            (lo.min(*v), hi.max(*v))
        });
    let mut series = Vec::new();
    if shown {
        series.push(ChartSeries {
            label: "Predicted".to_owned(),
            role: SeriesRole::Predicted,
            style: SeriesStyle::Line,
            points: levels.iter().map(|(t, v)| point(&z, &f, *t, *v)).collect(),
        });
        if let Some(observed) = &tide.observed {
            let points: Vec<ChartPoint> = thinned(observed, start, now.0, step)
                .into_iter()
                .map(|(t, v)| point(&z, &f, t, v))
                .collect();
            if !points.is_empty() {
                series.push(ChartSeries {
                    label: "Observed".to_owned(),
                    role: SeriesRole::Observed,
                    style: SeriesStyle::Line,
                    points,
                });
            }
        }
    } else if !levels.is_empty() {
        // A relative curve: scaled to 0..1, labelled with the time, never with a height.
        let range = hi - lo;
        series.push(ChartSeries {
            label: "Modelled sea level".to_owned(),
            role: SeriesRole::Modelled,
            style: SeriesStyle::Line,
            points: levels
                .iter()
                .map(|(t, v)| ChartPoint {
                    t: *t,
                    value: if range > 0.0 { (v - lo) / range } else { 0.5 },
                    label: f.clock(UnixTime(*t)),
                    major: z.on_the_hour(*t),
                })
                .collect(),
        });
    }

    let mut markers: Vec<ChartMarker> = events
        .iter()
        .filter(|e| e.time.0 >= start && e.time.0 <= end)
        .map(|e| {
            let q = shown.then(|| f.length(e.height_m));
            ChartMarker {
                kind: match e.kind {
                    TideKind::High => MarkerKind::High,
                    TideKind::Low => MarkerKind::Low,
                },
                t: e.time.0,
                value: q.as_ref().and_then(|q| q.value),
                label: match &q {
                    Some(q) => format!("{} {}", kind_label(e.kind), q.full()),
                    None => format!("{} {}", kind_label(e.kind), f.clock(e.time)),
                },
            }
        })
        .collect();
    let described: Vec<String> = markers
        .iter()
        .map(|m| format!("{} at {}", m.label, f.clock(UnixTime(m.t))))
        .collect();
    for day in &snapshot.sky.days {
        for (kind, label, at) in [
            (MarkerKind::Sunrise, "Sunrise", day.sun.sunrise),
            (MarkerKind::Sunset, "Sunset", day.sun.sunset),
        ] {
            if let Some(t) = at.filter(|t| t.0 >= start && t.0 <= end) {
                markers.push(ChartMarker {
                    kind,
                    t: t.0,
                    value: None,
                    label: format!("{label} {}", f.clock(t)),
                });
            }
        }
    }
    markers.sort_by_key(|m| m.t);
    let chart = (!series.is_empty()).then(|| ChartVm {
        title: f.strings.card_title(CardId::Tide).to_owned(),
        unit: if shown {
            input.units.height.symbol().to_owned()
        } else {
            String::new()
        },
        x_ticks: ticks(&z, &f, start, end),
        series,
        bands: night_bands(&snapshot.sky, start, end),
        rules: Vec::new(),
        markers,
        now_t: Some(now.0),
        accessibility: format!("Tide: {}.", described.join(", ")),
    });

    let today = day_number(z.local(now).date);
    let mut days: Vec<TideDayVm> = Vec::new();
    let mut last_date = None;
    for e in events
        .iter()
        .filter(|e| day_number(z.local(e.time).date) >= today)
    {
        let date = z.local(e.time).date;
        if last_date != Some(date) {
            last_date = Some(date);
            let sky = snapshot.sky.days.iter().find(|d| d.date == date);
            let timed = |label: &str, t: Option<UnixTime>| {
                t.map(|t| TimedLabel {
                    label: label.to_owned(),
                    time: f.clock(t),
                    t: t.0,
                })
            };
            days.push(TideDayVm {
                date: day_label(&f, date),
                events: Vec::new(),
                sunrise: sky.and_then(|d| timed("Sunrise", d.sun.sunrise)),
                sunset: sky.and_then(|d| timed("Sunset", d.sun.sunset)),
                moon: sky.map(|d| MoonPhaseVm {
                    phase: d.moon.phase,
                    label: moon_label(d.moon.phase).to_owned(),
                    illumination: f.percent(Some(d.moon.illuminated_fraction * 100.0)),
                }),
            });
        }
        if let Some(day) = days.last_mut() {
            day.events.push(event_vm(&f, tide, shown, e));
        }
    }

    let datum_label = shown.then(|| {
        let unit_text = height(Some(0.0)).unit;
        let name = unit_text.split_once(' ').map_or("", |(_, d)| d);
        format!("above {name}")
    });
    let p = &tide.provenance;
    let mut rows = vec![Row {
        label: "Station".to_owned(),
        value: p
            .station
            .as_ref()
            .map_or_else(|| source_name(&p.source), |s| s.name.clone()),
        detail: p.distance_m.map(|m| f.distance(Some(m)).full()),
    }];
    if let Some(label) = &datum_label {
        rows.push(Row {
            label: "Heights".to_owned(),
            value: label.clone(),
            detail: None,
        });
    }

    let next_all = upcoming.first().map(|e| event_vm(&f, tide, shown, e));
    let value_text = height_now
        .as_ref()
        .map_or_else(|| "times only".to_owned(), Quantity::full);
    let mut lines = Vec::new();
    if let Some(n) = &next_all {
        lines.push(format!("{stage_label} · {} {}", n.label, n.time));
        if let Some(h) = &n.height {
            lines.push(h.full());
        }
    }
    let spark = if shown {
        chart
            .as_ref()
            .and_then(|c| c.series.first())
            .map(|s| {
                s.points
                    .iter()
                    .filter(|p| p.t >= now.0)
                    .map(|p| p.value)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let headline = height_now.clone().unwrap_or_else(|| Quantity {
        value: None,
        text: next_all.as_ref().map_or_else(
            || f.strings.get(Key::Missing).to_owned(),
            |n| n.time.clone(),
        ),
        unit: String::new(),
    });

    let mut notices = vec![f.notice(NoticeKind::NotForNavigation)];
    if model {
        notices.push(f.notice(NoticeKind::Modelled));
    } else if tide.source_kind == TideSourceKind::SubordinateStation
        || tide.curve.is_none()
        || tide
            .curve
            .as_ref()
            .is_some_and(|c| c.provenance().quality == Quality::Estimated)
    {
        notices.push(f.notice(NoticeKind::Approximate));
    }
    let card_state = if next_all.is_none() {
        CardState::Unavailable {
            reason: f.strings.get(Key::NoData).to_owned(),
        }
    } else {
        state(input, &f, p, Measurement::Tide)
    };
    let accessibility = match &next_all {
        Some(n) => format!(
            "{stage_label}. Next {} {}{}.",
            n.label.to_lowercase(),
            n.time,
            match (&n.height, &datum_label) {
                (Some(h), Some(d)) => {
                    format!(", {} {} {d}", h.text, input.units.height.symbol())
                }
                _ => String::new(),
            }
        ),
        None => stage_label.clone(),
    };

    Some(TideCard {
        chrome: CardChrome {
            id: CardId::Tide,
            title: f.strings.card_title(CardId::Tide).to_owned(),
            footnote: Some(f.footnote(&format!("Tide {value_text}"), p, now)),
            credits: credits(snapshot, &p.source),
            state: card_state,
            notices,
            accessibility,
        },
        compact: Compact {
            headline,
            lines,
            spark,
        },
        full: TideFull {
            stage: stage_kind,
            stage_label,
            height_now,
            next,
            datum_label,
            heights_shown: shown,
            chart,
            days,
            sections: vec![Section {
                title: "Station".to_owned(),
                rows,
            }],
        },
    })
}

#[cfg(test)]
pub(super) mod testing {
    //! Fixture-built inputs for the tide and current cards.

    use crate::datum::VerticalDatum;
    use crate::model::{
        MoonDay, MoonPhase, SkyDay, SunTimes, TideEvent, TideKind, TideSeries, TideSourceKind,
    };
    use crate::provenance::{Provenance, Quality, ValueKind};
    use crate::series::{Sample, Series};
    use crate::time::{LocalDate, UnixTime, parse_coops, parse_rfc3339};
    use crate::view::ViewInput;
    use crate::view::tests::sample_input;

    pub(crate) fn fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/coops/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap()
    }

    pub(crate) fn at(iso: &str) -> UnixTime {
        parse_rfc3339(iso).unwrap()
    }

    /// New York Harbor with the recorded week of CO-OPS predictions, at 14:30 UTC on 2026-11-08.
    pub(crate) fn input() -> ViewInput {
        let mut input = sample_input();
        input.now = at("2026-11-08T14:30:00Z");
        let sample = input.snapshot.marine.tide.take().unwrap();
        let prov = Provenance {
            kind: ValueKind::Predicted,
            quality: Quality::Approved,
            observed_at: at("2026-11-08T00:00:00Z"),
            ..sample.provenance
        };
        let hilo = fixture("coops-8518750-hilo-2026-11-08.json");
        let events = hilo["predictions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| TideEvent {
                time: parse_coops(p["t"].as_str().unwrap()).unwrap(),
                kind: if p["type"] == "H" {
                    TideKind::High
                } else {
                    TideKind::Low
                },
                height_m: Some(p["v"].as_str().unwrap().parse().unwrap()),
            })
            .collect();
        let samples = |name: &str, key: &str| {
            fixture(name)[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    Sample::new(
                        parse_coops(p["t"].as_str().unwrap()).unwrap(),
                        p["v"].as_str().unwrap().parse::<f32>().unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let observed_prov = Provenance {
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
            ..prov.clone()
        };
        input.snapshot.marine.tide = Some(TideSeries {
            source_kind: TideSourceKind::HarmonicStation,
            datum: VerticalDatum::Mllw,
            events,
            curve: Some(Series::new(
                prov.clone(),
                samples(
                    "coops-8518750-predictions-6min-2026-11-08.json",
                    "predictions",
                ),
            )),
            observed: Some(Series::new(
                observed_prov,
                samples("coops-8518750-water-level-latest.json", "data"),
            )),
            provenance: prov,
        });
        // Sunrise 6:25 am and sunset 4:46 pm local on Nov 8 (synthetic values).
        input.snapshot.sky.days = vec![SkyDay {
            date: LocalDate {
                year: 2026,
                month: 11,
                day: 8,
            },
            sun: SunTimes {
                sunrise: Some(at("2026-11-08T11:25:00Z")),
                sunset: Some(at("2026-11-08T21:46:00Z")),
                ..SunTimes::default()
            },
            moon: MoonDay {
                rise: None,
                set: None,
                upper_transit: None,
                lower_transit: None,
                elongation_deg: 300.0,
                illuminated_fraction: 0.31,
                phase: MoonPhase::WaningCrescent,
                nearest_principal: None,
            },
            solunar: Vec::new(),
        }];
        input
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{at, input};
    use super::*;
    use crate::units::{HeightUnit, SpeedUnit};
    use crate::view::Voice;
    use whether_core::presentation::TimeFormat;

    #[test]
    fn next_event_is_the_evening_low_in_feet_mllw() {
        let card = build(&input()).unwrap();
        let low = &card.full.next[0];
        assert_eq!(low.t, at("2026-11-08T18:34:00Z").0);
        assert_eq!(low.time, "1:34 PM");
        assert_eq!(low.kind, TideEventKind::Low);
        let h = low.height.as_ref().unwrap();
        assert_eq!((h.text.as_str(), h.unit.as_str()), ("0.0", "ft MLLW"));
        // The high follows after midnight UTC.
        assert_eq!(card.full.next[1].kind, TideEventKind::High);
        assert_eq!(card.full.next[1].time, "7:27 PM");
        assert_eq!(card.full.stage, TideStage::Falling);
        assert_eq!(card.full.datum_label.as_deref(), Some("above MLLW"));
        assert!(card.full.heights_shown);
        let kinds: Vec<_> = card.chrome.notices.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [NoticeKind::NotForNavigation]);
        assert_eq!(card.chrome.state, CardState::Fresh);
    }

    #[test]
    fn chart_overlays_observed_level_and_marks_the_day() {
        let card = build(&input()).unwrap();
        let chart = card.full.chart.unwrap();
        assert_eq!(chart.unit, "ft");
        assert_eq!(chart.now_t, Some(at("2026-11-08T14:30:00Z").0));
        let roles: Vec<_> = chart.series.iter().map(|s| s.role).collect();
        assert_eq!(roles, [SeriesRole::Predicted, SeriesRole::Observed]);
        assert!(
            chart.series[0]
                .points
                .iter()
                .all(|p| p.label.ends_with(" ft"))
        );
        assert!(chart.series[0].points.iter().any(|p| p.major));
        assert!(chart.markers.iter().any(|m| m.kind == MarkerKind::Sunset));
        assert_eq!(chart.bands.len(), 1);
        let day = &card.full.days[0];
        assert_eq!(day.date, "Sun Nov 8");
        assert_eq!(day.sunrise.as_ref().unwrap().time, "6:25 AM");
        assert_eq!(day.moon.as_ref().unwrap().label, "Waning crescent");
    }

    #[test]
    fn model_mean_shows_times_and_no_heights() {
        let mut input = input();
        let tide = input.snapshot.marine.tide.as_mut().unwrap();
        tide.source_kind = TideSourceKind::ModelMean;
        tide.datum = VerticalDatum::ModelMean;
        tide.observed = None;
        for e in &mut tide.events {
            e.height_m = None;
        }
        let card = build(&input).unwrap();
        assert!(!card.full.heights_shown);
        assert!(card.full.height_now.is_none());
        assert!(card.full.datum_label.is_none());
        assert_eq!(card.full.next[0].time, "1:34 PM");
        assert!(card.full.next[0].height.is_none());
        assert!(
            card.full
                .days
                .iter()
                .flat_map(|d| &d.events)
                .all(|e| e.height.is_none())
        );
        assert_eq!(card.compact.headline.value, None);
        assert_eq!(card.compact.headline.text, "1:34 PM");
        assert_eq!(card.compact.spark, Vec::<f32>::new());
        let json = serde_json::to_string(&card).unwrap();
        assert!(!json.contains("MLLW") && !json.contains(" ft"), "{json}");
        let kinds: Vec<_> = card.chrome.notices.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [NoticeKind::NotForNavigation, NoticeKind::Modelled]);
        let chart = card.full.chart.clone().unwrap();
        assert_eq!(chart.series[0].role, SeriesRole::Modelled);
        assert!(
            chart.series[0]
                .points
                .iter()
                .all(|p| (0.0..=1.0).contains(&p.value))
        );
        insta::assert_json_snapshot!("tide_model_mean", card);
    }

    #[test]
    fn subordinate_and_curveless_series_are_approximate() {
        let mut input = input();
        let tide = input.snapshot.marine.tide.as_mut().unwrap();
        tide.source_kind = TideSourceKind::SubordinateStation;
        tide.curve = None;
        let card = build(&input).unwrap();
        let kinds: Vec<_> = card.chrome.notices.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [NoticeKind::NotForNavigation, NoticeKind::Approximate]
        );
        // The height now comes from a cosine between the events.
        assert!(card.full.height_now.unwrap().value.unwrap() > 0.0);
    }

    #[test]
    fn a_distant_station_is_far_and_no_events_is_no_card() {
        let mut input = input();
        let tide = input.snapshot.marine.tide.as_mut().unwrap();
        tide.provenance.distance_m = Some(80_000.0);
        assert!(matches!(
            build(&input).unwrap().chrome.state,
            CardState::Far { .. }
        ));
        let tide = input.snapshot.marine.tide.as_mut().unwrap();
        tide.events.clear();
        tide.curve = None;
        assert!(build(&input).is_none());
    }

    #[test]
    fn snapshots_in_both_voices_and_unit_systems() {
        let luna = build(&input()).unwrap();
        insta::assert_json_snapshot!("tide_us_luna", luna);
        let mut metric = input();
        metric.voice = Voice::Plain;
        metric.clock = TimeFormat::H24;
        metric.units.height = HeightUnit::M;
        metric.units.current_speed = SpeedUnit::Ms;
        let card = build(&metric).unwrap();
        assert_eq!(card.full.next[0].time, "13:34");
        assert_eq!(card.full.next[0].height.as_ref().unwrap().unit, "m MLLW");
        insta::assert_json_snapshot!("tide_metric_plain", card);
    }
}
