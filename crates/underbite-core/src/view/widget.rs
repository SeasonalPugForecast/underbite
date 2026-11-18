//! Widget and Live Activity payloads: entries precomputed for the next hours, every value
//! already in words, so the widgets never format.
//!
//! A [`WidgetPayload`] is built for a spot ([`spot_payload`]) or for a trip ([`trip_payload`],
//! with up to two of its places for the largest iPad sizes). Each place's tide chart is built
//! once and shared by the entries; each [`WidgetEntry`] holds the tide, river flow, snack time
//! and go/no-go as they read at its instant, from the same cards and trip view the app shows,
//! each with its credit line. Values are in display units and words in the chosen voice and
//! locale: a payload is built again when any of them changes.
//!
//! [`trip_day_activity`] gives the trip-day Live Activity its target instants and every label;
//! only the countdown is drawn by the system's timer.

use std::collections::BTreeMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::TideSeries;
use crate::offline::tables::{TideStation, tide_table};
use crate::time::{UnixTime, local_date, local_date_bounds, zone};
use crate::trip::grid::{PlaceInputs, PlaceKey, TideConstants, TripGrid};
use crate::trip::model::{Trip, TripPlace};
use crate::view::cards::sun_moon::computed_credit;
use crate::view::cards::tide::{TideEventKind, TideStage};
use crate::view::cards::{current, flow, tide, windows};
use crate::view::chart::{LOOKBACK_S, Zoned, grid};
use crate::view::format::Formatter;
use crate::view::strings::Key;
use crate::view::trip::{TripAppState, TripCompact, build_trip_view};
use crate::view::{
    CardChrome, CardState, ChartVm, Compact, CreditVm, Notice, Quantity, RiverRanges, TimelineSpan,
    TrendVm, ViewInput,
};

/// Entries after the first: one per local hour.
const ENTRY_HOURS: i64 = 12;
/// Hours of tide an entry shows after its instant; [`LOOKBACK_S`] before it.
const SHOWN_AHEAD_H: i64 = 21;
/// Minutes between the tide chart's points (the on-the-hour ones are `major`, for small sizes).
const CHART_STEP_MIN: u16 = 30;
/// Places a trip payload carries.
const TRIP_PLACES: usize = 2;
/// Spacing of tides computed from harmonic constants.
const TIDE_STEP: Duration = Duration::from_mins(6);
const HOUR_S: i64 = 3_600;
const DAY_S: i64 = 86_400;

// ---------------------------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------------------------

/// What a payload is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSubject {
    Spot,
    Trip,
}

/// Everything a widget reads for one spot or one trip, built ahead for the next hours.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetPayload {
    pub subject: WidgetSubject,
    /// The spot's or the trip's id.
    pub id: String,
    /// The spot's or the trip's name.
    pub name: String,
    /// When the payload was built, unix seconds.
    pub generated_at: i64,
    /// One for a spot; for a trip, its first two places with data.
    pub places: Vec<WidgetPlace>,
    /// The first at `generated_at`, then one on each local hour for 12 hours.
    pub entries: Vec<WidgetEntry>,
}

/// What a place's entries share.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetPlace {
    /// The spot's id.
    pub id: String,
    pub name: String,
    /// The tide from 3 h before the first entry to 21 h after the last, every 30 min, with the
    /// highs and lows labelled and night shaded; each entry says which part to show, and marks
    /// now at its own `t` (the chart's `now_t` is the first entry's).
    pub tide_chart: Option<ChartVm>,
    /// A week of river flow (or stage) in display units, oldest first; empty without a gauge.
    pub flow_spark: Vec<f32>,
}

/// What the widgets show from one instant on.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetEntry {
    /// When the entry becomes current, unix seconds.
    pub t: i64,
    /// One per place of the payload, in its order.
    pub places: Vec<WidgetPlaceEntry>,
    /// The trip's go/no-go; trip payloads only.
    pub trip: Option<WidgetTrip>,
}

/// One place at an entry's instant; a part is absent when the place has no such data.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetPlaceEntry {
    pub tide: Option<WidgetTide>,
    pub flow: Option<WidgetFlow>,
    /// The next fishing window: "snack time" in Luna's voice.
    pub snack: Option<WidgetCard>,
}

