//! Tidal current speed, direction and slack times.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{CurrentEvent, CurrentKind, CurrentSeries, Measurement};
use crate::provenance::{Quality, StationKind};
use crate::time::UnixTime;
use crate::view::cards::tide::{credits, state};
use crate::view::chart::{Zoned, grid, speed_at, ticks, window};
use crate::view::format::{Formatter, Phase};
use crate::view::strings::{Key, source_name};
use crate::view::{
    BandKind, CardChrome, CardId, CardState, ChartBand, ChartMarker, ChartPoint, ChartSeries,
    ChartVm, Compact, MarkerKind, NoticeKind, Quantity, Row, Section, SeriesRole, SeriesStyle,
    ViewInput,
};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CurrentCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: CurrentFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurrentPhase {
    Flood,
    Ebb,
    Slack,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurrentEventKind {
    MaxFlood,
    MaxEbb,
    Slack,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CurrentEventVm {
    pub t: i64,
    pub time: String,
    pub kind: CurrentEventKind,
    /// "Max flood", "Slack", ...
    pub label: String,
    /// None for slack.
    pub speed: Option<Quantity>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CurrentFull {
    pub phase: CurrentPhase,
    /// "Flooding", "Ebbing", "Slack"
    pub phase_label: String,
    pub speed_now: Option<Quantity>,
    /// Where the water is heading now, "NNE 025°".
    pub direction_now: Option<String>,
    pub next_slack: Option<CurrentEventVm>,
    pub events: Vec<CurrentEventVm>,
    pub flood_direction: Option<String>,
    pub ebb_direction: Option<String>,
    /// "Bin 3 at 12 ft": the depth the station measures at.
    pub bin_depth: Option<String>,
    /// Speed over time with slack windows highlighted.
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

/// Below this speed (m/s, about 0.1 kn) the water counts as slack.
const SLACK_MS: f32 = 0.05;
/// How far either side of a slack event the window reaches when there is no curve, seconds.
const SLACK_HALF_WINDOW_S: i64 = 15 * 60;

fn event_vm(f: &Formatter, e: &CurrentEvent) -> CurrentEventVm {
    let (kind, label, speed) = match e.kind {
        CurrentKind::MaxFlood => (
            CurrentEventKind::MaxFlood,
            f.strings.get(Key::MaxFlood).to_owned(),
            Some(f.current_speed(Some(e.speed_ms.abs()))),
        ),
        CurrentKind::MaxEbb => (
            CurrentEventKind::MaxEbb,
            f.strings.get(Key::MaxEbb).to_owned(),
            Some(f.current_speed(Some(e.speed_ms.abs()))),
        ),
        CurrentKind::Slack => (
            CurrentEventKind::Slack,
            f.phase(Phase::Slack).to_owned(),
            None,
        ),
    };
    CurrentEventVm {
        t: e.time.0,
        time: f.clock(e.time),
        kind,
        label,
        speed,
    }
}

/// Runs of the curve below the slack speed, as (start, end) pairs inside the window.
fn slack_runs(
    series: &CurrentSeries,
    events: &[CurrentEvent],
    start: i64,
    end: i64,
) -> Vec<(i64, i64)> {
    let Some(curve) = &series.curve else {
        return events
            .iter()
            .filter(|e| e.kind == CurrentKind::Slack)
            .map(|e| {
                (
                    e.time.0 - SLACK_HALF_WINDOW_S,
                    e.time.0 + SLACK_HALF_WINDOW_S,
                )
            })
            .filter(|(a, b)| *b >= start && *a <= end)
            .collect();
    };
    let mut runs: Vec<(i64, i64)> = Vec::new();
    let mut open: Option<(i64, i64)> = None;
    for s in curve.window(UnixTime(start), UnixTime(end)) {
        if s.value.abs() < SLACK_MS {
            open = Some(open.map_or((s.time.0, s.time.0), |(a, _)| (a, s.time.0)));
        } else if let Some(run) = open.take() {
            runs.push(run);
        }
    }
    runs.extend(open);
    runs
}

#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn build(input: &ViewInput) -> Option<CurrentCard> {
    let snapshot = &input.snapshot;
    let series = snapshot.marine.tidal_current.as_ref()?;
    if series.events.is_empty() && series.curve.is_none() {
        return None;
    }
    let f = Formatter::for_input(input);
    let z = Zoned::new(&snapshot.spot.time_zone);
    let now = input.now;
    let mut events = series.events.clone();
    events.sort_by_key(|e| e.time);

    let speed = speed_at(series, &events, now);
    let phase = speed.map_or(CurrentPhase::Unknown, |v| {
        if v.abs() < SLACK_MS {
            CurrentPhase::Slack
        } else if v > 0.0 {
            CurrentPhase::Flood
        } else {
            CurrentPhase::Ebb
        }
    });
    let phase_label = match phase {
        CurrentPhase::Flood => f.phase(Phase::Flood),
        CurrentPhase::Ebb => f.phase(Phase::Ebb),
        CurrentPhase::Slack => f.phase(Phase::Slack),
        CurrentPhase::Unknown => f.strings.get(Key::Missing),
    }
    .to_owned();
    let speed_now = speed.map(|v| f.current_speed(Some(v.abs())));
    // A station without flood and ebb directions shows the phase alone.
    let toward = match phase {
        CurrentPhase::Flood => series.flood_dir_deg,
        CurrentPhase::Ebb => series.ebb_dir_deg,
        CurrentPhase::Slack | CurrentPhase::Unknown => None,
    };
    let direction_now = toward.map(|d| f.direction(Some(d)));

    let upcoming: Vec<&CurrentEvent> = events.iter().filter(|e| e.time > now).collect();
    let next_slack = upcoming
        .iter()
        .find(|e| e.kind == CurrentKind::Slack)
        .map(|e| event_vm(&f, e));
    let (start, end) = window(input);
    let listed: Vec<CurrentEventVm> = upcoming
        .iter()
        .filter(|e| e.time.0 <= end)
        .map(|e| event_vm(&f, e))
        .collect();

    let bin_depth = match (series.bin, series.depth_m) {
        (Some(bin), Some(d)) => Some(f.strings.fmt(
            Key::BinAt,
            &[
                ("bin", &bin.to_string()),
                ("depth", &f.length(Some(d)).full()),
            ],
        )),
        (Some(bin), None) => Some(f.strings.fmt(Key::BinOnly, &[("bin", &bin.to_string())])),
        (None, Some(d)) => Some(
            f.strings
                .fmt(Key::DepthAt, &[("depth", &f.length(Some(d)).full())]),
        ),
        (None, None) => None,
    };

    let step = input.timeline_span.step_minutes;
    let signed = |v: f32| {
        let q = f.current_speed(Some(v));
        (q.value.unwrap_or_default(), q.full())
    };
    let (label, style, times): (&str, _, Vec<i64>) = if series.curve.is_some() {
        (
            f.strings.get(Key::Predicted),
            SeriesStyle::Line,
            grid(start, end, step),
        )
    } else {
        (
            f.strings.get(Key::PredictedEvents),
            SeriesStyle::Dots,
            events
                .iter()
                .map(|e| e.time.0)
                .filter(|t| *t >= start && *t <= end)
                .collect(),
        )
    };
    let points: Vec<ChartPoint> = times
        .into_iter()
        .filter_map(|t| {
            let (value, text) = signed(speed_at(series, &events, UnixTime(t))?);
            Some(ChartPoint {
                t,
                value,
                label: text,
                major: z.on_the_hour(t),
            })
        })
        .collect();
    let chart = (!points.is_empty()).then(|| ChartVm {
        title: f.strings.card_title(CardId::Current).to_owned(),
        unit: input.units.current_speed.symbol().to_owned(),
        x_ticks: ticks(&z, &f, start, end),
        series: vec![ChartSeries {
            label: label.to_owned(),
            role: SeriesRole::Predicted,
            style,
            points,
        }],
        bands: slack_runs(series, &events, start, end)
            .into_iter()
            .map(|(a, b)| ChartBand {
                kind: BandKind::Slack,
                start_t: a,
                end_t: b,
                label: f.phase(Phase::Slack).to_owned(),
            })
            .collect(),
        rules: Vec::new(),
        markers: events
            .iter()
            .filter(|e| e.kind == CurrentKind::Slack && e.time.0 >= start && e.time.0 <= end)
            .map(|e| ChartMarker {
                kind: MarkerKind::Slack,
                t: e.time.0,
                value: None,
                label: format!("{} {}", f.phase(Phase::Slack), f.clock(e.time)),
            })
            .collect(),
        now_t: Some(now.0),
        accessibility: format!(
            "{}: {}.",
            f.strings.get(Key::CurrentSpeed),
            listed
                .iter()
                .map(|e| f.strings.fmt(
                    Key::EventAt,
                    &[("event", &e.label.to_lowercase()), ("time", &e.time)]
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    });

    let p = &series.provenance;
    let mut line = phase_label.clone();
    if let (Some(q), CurrentPhase::Flood | CurrentPhase::Ebb) = (&speed_now, phase) {
        line = format!("{phase_label} {}", q.full());
        if let Some(d) = &direction_now {
            line = f
                .strings
                .fmt(Key::TowardLine, &[("line", &line), ("direction", d)]);
        }
    }
    let mut lines = vec![line.clone()];
    if let Some(n) = &next_slack {
        lines.push(f.strings.fmt(Key::NextSlack, &[("time", &n.time)]));
    }
    let mut notices = vec![f.notice(NoticeKind::NotForNavigation)];
    let subordinate = p
        .station
        .as_ref()
        .is_some_and(|s| s.kind == StationKind::CurrentSubordinate);
    if subordinate
        || series.curve.is_none()
        || series
            .curve
            .as_ref()
            .is_some_and(|c| c.provenance().quality == Quality::Estimated)
    {
        notices.push(f.notice(NoticeKind::Approximate));
    }
    let card_state = if upcoming.is_empty() && speed.is_none() {
        CardState::Unavailable {
            reason: f.strings.get(Key::NoData).to_owned(),
        }
    } else {
        state(input, &f, p, Measurement::TidalCurrent)
    };
    let mut rows = vec![Row {
        label: f.strings.get(Key::Station).to_owned(),
        value: p
            .station
            .as_ref()
            .map_or_else(|| source_name(&p.source), |s| s.name.clone()),
        detail: p.distance_m.map(|m| f.distance(Some(m)).full()),
    }];
    if let Some(d) = &bin_depth {
        rows.push(Row {
            label: f.strings.get(Key::Depth).to_owned(),
            value: d.clone(),
            detail: None,
        });
    }

    Some(CurrentCard {
        chrome: CardChrome {
            id: CardId::Current,
            title: f.strings.card_title(CardId::Current).to_owned(),
            footnote: Some(f.footnote(
                &format!("{} {line}", f.strings.card_title(CardId::Current)),
                p,
                now,
            )),
            credits: credits(snapshot, &p.source),
            state: card_state,
            notices,
            accessibility: match &next_slack {
                Some(n) => format!(
                    "{line}. {}.",
                    f.strings.fmt(Key::NextSlack, &[("time", &n.time)])
                ),
                None => format!("{line}."),
            },
        },
        compact: Compact {
            headline: speed_now.clone().unwrap_or_default(),
            lines,
            spark: Vec::new(),
        },
        full: CurrentFull {
            phase,
            phase_label,
            speed_now,
            direction_now,
            next_slack,
            events: listed,
            flood_direction: series.flood_dir_deg.map(|d| f.direction(Some(d))),
            ebb_direction: series.ebb_dir_deg.map(|d| f.direction(Some(d))),
            bin_depth,
            chart,
            sections: vec![Section {
                title: f.strings.get(Key::Station).to_owned(),
                rows,
            }],
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CurrentSeries;
    use crate::provenance::{Provenance, StationKind, StationRef, ValueKind};
    use crate::series::{Sample, Series};
    use crate::time::parse_coops;
    use crate::units::{HeightUnit, SpeedUnit};
    use crate::view::Voice;
    use crate::view::cards::tide::testing::{at, fixture, input};
    use whether_core::presentation::TimeFormat;

    /// The harmonic current station NYH1927 at bin 13, from the recorded week.
    fn nyh1927() -> ViewInput {
        let mut input = input();
        let base = input.snapshot.marine.tide.take().unwrap().provenance;
        let prov = Provenance {
            station: Some(StationRef {
                id: "NYH1927".into(),
                name: "The Narrows, bin 13".into(),
                kind: StationKind::CurrentHarmonic,
                bin: Some(13),
                ..base.station.clone().unwrap()
            }),
            distance_m: Some(4_000.0),
            ..base
        };
        let cm = |v: &serde_json::Value| -> f32 { v.as_f64().map(|f| f as f32).unwrap() / 100.0 };
        let events =
            fixture("coops-NYH1927-bin13-max-slack-2026-11-08.json")["current_predictions"]["cp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| CurrentEvent {
                    time: parse_coops(c["Time"].as_str().unwrap()).unwrap(),
                    kind: match c["Type"].as_str().unwrap() {
                        "flood" => CurrentKind::MaxFlood,
                        "ebb" => CurrentKind::MaxEbb,
                        _ => CurrentKind::Slack,
                    },
                    speed_ms: cm(&c["Velocity_Major"]),
                })
                .collect();
        let curve = fixture("coops-NYH1927-bin13-currents-predictions-6min-2026-11-08.json")
            ["current_predictions"]["cp"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                Sample::new(
                    parse_coops(c["Time"].as_str().unwrap()).unwrap(),
                    cm(&c["Velocity_Major"]),
                )
            })
            .collect();
        input.snapshot.marine.tidal_current = Some(CurrentSeries {
            events,
            curve: Some(Series::new(
                Provenance {
                    kind: ValueKind::Predicted,
                    ..prov.clone()
                },
                curve,
            )),
            flood_dir_deg: Some(11.0),
            ebb_dir_deg: Some(183.0),
            bin: Some(13),
            depth_m: Some(2.1),
            observed: None,
            provenance: prov,
        });
        input
    }

    #[test]
    fn names_the_next_slack_then_the_ebb() {
        let card = build(&nyh1927()).unwrap();
        let slack = card.full.next_slack.as_ref().unwrap();
        assert_eq!(slack.t, at("2026-11-08T15:41:00Z").0);
        assert_eq!(slack.time, "10:41 AM");
        assert!(slack.speed.is_none());
        let ebb = &card.full.events[1];
        assert_eq!(ebb.t, at("2026-11-08T19:08:00Z").0);
        assert_eq!(ebb.kind, CurrentEventKind::MaxEbb);
        assert_eq!(ebb.label, "Max ebb");
        let speed = ebb.speed.as_ref().unwrap();
        assert_eq!((speed.text.as_str(), speed.unit.as_str()), ("2.6", "kn"));
        assert!((speed.value.unwrap() - 2.607).abs() < 0.01);
        assert_eq!(card.full.events[0].kind, CurrentEventKind::Slack);
    }

    #[test]
    fn flooding_now_with_direction_and_bin() {
        let card = build(&nyh1927()).unwrap();
        assert_eq!(card.full.phase, CurrentPhase::Flood);
        assert_eq!(card.full.phase_label, "Flood");
        assert_eq!(card.full.direction_now.as_deref(), Some("N 11°"));
        assert_eq!(card.full.flood_direction.as_deref(), Some("N 11°"));
        assert_eq!(card.full.ebb_direction.as_deref(), Some("S 183°"));
        assert_eq!(card.full.bin_depth.as_deref(), Some("Bin 13 at 6.9 ft"));
        assert!(card.compact.lines[0].starts_with("Flood "));
        assert!(card.compact.lines[0].contains(" toward N 11°"));
        assert_eq!(card.compact.lines[1], "Next slack 10:41 AM");
        let kinds: Vec<_> = card.chrome.notices.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, [NoticeKind::NotForNavigation]);
        let chart = card.full.chart.as_ref().unwrap();
        assert_eq!(chart.unit, "kn");
        assert!(chart.bands.len() >= 2);
        assert!(chart.series[0].points.iter().any(|p| p.value < 0.0));
    }

    #[test]
    fn a_station_without_directions_shows_the_phase_alone() {
        let mut input = nyh1927();
        let series = input.snapshot.marine.tidal_current.as_mut().unwrap();
        series.flood_dir_deg = None;
        series.ebb_dir_deg = None;
        series.bin = None;
        let card = build(&input).unwrap();
        assert_eq!(card.full.phase, CurrentPhase::Flood);
        assert!(card.full.direction_now.is_none());
        assert!(card.full.flood_direction.is_none());
        assert_eq!(card.full.bin_depth.as_deref(), Some("At 6.9 ft"));
        assert!(!card.compact.lines[0].contains("toward"));
    }

    #[test]
    fn without_a_curve_the_chart_is_the_events_and_the_card_is_approximate() {
        let mut input = nyh1927();
        input.snapshot.marine.tidal_current.as_mut().unwrap().curve = None;
        let card = build(&input).unwrap();
        let kinds: Vec<_> = card.chrome.notices.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            [NoticeKind::NotForNavigation, NoticeKind::Approximate]
        );
        let chart = card.full.chart.as_ref().unwrap();
        assert_eq!(chart.series[0].style, SeriesStyle::Dots);
        assert_eq!(card.full.phase, CurrentPhase::Flood);
    }

    #[test]
    fn snapshots_in_both_voices_and_unit_systems() {
        insta::assert_json_snapshot!("current_us_luna", build(&nyh1927()).unwrap());
        let mut metric = nyh1927();
        metric.voice = Voice::Plain;
        metric.clock = TimeFormat::H24;
        metric.units.height = HeightUnit::M;
        metric.units.current_speed = SpeedUnit::Ms;
        let card = build(&metric).unwrap();
        assert_eq!(card.full.next_slack.as_ref().unwrap().time, "10:41");
        assert_eq!(card.full.bin_depth.as_deref(), Some("Bin 13 at 2.1 m"));
        insta::assert_json_snapshot!("current_metric_plain", card);
    }
}