/// A card's compact form with what a widget must show beside it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetCard {
    /// "Tide", "River flow", "Snack times".
    pub title: String,
    pub headline: Quantity,
    /// At most two.
    pub lines: Vec<String>,
    /// Stale and far values are greyed with the reason.
    pub state: CardState,
    /// The sources' names: "NOAA CO-OPS".
    pub credit: String,
    /// "Not for navigation." with tides; safety text.
    pub notices: Vec<Notice>,
    pub accessibility: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetTide {
    pub card: WidgetCard,
    /// Rising, falling or slack, for the arrow.
    pub stage: TideStage,
    /// The part of the place's `tide_chart` to show, unix seconds.
    pub start_t: i64,
    pub end_t: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetFlow {
    pub card: WidgetCard,
    /// The direction for the arrow, and "Rising 0.4 cfs in 3 h".
    pub trend: Option<TrendVm>,
    /// Against normal for the date, short: "Normal", "Below normal".
    pub normal: Option<String>,
}

/// The next trip's go/no-go.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WidgetTrip {
    pub compact: TripCompact,
    /// The sources' names the trip rests on.
    pub credit: String,
}

/// The payload of a spot's widgets from its view input; `input.now` is the first entry.
pub fn spot_payload(input: &ViewInput) -> WidgetPayload {
    let spot = &input.snapshot.spot;
    let times = entry_times(&spot.time_zone, input.now);
    WidgetPayload {
        subject: WidgetSubject::Spot,
        id: spot.id.clone(),
        name: spot.name.clone(),
        generated_at: input.now.0,
        places: vec![place(&spot.id, &spot.name, input)],
        entries: times
            .iter()
            .map(|&t| WidgetEntry {
                t: t.0,
                places: vec![place_entry(&at(input, t))],
                trip: None,
            })
            .collect(),
    }
}

/// The payload of a trip's widget: its go/no-go at each entry and its first two places with a
/// snapshot (tides from their harmonic constants where the offline tables have them).
///
/// `grid` and `inputs` are the ones the trip view is built from; of `input` the settings and
/// `now` are read, as [`build_trip_view`] does.
pub fn trip_payload(
    trip: &Trip,
    grid: &TripGrid,
    inputs: &BTreeMap<PlaceKey, PlaceInputs>,
    input: &ViewInput,
) -> WidgetPayload {
    let places: Vec<(&TripPlace, ViewInput)> = trip_places(trip, inputs, input)
        .into_iter()
        .take(TRIP_PLACES)
        .collect();
    let zone = trip
        .places
        .first()
        .map_or(input.snapshot.spot.time_zone.as_str(), |p| {
            p.time_zone.as_str()
        });
    let times = entry_times(zone, input.now);
    let app = TripAppState::default();
    WidgetPayload {
        subject: WidgetSubject::Trip,
        id: trip.id.clone(),
        name: trip.name.clone(),
        generated_at: input.now.0,
        places: places
            .iter()
            .map(|(p, i)| place(&p.spot_id, &p.name, i))
            .collect(),
        entries: times
            .iter()
            .map(|&t| {
                let view = build_trip_view(trip, grid, inputs, &app, &at(input, t));
                WidgetEntry {
                    t: t.0,
                    places: places.iter().map(|(_, i)| place_entry(&at(i, t))).collect(),
                    trip: Some(WidgetTrip {
                        credit: credit_line(&view.attribution),
                        compact: view.compact,
                    }),
                }
            })
            .collect(),
    }
}

/// `now`, then each local whole hour for [`ENTRY_HOURS`] hours.
fn entry_times(zone: &str, now: UnixTime) -> Vec<UnixTime> {
    let z = Zoned::new(zone);
    let hours = grid(now.0 + 1, now.0 + ENTRY_HOURS * HOUR_S, 15)
        .into_iter()
        .filter(|t| z.on_the_hour(*t))
        .map(UnixTime);
    std::iter::once(now).chain(hours).collect()
}

/// The input as it reads at `t`. The cards' own charts are not used, so their span is short.
fn at(input: &ViewInput, t: UnixTime) -> ViewInput {
    ViewInput {
        now: t,
        timeline_span: TimelineSpan {
            hours: 1,
            step_minutes: 60,
        },
        ..input.clone()
    }
}

/// What the entries of a place share, built at `input.now`.
fn place(id: &str, name: &str, input: &ViewInput) -> WidgetPlace {
    let span = ViewInput {
        timeline_span: TimelineSpan {
            hours: u16::try_from(ENTRY_HOURS + SHOWN_AHEAD_H).unwrap_or(u16::MAX),
            step_minutes: CHART_STEP_MIN,
        },
        ..input.clone()
    };
    WidgetPlace {
        id: id.to_owned(),
        name: name.to_owned(),
        tide_chart: tide::build(&span).and_then(|c| c.full.chart),
        flow_spark: flow::build(input)
            .map(|c| c.compact.spark)
            .unwrap_or_default(),
    }
}

/// A place at `input.now`.
fn place_entry(input: &ViewInput) -> WidgetPlaceEntry {
    let t = input.now.0;
    WidgetPlaceEntry {
        tide: tide::build(input).map(|c| WidgetTide {
            card: card(&c.chrome, c.compact),
            stage: c.full.stage,
            start_t: t - LOOKBACK_S,
            end_t: t + SHOWN_AHEAD_H * HOUR_S,
        }),
        flow: flow::build(input).map(|c| WidgetFlow {
            card: card(&c.chrome, c.compact),
            trend: c.full.trend,
            normal: c.full.percentile.map(|p| p.class_label),
        }),
        snack: windows::build(input).map(|c| card(&c.chrome, c.compact)),
    }
}

fn card(chrome: &CardChrome, compact: Compact) -> WidgetCard {
    WidgetCard {
        title: chrome.title.clone(),
        headline: compact.headline,
        lines: compact.lines,
        state: chrome.state.clone(),
        credit: credit_line(&chrome.credits),
        notices: chrome.notices.clone(),
        accessibility: chrome.accessibility.clone(),
    }
}

/// The credits' source names, each once, in order: "NOAA CO-OPS · NOAA CO-OPS harmonics".
fn credit_line(credits: &[CreditVm]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for c in credits {
        if !names.contains(&c.source.as_str()) {
            names.push(&c.source);
        }
    }
    names.join(" · ")
}

/// The trip's places that have a snapshot, in the trip's order, each with its view input.
fn trip_places<'a>(
    trip: &'a Trip,
    inputs: &BTreeMap<PlaceKey, PlaceInputs>,
    input: &ViewInput,
) -> Vec<(&'a TripPlace, ViewInput)> {
    trip.places
        .iter()
        .filter_map(|p| Some((p, place_input(input, inputs.get(&PlaceKey::of(p))?)?)))
        .collect()
}

/// A trip place's view input: the settings of `input` with the place's snapshot and river
/// ranges, and tides computed from its harmonic constants when it has them (as the trip grid
/// prefers them).
fn place_input(input: &ViewInput, place: &PlaceInputs) -> Option<ViewInput> {
    let mut snapshot = place.snapshot.clone()?;
    if let Some(tide) = computed_tide(place, input.now) {
        snapshot.marine.tide = Some(tide);
    }
    Some(ViewInput {
        snapshot,
        river_ranges: (place.river != RiverRanges::default()).then_some(place.river),
        ..input.clone()
    })
}

/// Tides from a day before `now` to two days after, from the place's constants.
fn computed_tide(place: &PlaceInputs, now: UnixTime) -> Option<TideSeries> {
    let (station, base) = match place.tide_constants.as_ref()? {
        TideConstants::Harmonic { harmonics, base } => (TideStation::Harmonic(harmonics), base),
        TideConstants::Subordinate {
            reference,
            offsets,
            base,
        } => (TideStation::Subordinate { reference, offsets }, base),
    };
    let (from, to) = (UnixTime(now.0 - DAY_S), UnixTime(now.0 + 2 * DAY_S));
    tide_table(station, base.clone(), from, to, TIDE_STEP).ok()
}

// ---------------------------------------------------------------------------------------------
// The trip-day Live Activity
// ---------------------------------------------------------------------------------------------

/// A time the Live Activity counts down to, with its words.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ActivityTarget {
    /// The instant the system timer counts down to, unix seconds.
    pub at: i64,
    /// What stands beside the timer: "High tide in".
    pub label: String,
    /// The target in words, for the compact views and once stale: "High 4.6 ft at 3:12 PM".
    pub text: String,
}

/// The trip-day Live Activity's content at one place.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TripDayActivity {
    /// The trip's name.
    pub title: String,
    /// The place the times are for: the trip's first with a tide, else its first with data.
    pub place: String,
    /// When the content was built, unix seconds.
    pub generated_at: i64,
    /// The next high or low.
    pub next_tide_change: Option<ActivityTarget>,
    /// The next slack water; current stations only.
    pub slack: Option<ActivityTarget>,
    /// Today's sunset while it is ahead.
    pub sunset: Option<ActivityTarget>,
    /// When the content goes out of date: the first target, else `ends_at`.
    pub stale_at: i64,
    /// What to show once stale.
    pub stale: String,
    /// When to end the activity: today's sunset, else the end of the local day.
    pub ends_at: i64,
    /// The sources' names.
    pub credit: String,
    /// "Not for navigation." and any other tide or current notice; safety text.
    pub notices: Vec<Notice>,
    pub accessibility: String,
}

/// The Live Activity of a trip day, from the trip's first place with a tide (else its first
/// place with a snapshot) at `input.now`. Whether today is a trip day is the trip view's to say.
pub fn trip_day_activity(
    trip: &Trip,
    inputs: &BTreeMap<PlaceKey, PlaceInputs>,
    input: &ViewInput,
) -> TripDayActivity {
    let places = trip_places(trip, inputs, input);
    let chosen = places
        .iter()
        .find(|(_, i)| i.snapshot.marine.tide.is_some())
        .or_else(|| places.first());
    if let Some((p, i)) = chosen {
        return activity(&trip.name, &p.name, i);
    }
    // No place has data: the words alone, ending with the local day.
    let f = Formatter::for_input(input);
    let first = trip.places.first();
    let zone = first.map_or("UTC", |p| p.time_zone.as_str());
    let ends_at = day_end(zone, input.now).unwrap_or(input.now.0);
    TripDayActivity {
        title: trip.name.clone(),
        place: first.map(|p| p.name.clone()).unwrap_or_default(),
        generated_at: input.now.0,
        next_tide_change: None,
        slack: None,
        sunset: None,
        stale_at: ends_at,
        stale: f.strings.get(Key::ActivityStale).to_owned(),
        ends_at,
        credit: String::new(),
        notices: Vec::new(),
        accessibility: trip.name.clone(),
    }
}

/// The end of the local day of `now` in `zone`.
fn day_end(zone_name: &str, now: UnixTime) -> Option<i64> {
    let tz = zone(zone_name).ok()?;
    let today = local_date(now, &tz).ok()?;
    Some(local_date_bounds(today, &tz).ok()?.1.0)
}

fn activity(title: &str, place: &str, input: &ViewInput) -> TripDayActivity {
    let f = Formatter::for_input(input);
    let now = input.now;
    let event_at = |event: &str, time: &str| {
        f.strings
            .fmt(Key::EventAt, &[("event", event), ("time", time)])
    };
    let mut credits: Vec<CreditVm> = Vec::new();
    let mut notices: Vec<Notice> = Vec::new();
    let mut take = |chrome: &CardChrome| {
        credits.extend(chrome.credits.iter().cloned());
        for n in &chrome.notices {
            if !notices.iter().any(|m| m.kind == n.kind) {
                notices.push(n.clone());
            }
        }
    };

    let tide_card = tide::build(input);
    let next_tide_change = tide_card.as_ref().and_then(|c| {
        let e = c.full.next.first()?;
        take(&c.chrome);
        let event = match &e.height {
            Some(h) => format!("{} {}", e.label, h.full()),
            None => e.label.clone(),
        };
        Some(ActivityTarget {
            at: e.t,
            label: f
                .strings
                .get(match e.kind {
                    TideEventKind::High => Key::ActivityHighIn,
                    TideEventKind::Low => Key::ActivityLowIn,
                })
                .to_owned(),
            text: event_at(&event, &e.time),
        })
    });
    let current_card = current::build(input);
    let slack = current_card.as_ref().and_then(|c| {
        let e = c.full.next_slack.as_ref()?;
        take(&c.chrome);
        Some(ActivityTarget {
            at: e.t,
            label: f.strings.get(Key::ActivitySlackIn).to_owned(),
            text: event_at(&e.label, &e.time),
        })
    });

    let zone_name = &input.snapshot.spot.time_zone;
    let today = zone(zone_name)
        .ok()
        .and_then(|tz| local_date(now, &tz).ok());
    let sunset_t = input
        .snapshot
        .sky
        .days
        .iter()
        .find(|d| Some(d.date) == today)
        .and_then(|d| d.sun.sunset);
    let sunset = sunset_t.filter(|t| *t > now).map(|t| {
        credits.push(computed_credit(f.strings));
        ActivityTarget {
            at: t.0,
            label: f.strings.get(Key::ActivitySunsetIn).to_owned(),
            text: event_at(f.strings.get(Key::Sunset), &f.clock(t)),
        }
    });
    let ends_at = sunset_t
        .map(|t| t.0)
        .or_else(|| day_end(zone_name, now))
        .unwrap_or(now.0);
    let targets = [&next_tide_change, &slack, &sunset];
    let stale_at = targets
        .iter()
        .filter_map(|t| t.as_ref().map(|t| t.at))
        .min()
        .unwrap_or(ends_at);

    let mut said = vec![format!("{title}, {place}")];
    said.extend(
        targets
            .iter()
            .filter_map(|t| t.as_ref().map(|t| t.text.clone())),
    );
    let mut accessibility = format!("{}.", said.join(". "));
    for n in &notices {
        accessibility.push(' ');
        accessibility.push_str(&n.text);
    }

    TripDayActivity {
        title: title.to_owned(),
        place: place.to_owned(),
        generated_at: now.0,
        next_tide_change,
        slack,
        sunset,
        stale_at,
        stale: f.strings.get(Key::ActivityStale).to_owned(),
        ends_at,
        credit: credit_line(&credits),
        notices,
        accessibility,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::cards::current::tests::nyh1927;
    use crate::view::cards::flow::fixtures::{croton_input, metric, us};
    use crate::view::trip::tests::{Built, battery_three_days, two_places, view_input};
    use crate::view::{MarkerKind, NoticeKind, TrendDirection, Voice};

    /// The Battery at 8 AM EST on 2026-11-08: the recorded predictions and the computed sky.
    fn battery() -> ViewInput {
        crate::view::timeline::tests::input()
    }

    fn size(v: &impl Serialize) -> usize {
        serde_json::to_vec(v).unwrap().len()
    }

    /// The payload with its first entry only: the shape, for the JSON snapshot.
    fn first_entry(p: &WidgetPayload) -> WidgetPayload {
        let mut p = p.clone();
        p.entries.truncate(1);
        p
    }

    fn card_text(c: &WidgetCard) -> String {
        let state = match &c.state {
            CardState::Fresh => String::new(),
            CardState::Stale { reason }
            | CardState::Far { reason }
            | CardState::Unavailable { reason } => format!(" [{reason}]"),
        };
        format!(
            "{} | {} | {}{state}",
            c.headline.full(),
            c.lines.join(" | "),
            c.credit
        )
    }

    /// Every entry as lines, hours after the first: how the words change over the 12 hours.
    fn render(p: &WidgetPayload) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for e in &p.entries {
            let minutes = (e.t - p.generated_at) / 60;
            let _ = writeln!(out, "+{}:{:02}", minutes / 60, minutes % 60);
            for (place, x) in p.places.iter().zip(&e.places) {
                if let Some(t) = &x.tide {
                    let _ = writeln!(
                        out,
                        "  {} tide {:?} {} | shows {}..{} h",
                        place.name,
                        t.stage,
                        card_text(&t.card),
                        (t.start_t - e.t) / HOUR_S,
                        (t.end_t - e.t) / HOUR_S
                    );
                }
                if let Some(f) = &x.flow {
                    let _ = writeln!(
                        out,
                        "  {} flow {} | {} | {}",
                        place.name,
                        card_text(&f.card),
                        f.trend.as_ref().map_or("", |t| t.text.as_str()),
                        f.normal.as_deref().unwrap_or("")
                    );
                }
                if let Some(s) = &x.snack {
                    let _ = writeln!(out, "  {} snack {}", place.name, card_text(s));
                }
            }
            if let Some(t) = &e.trip {
                let _ = writeln!(
                    out,
                    "  trip {} | {} | {} | {}",
                    t.compact.when, t.compact.headline, t.compact.line, t.credit
                );
            }
        }
        out
    }

    #[test]
    fn battery_entries_are_hourly_with_the_tide_and_its_credit() {
        let input = battery();
        let p = spot_payload(&input);
        assert_eq!((p.subject, p.id.as_str()), (WidgetSubject::Spot, "battery"));
        assert_eq!(p.generated_at, input.now.0);
        // Now is 8:00 AM: the entries are 8 AM to 8 PM.
        assert_eq!(p.entries.len(), 13);
        assert_eq!(p.entries[0].t, input.now.0);
        assert!(p.entries.windows(2).all(|w| w[1].t - w[0].t == HOUR_S));

        let place = &p.places[0];
        let chart = place.tide_chart.as_ref().unwrap();
        assert_eq!(chart.unit, "ft");
        let points = &chart.series[0].points;
        assert!(points.windows(2).all(|w| w[1].t - w[0].t == 1_800));
        let (first, last) = (points[0].t, points[points.len() - 1].t);
        assert_eq!(first, input.now.0 - LOOKBACK_S);
        assert_eq!(last, input.now.0 + (ENTRY_HOURS + SHOWN_AHEAD_H) * HOUR_S);
        let extremes: Vec<&str> = chart
            .markers
            .iter()
            .filter(|m| matches!(m.kind, MarkerKind::High | MarkerKind::Low))
            .map(|m| m.label.as_str())
            .collect();
        assert!(extremes.len() >= 5, "{extremes:?}");
        assert!(
            extremes
                .iter()
                .all(|l| (l.starts_with("High ") || l.starts_with("Low ")) && l.ends_with(" ft"))
        );
        let mut stages = Vec::new();
        for e in &p.entries {
            let tide = e.places[0].tide.as_ref().unwrap();
            assert_eq!(tide.card.credit, "NOAA CO-OPS");
            assert!(
                tide.card
                    .notices
                    .iter()
                    .any(|n| n.kind == NoticeKind::NotForNavigation)
            );
            assert!(tide.start_t >= first && tide.end_t <= last);
            assert_eq!(tide.end_t - tide.start_t, 24 * HOUR_S);
            assert!(e.places[0].snack.is_some());
            assert!(e.trip.is_none());
            if !stages.contains(&tide.stage) {
                stages.push(tide.stage);
            }
        }
        // Twelve hours see the tide turn.
        assert!(stages.contains(&TideStage::Rising) && stages.contains(&TideStage::Falling));
        // Small enough to read at every timeline reload.
        assert!(size(&p) < 40_000, "{} bytes", size(&p));
        insta::assert_json_snapshot!("battery_payload", first_entry(&p));
        insta::assert_snapshot!("battery_entries", render(&p));
    }

    #[test]
    fn croton_flow_has_its_trend_and_normal() {
        let mut input = croton_input(us(), Voice::Luna);
        input.snapshot.spot.id = "croton".into();
        let p = spot_payload(&input);
        assert_eq!((p.id.as_str(), p.name.as_str()), ("croton", "Croton River"));
        assert_eq!(p.entries.len(), 13);
        assert!(p.places[0].tide_chart.is_none());
        assert_eq!(p.places[0].flow_spark.len(), 25);
        for e in &p.entries {
            let flow = e.places[0].flow.as_ref().unwrap();
            assert_eq!(flow.card.headline.full(), "12.4 cfs");
            assert_eq!(
                flow.trend.as_ref().map(|t| t.direction),
                Some(TrendDirection::Rising)
            );
            assert_eq!(flow.normal.as_deref(), Some("Normal"));
            assert_eq!(flow.card.credit, "USGS");
            assert!(e.places[0].tide.is_none());
        }
        assert!(size(&p) < 20_000, "{} bytes", size(&p));
        insta::assert_json_snapshot!("croton_payload", first_entry(&p));
        insta::assert_snapshot!("croton_entries", render(&p));
    }

    #[test]
    fn a_payload_is_in_the_units_it_was_built_with() {
        let mut input = battery();
        let feet = spot_payload(&input);
        input.units = metric();
        let metres = spot_payload(&input);
        let unit = |p: &WidgetPayload| p.places[0].tide_chart.as_ref().unwrap().unit.clone();
        assert_eq!((unit(&feet), unit(&metres)), ("ft".into(), "m".into()));
        let headline = |p: &WidgetPayload| {
            p.entries[0].places[0]
                .tide
                .as_ref()
                .unwrap()
                .card
                .headline
                .unit
                .clone()
        };
        assert_eq!(
            (headline(&feet), headline(&metres)),
            ("ft MLLW".into(), "m MLLW".into())
        );
    }

    #[test]
    fn a_trip_payload_has_two_places_and_the_go_no_go() {
        let input = view_input();
        let built = two_places(&input);
        let p = trip_payload(&built.trip, &built.grid, &built.inputs, &input);
        assert_eq!(p.subject, WidgetSubject::Trip);
        assert_eq!(p.name, built.trip.name);
        let names: Vec<&str> = p.places.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["The Battery", "Croton River"]);
        assert_eq!(p.entries.len(), 13);
        for e in &p.entries {
            assert_eq!(e.places.len(), 2);
            // The Battery's tides come from its harmonic constants.
            let tide = e.places[0].tide.as_ref().unwrap();
            assert_eq!(tide.card.credit, "NOAA CO-OPS · NOAA CO-OPS harmonics");
            assert!(e.places[1].flow.is_some() && e.places[1].tide.is_none());
            let trip = e.trip.as_ref().unwrap();
            assert_eq!(
                trip.compact.title,
                format!("Next trip: {}", built.trip.name)
            );
            assert!(trip.credit.contains("Open-Meteo"), "{}", trip.credit);
        }
        assert!(size(&p) < 60_000, "{} bytes", size(&p));
        insta::assert_json_snapshot!("trip_payload", first_entry(&p));
        insta::assert_snapshot!("trip_entries", render(&p));
    }

    /// The Battery trip with the Narrows current station in its snapshot.
    fn battery_trip(input: &ViewInput) -> Built {
        let mut built = battery_three_days(input);
        let current = nyh1927().snapshot.marine.tidal_current;
        for place in built.inputs.values_mut() {
            place.snapshot.as_mut().unwrap().marine.tidal_current = current.clone();
        }
        built
    }

    #[test]
    fn the_activity_counts_down_to_the_tide_the_slack_and_sunset() {
        let input = view_input();
        let built = battery_trip(&input);
        let a = trip_day_activity(&built.trip, &built.inputs, &input);
        assert_eq!(a.title, built.trip.name);
        assert_eq!(a.place, "The Battery");
        let tide = a.next_tide_change.as_ref().unwrap();
        let slack = a.slack.as_ref().unwrap();
        let sunset = a.sunset.as_ref().unwrap();
        for target in [tide, slack, sunset] {
            assert!(target.at > input.now.0);
        }
        assert!(["High tide in", "Low tide in"].contains(&tide.label.as_str()));
        assert_eq!(slack.label, "Slack in");
        assert_eq!(sunset.label, "Sunset in");
        assert!(sunset.text.starts_with("Sunset at "));
        assert_eq!(a.ends_at, sunset.at);
        assert_eq!(a.stale_at, tide.at.min(slack.at).min(sunset.at));
        assert!(a.credit.starts_with("NOAA CO-OPS") && a.credit.ends_with("Computed on device"));
        assert!(
            a.notices
                .iter()
                .any(|n| n.kind == NoticeKind::NotForNavigation)
        );
        assert!(a.accessibility.contains(&tide.text) && a.accessibility.contains(&slack.text));
        insta::assert_json_snapshot!("battery_activity", a);
    }

    #[test]
    fn after_sunset_the_activity_ends() {
        let mut input = view_input();
        let built = battery_trip(&input);
        let sunset = trip_day_activity(&built.trip, &built.inputs, &input)
            .sunset
            .unwrap()
            .at;
        input.now = UnixTime(sunset + 60);
        let a = trip_day_activity(&built.trip, &built.inputs, &input);
        assert!(a.sunset.is_none());
        assert_eq!(a.ends_at, sunset);
        assert!(a.next_tide_change.is_some());
    }

    #[test]
    fn without_data_the_activity_has_only_its_words() {
        let input = view_input();
        let built = battery_trip(&input);
        let a = trip_day_activity(&built.trip, &BTreeMap::new(), &input);
        assert!(a.next_tide_change.is_none() && a.slack.is_none() && a.sunset.is_none());
        assert_eq!(a.stale_at, a.ends_at);
        // The end of Nov 8 in New York, 05:00 UTC on Nov 9.
        assert_eq!(a.ends_at, 1_794_200_400);
        assert_eq!(a.credit, "");
        assert_eq!(a.stale, "Out of date: open the app to update the times");
    }

    #[test]
    fn schema_snapshot() {
        insta::assert_json_snapshot!(
            "widget_payload_schema",
            schemars::schema_for!(WidgetPayload)
        );
        insta::assert_json_snapshot!(
            "trip_day_activity_schema",
            schemars::schema_for!(TripDayActivity)
        );
    }
}
