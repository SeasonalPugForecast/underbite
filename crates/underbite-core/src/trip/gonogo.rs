//! Go or no-go: a day and its hours judged against the user's limits.
//!
//! The verdicts come from numbers (stations, models, alerts) and never from forecast text. Every
//! reason is data: which check, how it came out, the measured value and the limit in canonical
//! units, when, and from which source. The view words them.
//!
//! Safety first: a storm ahead, a hurricane cone over the spot, a gale-class marine warning, and,
//! when the limits ask for it, a small-craft advisory or lightning, are hard no-gos whatever the
//! other checks say. A check without data is `Unknown`, never `Go`: a day with an unknown hard
//! check and no failure is `Unknown`. Hours past the end of the forecast are `Unknown`; history is
//! never used.
//!
//! Units of [`Reason::measured`] and [`Reason::limit`] by [`Check`]: wind, gusts and current in
//! m/s, waves in metres, air temperature in °C, visibility in metres, flow in m³/s.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whether_core::condition::Condition;
use whether_core::model::Hourly;
use whether_core::storm::analysis;
use whether_core::storm::hurricane::hurricane_window;

use super::model::TripLimits;
use crate::model::{MarineAlert, Measurement, SpotSnapshot, WaveSample, Wind};
use crate::provenance::SourceId;
use crate::series::{Sample, Series};
use crate::time::{self, LocalDate, UnixTime};
use crate::units::SpeedUnit;
use crate::view::RiverRanges;

const HOUR: i64 = 3_600;
/// An hourly forecast sample covers up to this long when its neighbours are further apart.
const MAX_FORECAST_GAP: i64 = 3 * HOUR;
/// A value within this fraction of a limit is marginal.
const MARGIN: f64 = 0.10;
/// The least margin for the air temperature, °C, where a fraction of a small limit means nothing.
const MIN_TEMPERATURE_MARGIN_C: f64 = 1.0;
const NHC_LINK: &str = "https://www.nhc.noaa.gov/";

/// The overall result for a day or an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every checked limit is met with room to spare.
    Go,
    /// Nothing fails, but a value is within 10 % of a limit.
    Marginal,
    /// A hard limit is exceeded or a safety condition applies.
    NoGo,
    /// Something that decides it is missing, or the time is outside the forecast.
    Unknown,
}

/// What was checked.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// The forecast itself: its end, or its absence.
    Forecast,
    Wind,
    Gust,
    Waves,
    /// Tidal current speed.
    Current,
    AirTemperature,
    Visibility,
    /// River discharge against the user's ranges.
    Flow,
    SmallCraftAdvisory,
    /// A gale-class marine warning (gale, storm, hurricane-force wind, hazardous seas, special
    /// marine warning).
    MarineWarning,
    Lightning,
    /// A significant storm in the forecast models.
    StormAhead,
    /// An NHC forecast cone over the spot.
    Hurricane,
}

impl Check {
    /// Whether the limit is a least value (air temperature, visibility) rather than a greatest.
    pub const fn limit_is_minimum(self) -> bool {
        matches!(self, Self::AirTemperature | Self::Visibility)
    }
}

/// How a check came out.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReasonKind {
    /// The limit is exceeded, or the condition is present.
    Fails,
    /// Within 10 % of the limit.
    Marginal,
    /// The value that decides the check is missing.
    NoData,
    /// The time is past the end of the forecast.
    OutsideForecast,
    /// The time is already over.
    Past,
}

/// One finding. Not words: the view words it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reason {
    pub check: Check,
    pub kind: ReasonKind,
    /// The value found, in canonical units (see the module docs); absent for conditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured: Option<f64>,
    /// The user's limit in the same unit; absent for conditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    /// When it applies: the hour, or an alert's or storm's start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<UnixTime>,
    /// When an alert, storm or cone ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<UnixTime>,
    /// Where the value came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceId>,
    /// The alert's event name or the cyclone's name, as issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The official page for a hurricane (NHC's advisory, or its home page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl Reason {
    fn new(check: Check, kind: ReasonKind) -> Self {
        Self {
            check,
            kind,
            measured: None,
            limit: None,
            at: None,
            until: None,
            source: None,
            subject: None,
            link: None,
        }
    }

    fn no_data(check: Check, at: UnixTime) -> Self {
        Self {
            at: Some(at),
            ..Self::new(check, ReasonKind::NoData)
        }
    }

    /// How far past the limit, for picking the worst of several; larger is worse.
    fn excess(&self) -> f64 {
        match (self.measured, self.limit) {
            (Some(m), Some(l)) if self.check.limit_is_minimum() => l - m,
            (Some(m), Some(l)) => m - l,
            _ => 0.0,
        }
    }
}

/// One hour, starting at `start`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HourVerdict {
    pub start: UnixTime,
    pub verdict: Verdict,
    pub reasons: Vec<Reason>,
}

/// A local day: its verdict, why, and its hours.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DayVerdict {
    #[serde(with = "super::model::date_string")]
    #[schemars(with = "String")]
    pub day: LocalDate,
    pub verdict: Verdict,
    /// One reason per check and outcome, the worst occurrence of each, failures first. Hours
    /// already over are left out.
    pub reasons: Vec<Reason>,
    /// Every local hour of the day (23 or 25 on a daylight-saving change), in order.
    pub hours: Vec<HourVerdict>,
}

/// What the verdicts are built from.
#[derive(Clone, Copy, Debug)]
pub struct DayInputs<'a> {
    pub snapshot: &'a SpotSnapshot,
    /// The user's ranges for the spot's gauge, in m³/s; checked when set.
    pub river: RiverRanges,
}

impl<'a> DayInputs<'a> {
    /// Inputs without river ranges.
    pub fn new(snapshot: &'a SpotSnapshot) -> Self {
        Self {
            snapshot,
            river: RiverRanges::default(),
        }
    }

    /// The same inputs with the user's river ranges.
    #[must_use]
    pub fn with_river(mut self, river: RiverRanges) -> Self {
        self.river = river;
        self
    }
}

/// The limits converted once to canonical units.
struct Canon {
    wind_ms: Option<f64>,
    gust_ms: Option<f64>,
    wave_m: Option<f64>,
    current_ms: Option<f64>,
    min_air_c: Option<f64>,
    min_visibility_m: Option<f64>,
    flow_m3s: Option<f64>,
    no_sca: bool,
    no_lightning: bool,
}

fn knots(kn: f64) -> f64 {
    f64::from(SpeedUnit::Kn.to_canonical(kn as f32))
}

impl Canon {
    fn new(limits: &TripLimits, river: &RiverRanges) -> Self {
        // The tightest of the user's wadeable and blown-out flows.
        let flow = [river.wadeable_below_m3s, river.blown_out_above_m3s]
            .into_iter()
            .flatten()
            .map(f64::from)
            .reduce(f64::min);
        Self {
            wind_ms: limits.max_wind_kn.map(knots),
            gust_ms: limits.max_gust_kn.map(knots),
            wave_m: limits.max_wave_m,
            current_ms: limits.max_current_kn.map(knots),
            min_air_c: limits.min_air_temp_c,
            min_visibility_m: limits.min_visibility_km.map(|km| km * 1_000.0),
            flow_m3s: flow,
            no_sca: limits.no_small_craft_advisory,
            no_lightning: limits.no_lightning,
        }
    }
}

/// A value with where and when it was found.
struct Reading {
    value: f64,
    source: SourceId,
    at: UnixTime,
}

fn greatest(readings: impl IntoIterator<Item = Reading>) -> Option<Reading> {
    readings
        .into_iter()
        .reduce(|a, b| if b.value > a.value { b } else { a })
}

/// Judges a value against a greatest allowed value.
fn judge_max(check: Check, r: &Reading, limit: f64) -> Option<Reason> {
    let kind = if r.value > limit {
        ReasonKind::Fails
    } else if r.value >= limit * (1.0 - MARGIN) {
        ReasonKind::Marginal
    } else {
        return None;
    };
    Some(found(check, kind, r, limit))
}

/// Judges a value against a least allowed value, with `margin` as the marginal band above it.
fn judge_min(check: Check, r: &Reading, limit: f64, margin: f64) -> Option<Reason> {
    let kind = if r.value < limit {
        ReasonKind::Fails
    } else if r.value <= limit + margin {
        ReasonKind::Marginal
    } else {
        return None;
    };
    Some(found(check, kind, r, limit))
}

fn found(check: Check, kind: ReasonKind, r: &Reading, limit: f64) -> Reason {
    Reason {
        measured: Some(r.value),
        limit: Some(limit),
        at: Some(r.at),
        source: Some(r.source.clone()),
        ..Reason::new(check, kind)
    }
}

/// Samples of `series` that fall in the hour starting at `t`.
fn in_hour<T>(series: &Series<T>, t: UnixTime) -> &[Sample<T>] {
    series.window(t, UnixTime(t.0 + HOUR - 1))
}

/// The samples for the hour: those inside it, else the last one up to `reach` seconds before it.
fn for_hour<T>(series: &Series<T>, t: UnixTime, reach: i64) -> &[Sample<T>] {
    let inside = in_hour(series, t);
    if !inside.is_empty() {
        return inside;
    }
    let before = series.window(UnixTime(t.0 - reach), UnixTime(t.0 - 1));
    before.last().map_or(&[], std::slice::from_ref)
}

/// The forecast step that covers `t`, when there is one.
fn hourly_at(hourly: &[Hourly], t: UnixTime) -> Option<&Hourly> {
    let i = hourly.partition_point(|h| h.time <= t);
    let Some(prev) = i.checked_sub(1).and_then(|j| hourly.get(j)) else {
        // Before the first step: it counts when it starts within this hour.
        return hourly.first().filter(|h| h.time.0 < t.0 + HOUR);
    };
    match hourly.get(i) {
        Some(next) => (next.time.0 - prev.time.0 <= MAX_FORECAST_GAP).then_some(prev),
        None => (t.0 < prev.time.0 + step_of_last(hourly)).then_some(prev),
    }
}

fn step_of_last(hourly: &[Hourly]) -> i64 {
    match hourly {
        [.., a, b] if b.time.0 > a.time.0 => (b.time.0 - a.time.0).min(MAX_FORECAST_GAP),
        _ => HOUR,
    }
}

/// When the forecast ends: the last step and its length.
fn horizon(hourly: &[Hourly]) -> Option<UnixTime> {
    hourly
        .last()
        .map(|h| UnixTime(h.time.0 + step_of_last(hourly)))
}

struct Ctx<'a> {
    snap: &'a SpotSnapshot,
    canon: Canon,
    now: UnixTime,
    horizon: Option<UnixTime>,
    has_weather: bool,
}

impl Ctx<'_> {
    fn weather_source(&self) -> Option<SourceId> {
        self.snap
            .weather
            .as_ref()
            .map(|w| SourceId::from_static(w.provider.id()))
    }

    /// The checks that rest on the forecast, for the hour at `t`.
    fn weather_checks(&self, t: UnixTime, out: &mut Vec<Reason>) {
        let hourly = self
            .snap
            .weather
            .as_ref()
            .and_then(|w| hourly_at(&w.hourly, t));
        let source = self.weather_source();
        let weather = |value: f64| {
            source.clone().map(|source| Reading {
                value,
                source,
                at: t,
            })
        };
        let observed = self.observed_wind(t);

        if let Some(limit) = self.canon.wind_ms {
            let forecast = hourly
                .and_then(|h| h.wind_speed)
                .and_then(|v| weather(f64::from(v.0)));
            let seen = observed.iter().map(|s| Reading {
                value: f64::from(s.value.speed_ms),
                source: s.source.clone(),
                at: s.at,
            });
            push_max(
                out,
                Check::Wind,
                greatest(forecast.into_iter().chain(seen)),
                limit,
                t,
            );
        }
        if let Some(limit) = self.canon.gust_ms {
            let forecast = hourly
                .and_then(|h| h.wind_gust)
                .and_then(|v| weather(f64::from(v.0)));
            let seen = observed.iter().filter_map(|s| {
                s.value.gust_ms.map(|g| Reading {
                    value: f64::from(g),
                    source: s.source.clone(),
                    at: s.at,
                })
            });
            push_max(
                out,
                Check::Gust,
                greatest(forecast.into_iter().chain(seen)),
                limit,
                t,
            );
        }
        if let Some(limit) = self.canon.min_air_c {
            let reading = hourly
                .and_then(|h| h.temperature)
                .and_then(|v| weather(f64::from(v.0)));
            let margin = (limit.abs() * MARGIN).max(MIN_TEMPERATURE_MARGIN_C);
            push(
                out,
                Check::AirTemperature,
                reading.and_then(|r| judge_min(Check::AirTemperature, &r, limit, margin)),
                t,
                hourly.and_then(|h| h.temperature).is_some(),
            );
        }
        if let Some(limit) = self.canon.min_visibility_m {
            let reading = hourly
                .and_then(|h| h.visibility)
                .and_then(|v| weather(f64::from(v.0)));
            push(
                out,
                Check::Visibility,
                reading.and_then(|r| judge_min(Check::Visibility, &r, limit, limit * MARGIN)),
                t,
                hourly.and_then(|h| h.visibility).is_some(),
            );
        }
        if self.canon.no_lightning {
            match hourly.map(|h| h.condition) {
                Some(Condition::Thunderstorm(_) | Condition::ThunderstormHail(_)) => {
                    out.push(Reason {
                        at: Some(t),
                        source: self.weather_source(),
                        ..Reason::new(Check::Lightning, ReasonKind::Fails)
                    });
                }
                Some(Condition::Unknown) | None => out.push(Reason::no_data(Check::Lightning, t)),
                Some(_) => {}
            }
        }
    }

    /// Wind observed at a station, for hours up to now.
    fn observed_wind(&self, t: UnixTime) -> Vec<ObservedWind> {
        let Some(series) = self.snap.marine.observed_wind.as_ref() else {
            return Vec::new();
        };
        if t > self.now {
            return Vec::new();
        }
        let source = series.provenance().source.clone();
        for_hour(series, t, HOUR)
            .iter()
            .map(|s| ObservedWind {
                value: s.value,
                source: source.clone(),
                at: s.time,
            })
            .collect()
    }

    /// The checks that rest on the sea and the river, for the hour at `t`.
    fn water_checks(&self, t: UnixTime, out: &mut Vec<Reason>) {
        if let Some(limit) = self.canon.wave_m {
            push_max(out, Check::Waves, self.wave_height(t), limit, t);
        }
        if let Some(limit) = self.canon.current_ms {
            push_max(out, Check::Current, self.current(t), limit, t);
        }
        if let Some(limit) = self.canon.flow_m3s {
            push_max(out, Check::Flow, self.flow(t), limit, t);
        }
    }

    fn wave_height(&self, t: UnixTime) -> Option<Reading> {
        let marine = &self.snap.marine;
        let height = |series: &Series<WaveSample>, reach: i64| {
            let source = series.provenance().source.clone();
            for_hour(series, t, reach)
                .iter()
                .filter_map(|s| {
                    s.value.significant_height_m.map(|h| Reading {
                        value: f64::from(h),
                        source: source.clone(),
                        at: s.time,
                    })
                })
                .collect::<Vec<_>>()
        };
        let observed = marine
            .waves
            .as_ref()
            .filter(|_| t <= self.now)
            .map(|s| height(s, HOUR))
            .unwrap_or_default();
        let forecast = marine
            .waves_forecast
            .as_ref()
            .map(|s| height(s, MAX_FORECAST_GAP))
            .unwrap_or_default();
        greatest(observed.into_iter().chain(forecast))
    }

    fn current(&self, t: UnixTime) -> Option<Reading> {
        let series = self.snap.marine.tidal_current.as_ref()?.curve.as_ref()?;
        let source = series.provenance().source.clone();
        let mut values: Vec<f64> = in_hour(series, t)
            .iter()
            .map(|s| f64::from(s.value.abs()))
            .collect();
        if let Some(mid) = series.at(UnixTime(t.0 + HOUR / 2), std::time::Duration::from_hours(2)) {
            values.push(f64::from(mid.abs()));
        }
        values.into_iter().reduce(f64::max).map(|value| Reading {
            value,
            source,
            at: t,
        })
    }

    fn flow(&self, t: UnixTime) -> Option<Reading> {
        let water = &self.snap.water;
        let gap = std::time::Duration::from_secs(MAX_FORECAST_GAP.unsigned_abs());
        let mid = UnixTime(t.0 + HOUR / 2);
        let observed = water
            .discharge_m3s
            .as_ref()
            .filter(|_| t <= self.now)
            .and_then(|s| s.at(mid, gap).map(|v| (v, s)));
        let forecast = water
            .discharge_forecast_m3s
            .as_ref()
            .and_then(|s| s.at(mid, gap).map(|v| (v, s)));
        greatest(
            [observed, forecast]
                .into_iter()
                .flatten()
                .map(|(v, s)| Reading {
                    value: f64::from(v),
                    source: s.provenance().source.clone(),
                    at: t,
                }),
        )
    }

    /// Marine alerts in effect during the hour at `t`.
    fn alert_checks(&self, t: UnixTime, out: &mut Vec<Reason>) {
        let marine = &self.snap.marine;
        let hour_end = t.0 + HOUR;
        for alert in &marine.marine_alerts {
            let class = alert_class(&alert.event);
            let counts = match class {
                Some(AlertClass::SmallCraft) => self.canon.no_sca,
                Some(AlertClass::Warning) => true,
                None => false,
            };
            if !counts {
                continue;
            }
            let (start, end) = alert_span(alert);
            if start.0 < hour_end && end.0 > t.0 {
                out.push(Reason {
                    at: alert.onset.or(Some(t)),
                    until: alert.ends.or(alert.expires),
                    source: Some(SourceId::NWS),
                    subject: Some(alert.event.clone()),
                    ..Reason::new(
                        if class == Some(AlertClass::SmallCraft) {
                            Check::SmallCraftAdvisory
                        } else {
                            Check::MarineWarning
                        },
                        ReasonKind::Fails,
                    )
                });
            }
        }
        let alerts_failed = self
            .snap
            .errors
            .iter()
            .any(|e| e.measurement == Some(Measurement::MarineAlerts));
        if self.canon.no_sca && alerts_failed && marine.marine_alerts.is_empty() {
            out.push(Reason::no_data(Check::SmallCraftAdvisory, t));
        }
    }
}

struct ObservedWind {
    value: Wind,
    source: SourceId,
    at: UnixTime,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AlertClass {
    SmallCraft,
    Warning,
}

/// NWS event names this check knows; others are not judged.
fn alert_class(event: &str) -> Option<AlertClass> {
    const WARNINGS: [&str; 6] = [
        "Gale Warning",
        "Storm Warning",
        "Hurricane Force Wind Warning",
        "Hazardous Seas Warning",
        "Special Marine Warning",
        "Hurricane Warning",
    ];
    if event.starts_with("Small Craft Advisory") {
        Some(AlertClass::SmallCraft)
    } else if WARNINGS.contains(&event) {
        Some(AlertClass::Warning)
    } else {
        None
    }
}

/// The span an alert covers: `onset` to `ends`, or to `expires` when no end is given; open on a
/// side where the alert gives no time.
fn alert_span(alert: &MarineAlert) -> (UnixTime, UnixTime) {
    (
        alert.onset.unwrap_or(UnixTime(i64::MIN)),
        alert.ends.or(alert.expires).unwrap_or(UnixTime(i64::MAX)),
    )
}

fn push_max(
    out: &mut Vec<Reason>,
    check: Check,
    reading: Option<Reading>,
    limit: f64,
    t: UnixTime,
) {
    match reading {
        Some(r) => out.extend(judge_max(check, &r, limit)),
        None => out.push(Reason::no_data(check, t)),
    }
}

/// Adds a finding, or no data when the value was absent.
fn push(out: &mut Vec<Reason>, check: Check, reason: Option<Reason>, t: UnixTime, present: bool) {
    if present {
        out.extend(reason);
    } else {
        out.push(Reason::no_data(check, t));
    }
}

fn verdict_of(reasons: &[Reason]) -> Verdict {
    let has = |k: ReasonKind| reasons.iter().any(|r| r.kind == k);
    if has(ReasonKind::Fails) {
        Verdict::NoGo
    } else if has(ReasonKind::NoData) || has(ReasonKind::OutsideForecast) || has(ReasonKind::Past) {
        Verdict::Unknown
    } else if has(ReasonKind::Marginal) {
        Verdict::Marginal
    } else {
        Verdict::Go
    }
}

/// The storm and hurricane findings for the span `[from, to)`.
fn storm_reasons(snap: &SpotSnapshot, now: UnixTime, from: i64, to: i64) -> Vec<Reason> {
    let mut out = Vec::new();
    let Some(watch) = snap.storm.as_ref() else {
        return out;
    };
    if let Some(outlook) = watch.point.as_ref() {
        for event in analysis::all_events(outlook) {
            if event.end > now
                && analysis::is_significant(&event)
                && event.start.0 < to
                && event.end.0 > from
            {
                out.push(Reason {
                    at: Some(event.start),
                    until: Some(event.end),
                    source: Some(SourceId::new("open-meteo")),
                    ..Reason::new(Check::StormAhead, ReasonKind::Fails)
                });
            }
        }
    }
    let place = (snap.spot.latitude, snap.spot.longitude);
    for h in &watch.hurricanes {
        let horizon = (to - now.0).max(0);
        if let Some((start, end)) = hurricane_window(h, place, now, horizon)
            && start.0 < to
            && end.0 > from
        {
            out.push(Reason {
                at: Some(start),
                until: Some(end),
                source: Some(SourceId::new("nhc")),
                subject: Some(h.name.clone()),
                link: Some(
                    h.advisory_url
                        .clone()
                        .unwrap_or_else(|| NHC_LINK.to_owned()),
                ),
                ..Reason::new(Check::Hurricane, ReasonKind::Fails)
            });
        }
    }
    out
}

/// Judges one local day of a trip against the trip's limits.
///
/// Each local hour is judged on its own; the day is the worst of its hours that are not over yet
/// (at `now`), plus the storm checks for the whole day. A day that is over is `Unknown` with a
/// [`ReasonKind::Past`] reason. `limits` are the trip's own, in canonical units; the core's copy
/// of the configuration's limits profile.
pub fn assess(
    day: LocalDate,
    inputs: &DayInputs<'_>,
    limits: &TripLimits,
    now: UnixTime,
) -> DayVerdict {
    let snap = inputs.snapshot;
    let unknown = |reason: Reason, hours: Vec<HourVerdict>| DayVerdict {
        day,
        verdict: Verdict::Unknown,
        reasons: vec![reason],
        hours,
    };
    let bounds = time::zone(&snap.spot.time_zone).and_then(|tz| time::local_date_bounds(day, &tz));
    let Ok((start, end)) = bounds else {
        return unknown(Reason::new(Check::Forecast, ReasonKind::NoData), Vec::new());
    };
    let hourly = snap.weather.as_ref().map(|w| w.hourly.as_slice());
    let ctx = Ctx {
        snap,
        canon: Canon::new(limits, &inputs.river),
        now,
        horizon: hourly.and_then(horizon),
        has_weather: hourly.is_some_and(|h| !h.is_empty()),
    };
    let storms = storm_reasons(snap, now, start.0, end.0);

    let mut hours = Vec::new();
    let mut t = start.0;
    while t < end.0 {
        let at = UnixTime(t);
        let mut reasons = Vec::new();
        match ctx.horizon {
            Some(h) if ctx.has_weather && at >= h => {
                reasons.push(Reason {
                    at: Some(at),
                    ..Reason::new(Check::Forecast, ReasonKind::OutsideForecast)
                });
            }
            _ if !ctx.has_weather => reasons.push(Reason::no_data(Check::Forecast, at)),
            _ => ctx.weather_checks(at, &mut reasons),
        }
        ctx.water_checks(at, &mut reasons);
        ctx.alert_checks(at, &mut reasons);
        reasons.extend(
            storms
                .iter()
                .filter(|r| {
                    r.at.is_some_and(|s| s.0 < t + HOUR) && r.until.is_some_and(|e| e.0 > t)
                })
                .cloned(),
        );
        hours.push(HourVerdict {
            start: at,
            verdict: verdict_of(&reasons),
            reasons,
        });
        t += HOUR;
    }

    if end <= now {
        return unknown(
            Reason {
                at: Some(start),
                ..Reason::new(Check::Forecast, ReasonKind::Past)
            },
            hours,
        );
    }

    let live: Vec<&HourVerdict> = hours.iter().filter(|h| h.start.0 + HOUR > now.0).collect();
    let mut worst: BTreeMap<(Check, ReasonKind, Option<String>), Reason> = BTreeMap::new();
    let everything = live
        .iter()
        .flat_map(|h| h.reasons.iter())
        .chain(storms.iter());
    for reason in everything {
        let key = (reason.check, reason.kind, reason.subject.clone());
        match worst.get(&key) {
            Some(kept) if kept.excess() >= reason.excess() => {}
            _ => {
                worst.insert(key, reason.clone());
            }
        }
    }
    let mut reasons: Vec<Reason> = worst.into_values().collect();
    reasons.sort_by_key(|r| r.kind);
    DayVerdict {
        day,
        verdict: verdict_of(&reasons),
        reasons,
        hours,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::snapshot::testdata as sample;
    use crate::model::{AlertSeverity, StormWatch};
    use crate::provenance::{Provenance, Quality, Relation, ValueKind};
    use crate::series::{Sample, Series};
    use crate::units::DischargeUnit;
    use whether_core::builder::SnapshotBuilder;
    use whether_core::condition::Intensity;
    use whether_core::model::{ProviderId, WeatherSnapshot};
    use whether_core::storm::testdata as storm_data;
    use whether_core::units::{Celsius, Meters, Mps};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/raw/");

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    fn fixture(path: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}{path}")).unwrap()
    }

    fn bounds(day: LocalDate) -> (i64, i64) {
        let tz = time::zone("America/New_York").unwrap();
        let (a, b) = time::local_date_bounds(day, &tz).unwrap();
        (a.0, b.0)
    }

    fn prov(source: SourceId) -> Provenance {
        Provenance {
            source,
            station: None,
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(0),
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        }
    }

    /// Calm, clear hourly weather for `hours` hours from `from`; `tweak` changes hour `i`.
    fn weather_from(
        from: i64,
        hours: usize,
        tweak: impl Fn(usize, &mut Hourly),
    ) -> WeatherSnapshot {
        let mut b = SnapshotBuilder::new(ProviderId::OpenMeteo);
        b.set_location(whether_core::testdata::sample_location());
        b.set_hourly(
            (0..hours)
                .map(|i| {
                    let mut h = Hourly {
                        time: UnixTime(from + i64::try_from(i).unwrap() * HOUR),
                        condition: Condition::PartlyCloudy,
                        temperature: Some(Celsius(12.0)),
                        wind_speed: Some(Mps(3.0)),
                        wind_gust: Some(Mps(4.5)),
                        visibility: Some(Meters(20_000.0)),
                        ..Hourly::default()
                    };
                    tweak(i, &mut h);
                    h
                })
                .collect(),
        );
        b.finish(UnixTime(from)).unwrap()
    }

    /// A snapshot with calm weather for four days from 2026-11-08 local midnight.
    fn snapshot_with(tweak: impl Fn(usize, &mut Hourly)) -> SpotSnapshot {
        let (start, _) = bounds(date(2026, 11, 8));
        let mut s = SpotSnapshot::new(sample::spot(), UnixTime(start));
        s.weather = Some(weather_from(start, 96, tweak));
        s
    }

    fn calm() -> SpotSnapshot {
        snapshot_with(|_, _| {})
    }

    fn before_day() -> UnixTime {
        UnixTime(bounds(date(2026, 11, 8)).0 - HOUR)
    }

    fn kayak() -> TripLimits {
        TripLimits {
            name: "Kayak".into(),
            max_wind_kn: Some(12.0),
            max_gust_kn: Some(18.0),
            max_wave_m: Some(0.6),
            no_small_craft_advisory: true,
            no_lightning: true,
            ..TripLimits::default()
        }
    }

    fn wading() -> TripLimits {
        TripLimits {
            name: "Wading".into(),
            no_lightning: true,
            ..TripLimits::default()
        }
    }

    fn assess_day(
        snap: &SpotSnapshot,
        day: LocalDate,
        limits: &TripLimits,
        now: UnixTime,
    ) -> DayVerdict {
        assess(day, &DayInputs::new(snap), limits, now)
    }

    fn kinds(reasons: &[Reason], check: Check) -> Vec<ReasonKind> {
        reasons
            .iter()
            .filter(|r| r.check == check)
            .map(|r| r.kind)
            .collect()
    }

    fn rfc3339(s: &str) -> UnixTime {
        time::parse_rfc3339(s).unwrap()
    }

    /// The marine alerts of the recorded active-alerts response.
    fn recorded_alerts() -> Vec<MarineAlert> {
        let v: serde_json::Value =
            serde_json::from_str(&fixture("nws-marine/alerts-active-zone-ANZ338.geo.json"))
                .unwrap();
        v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let p = &f["properties"];
                let at = |k: &str| p[k].as_str().map(rfc3339);
                MarineAlert {
                    id: p["id"].as_str().unwrap().into(),
                    event: p["event"].as_str().unwrap().into(),
                    headline: None,
                    description: None,
                    instruction: None,
                    onset: at("onset"),
                    ends: at("ends"),
                    expires: at("expires"),
                    severity: AlertSeverity::Minor,
                    zones: Vec::new(),
                    vtec_key: None,
                }
            })
            .collect()
    }

    fn alert(event: &str, onset: &str, ends: &str) -> MarineAlert {
        MarineAlert {
            id: format!("test-{event}"),
            event: event.into(),
            headline: None,
            description: None,
            instruction: None,
            onset: Some(rfc3339(onset)),
            ends: Some(rfc3339(ends)),
            expires: None,
            severity: AlertSeverity::Severe,
            zones: Vec::new(),
            vtec_key: None,
        }
    }

    fn hour_at<'a>(day: &'a DayVerdict, utc: &str) -> &'a HourVerdict {
        let t = rfc3339(utc);
        day.hours.iter().find(|h| h.start == t).unwrap()
    }

    #[test]
    fn small_craft_advisory_is_no_go_between_onset_and_ends() {
        let mut snap = calm();
        snap.marine.marine_alerts = recorded_alerts();
        let alerts = snap.marine.marine_alerts.clone();
        assert_eq!(alerts[0].event, "Small Craft Advisory");
        let (onset, ends) = (alerts[0].onset.unwrap(), alerts[0].ends.unwrap());
        assert_eq!(onset, rfc3339("2026-11-08T06:48:00Z"));
        assert_eq!(ends, rfc3339("2026-11-09T23:00:00Z"));

        // The first day: go before the onset hour, no-go from it.
        let day = assess_day(&snap, date(2026, 11, 8), &kayak(), before_day());
        assert_eq!(
            hour_at(&day, "2026-11-08T05:00:00Z").verdict,
            Verdict::Unknown
        );
        let reason = day
            .reasons
            .iter()
            .find(|r| r.check == Check::SmallCraftAdvisory)
            .unwrap();
        assert_eq!(hour_at(&day, "2026-11-08T06:00:00Z").verdict, Verdict::NoGo);
        assert_eq!(day.verdict, Verdict::NoGo);
        assert_eq!(reason.kind, ReasonKind::Fails);
        assert_eq!(reason.subject.as_deref(), Some("Small Craft Advisory"));
        assert_eq!((reason.at, reason.until), (Some(onset), Some(ends)));
        assert_eq!(reason.source, Some(SourceId::NWS));
        // The 05:00 hour is only unknown because the kayak's wave limit has no data.
        let first = hour_at(&day, "2026-11-08T05:00:00Z");
        assert!(first.reasons.iter().all(|r| r.check == Check::Waves));
    }

    #[test]
    fn the_advisory_covers_exactly_its_hours() {
        let mut snap = calm();
        snap.marine.marine_alerts = recorded_alerts();
        let limits = TripLimits {
            no_small_craft_advisory: true,
            ..TripLimits::default()
        };
        let day = assess_day(&snap, date(2026, 11, 8), &limits, before_day());
        assert_eq!(hour_at(&day, "2026-11-08T05:00:00Z").verdict, Verdict::Go);
        assert_eq!(hour_at(&day, "2026-11-08T06:00:00Z").verdict, Verdict::NoGo);

        // The last covered hour is 22:00 UTC on the 9th; the next one is clear.
        let last = assess_day(&snap, date(2026, 11, 9), &limits, before_day());
        assert_eq!(
            hour_at(&last, "2026-11-09T22:00:00Z").verdict,
            Verdict::NoGo
        );
        assert_eq!(hour_at(&last, "2026-11-09T23:00:00Z").verdict, Verdict::Go);

        // The day after the advisory is clear.
        let after = assess_day(&snap, date(2026, 11, 10), &limits, before_day());
        assert_eq!(after.verdict, Verdict::Go);
    }

    #[test]
    fn advisory_is_ignored_when_the_profile_does_not_ask() {
        let mut snap = calm();
        snap.marine.marine_alerts = recorded_alerts();
        let day = assess_day(&snap, date(2026, 11, 8), &wading(), before_day());
        assert_eq!(day.verdict, Verdict::Go);
        assert_eq!(day.reasons.len(), 0);
    }

    #[test]
    fn gale_class_warnings_are_no_go_for_every_profile() {
        let mut snap = calm();
        snap.marine.marine_alerts = vec![alert(
            "Gale Warning",
            "2026-11-08T12:00:00Z",
            "2026-11-08T18:00:00Z",
        )];
        let day = assess_day(&snap, date(2026, 11, 8), &wading(), before_day());
        assert_eq!(day.verdict, Verdict::NoGo);
        assert_eq!(hour_at(&day, "2026-11-08T11:00:00Z").verdict, Verdict::Go);
        assert_eq!(hour_at(&day, "2026-11-08T12:00:00Z").verdict, Verdict::NoGo);
        assert_eq!(hour_at(&day, "2026-11-08T17:00:00Z").verdict, Verdict::NoGo);
        assert_eq!(hour_at(&day, "2026-11-08T18:00:00Z").verdict, Verdict::Go);
        assert_eq!(
            kinds(&day.reasons, Check::MarineWarning),
            [ReasonKind::Fails]
        );
    }

    #[test]
    fn a_failed_alerts_fetch_is_unknown_when_advisories_matter() {
        use crate::model::{SourceError, SourceErrorKind};
        let mut snap = calm();
        snap.errors.push(SourceError {
            source: SourceId::NWS,
            measurement: Some(Measurement::MarineAlerts),
            kind: SourceErrorKind::Transport,
            message: "unreachable".into(),
        });
        let limits = TripLimits {
            no_small_craft_advisory: true,
            ..TripLimits::default()
        };
        let day = assess_day(&snap, date(2026, 11, 8), &limits, before_day());
        assert_eq!(day.verdict, Verdict::Unknown);
        assert_eq!(
            kinds(&day.reasons, Check::SmallCraftAdvisory),
            [ReasonKind::NoData]
        );
        let plain = assess_day(&snap, date(2026, 11, 8), &wading(), before_day());
        assert_eq!(plain.verdict, Verdict::Go);
    }

    fn ndbc_waves() -> Series<WaveSample> {
        let text = fixture("ndbc/ndbc-44025.txt");
        let samples = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let cols: Vec<&str> = l.split_whitespace().collect();
                let height: f32 = cols.get(8)?.parse().ok()?;
                let t = time::parse_ndbc(&cols[..5].join(" ")).ok()?;
                Some(Sample::new(
                    t,
                    WaveSample {
                        significant_height_m: Some(height),
                        ..WaveSample::default()
                    },
                ))
            })
            .collect();
        Series::new(prov(SourceId::NDBC), samples)
    }

    fn open_meteo_waves() -> Series<WaveSample> {
        let v: serde_json::Value = serde_json::from_str(&fixture(
            "open-meteo-marine/marine-40.25_-73.16-hourly-2d.json",
        ))
        .unwrap();
        assert_eq!(v["utc_offset_seconds"], 0);
        let times = v["hourly"]["time"].as_array().unwrap();
        let heights = v["hourly"]["wave_height"].as_array().unwrap();
        let samples = times
            .iter()
            .zip(heights)
            .map(|(t, h)| {
                Sample::new(
                    time::parse_local_iso(t.as_str().unwrap(), time::UtcOffset(0)).unwrap(),
                    WaveSample {
                        significant_height_m: Some(h.as_f64().unwrap() as f32),
                        ..WaveSample::default()
                    },
                )
            })
            .collect();
        Series::new(prov(SourceId::OPEN_METEO_MARINE), samples)
    }

    fn waves_limit(m: f64) -> TripLimits {
        TripLimits {
            max_wave_m: Some(m),
            ..TripLimits::default()
        }
    }

    #[test]
    fn buoy_waves_decide_the_hours_up_to_now() {
        let mut snap = calm();
        snap.marine.waves = Some(ndbc_waves());
        let now = rfc3339("2026-11-08T14:00:00Z");
        // 13:10 reads 1.9 m and 13:40 reads 1.8 m: the hour's worst is used.
        let day = assess_day(&snap, date(2026, 11, 8), &waves_limit(0.6), now);
        let hour = hour_at(&day, "2026-11-08T13:00:00Z");
        assert_eq!(hour.verdict, Verdict::NoGo);
        let r = &hour.reasons[0];
        assert_eq!((r.check, r.kind), (Check::Waves, ReasonKind::Fails));
        assert!((r.measured.unwrap() - 1.9).abs() < 1e-6, "{:?}", r.measured);
        assert_eq!(r.limit, Some(0.6));
        assert_eq!(r.source, Some(SourceId::NDBC));

        let marginal = assess_day(&snap, date(2026, 11, 8), &waves_limit(2.0), now);
        assert_eq!(
            hour_at(&marginal, "2026-11-08T13:00:00Z").verdict,
            Verdict::Marginal
        );
        let fine = assess_day(&snap, date(2026, 11, 8), &waves_limit(5.0), now);
        assert_eq!(hour_at(&fine, "2026-11-08T13:00:00Z").verdict, Verdict::Go);

        // A buoy reading is no forecast: later hours have no data.
        assert_eq!(
            hour_at(&fine, "2026-11-08T18:00:00Z").verdict,
            Verdict::Unknown
        );
    }

    #[test]
    fn modelled_waves_decide_the_hours_ahead() {
        let mut snap = calm();
        snap.marine.waves_forecast = Some(open_meteo_waves());
        let day = assess_day(&snap, date(2026, 11, 8), &waves_limit(1.0), before_day());
        let hour = hour_at(&day, "2026-11-08T05:00:00Z");
        assert_eq!(hour.verdict, Verdict::NoGo);
        let r = &hour.reasons[0];
        assert_eq!(r.source, Some(SourceId::OPEN_METEO_MARINE));
        assert!(
            (r.measured.unwrap() - 1.74).abs() < 0.05,
            "{:?}",
            r.measured
        );
        // The fixture ends after two days: the third has no waves.
        let later = assess_day(&snap, date(2026, 11, 10), &waves_limit(1.0), before_day());
        assert_eq!(later.verdict, Verdict::Unknown);
        assert_eq!(kinds(&later.reasons, Check::Waves), [ReasonKind::NoData]);
    }

    fn croton_flow() -> Series<f32> {
        let v: serde_json::Value = serde_json::from_str(&fixture(
            "usgs/usgs-continuous-croton-01375000-00060-2026-11-01_2026-11-08.json",
        ))
        .unwrap();
        let samples = v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let p = &f["properties"];
                let cfs: f32 = p["value"].as_str().unwrap().parse().unwrap();
                Sample::new(
                    rfc3339(p["time"].as_str().unwrap()),
                    DischargeUnit::Cfs.to_canonical(cfs),
                )
            })
            .collect();
        Series::new(prov(SourceId::USGS), samples)
    }

    #[test]
    fn flow_above_the_wadeable_range_fails() {
        let mut snap = calm();
        let flow = croton_flow();
        let day = date(2026, 11, 2);
        let (start, _) = bounds(day);
        let measured = f64::from(
            flow.at(
                UnixTime(start + HOUR / 2),
                std::time::Duration::from_secs(3_600),
            )
            .unwrap(),
        );
        snap.water.discharge_m3s = Some(flow);
        let now = UnixTime(start + 30 * HOUR);
        let first_hour = |wadeable: f64| {
            let ranges = RiverRanges {
                wadeable_below_m3s: Some(wadeable as f32),
                ..RiverRanges::default()
            };
            assess(
                day,
                &DayInputs::new(&snap).with_river(ranges),
                &TripLimits::default(),
                now,
            )
            .hours
            .remove(0)
        };

        let fails = first_hour(measured * 0.9);
        assert_eq!(fails.verdict, Verdict::NoGo);
        let r = &fails.reasons[0];
        assert_eq!((r.check, r.kind), (Check::Flow, ReasonKind::Fails));
        assert!((r.measured.unwrap() - measured).abs() < 1e-3);
        assert_eq!(r.source, Some(SourceId::USGS));
        // Within 10 %: marginal. Comfortably below the limit: go.
        assert_eq!(first_hour(measured * 1.05).verdict, Verdict::Marginal);
        assert_eq!(first_hour(measured * 3.0).verdict, Verdict::Go);
    }

    #[test]
    fn blown_out_flow_counts_when_no_wadeable_limit_is_set() {
        let mut snap = calm();
        let (start, _) = bounds(date(2026, 11, 2));
        snap.water.discharge_m3s = Some(croton_flow());
        let ranges = RiverRanges {
            blown_out_above_m3s: Some(0.1),
            ..RiverRanges::default()
        };
        let day = assess(
            date(2026, 11, 2),
            &DayInputs::new(&snap).with_river(ranges),
            &TripLimits::default(),
            UnixTime(start + 30 * HOUR),
        );
        assert_eq!(day.hours[0].verdict, Verdict::NoGo);
    }

    #[test]
    fn wind_gusts_visibility_and_cold_come_from_the_forecast() {
        let snap = snapshot_with(|i, h| match i {
            // 6.0 m/s is 11.7 kn: within 10 % of 12 kn.
            2 => h.wind_speed = Some(Mps(6.0)),
            // 11 m/s is about 21 kn, past the 18 kn gust limit.
            3 => h.wind_gust = Some(Mps(11.0)),
            4 => h.visibility = Some(Meters(3_200.0)),
            5 => h.visibility = Some(Meters(1_500.0)),
            6 => h.temperature = Some(Celsius(-2.0)),
            7 => h.temperature = Some(Celsius(-0.5)),
            _ => {}
        });
        let limits = TripLimits {
            max_wind_kn: Some(12.0),
            max_gust_kn: Some(18.0),
            min_visibility_km: Some(3.0),
            min_air_temp_c: Some(-1.0),
            ..TripLimits::default()
        };
        let day = assess_day(&snap, date(2026, 11, 8), &limits, before_day());
        let verdicts: Vec<Verdict> = day.hours[..9].iter().map(|h| h.verdict).collect();
        assert_eq!(
            verdicts,
            [
                Verdict::Go,
                Verdict::Go,
                Verdict::Marginal,
                Verdict::NoGo,
                Verdict::Marginal,
                Verdict::NoGo,
                Verdict::NoGo,
                Verdict::Marginal,
                Verdict::Go,
            ]
        );
        let gust = &day.hours[3].reasons[0];
        assert_eq!(gust.check, Check::Gust);
        assert!((gust.limit.unwrap() - 18.0 * 0.514_444).abs() < 1e-3);
        assert_eq!(gust.measured, Some(11.0));
        assert_eq!(gust.source, Some(SourceId::new("open-meteo")));
        assert_eq!(day.hours[5].reasons[0].check, Check::Visibility);
        assert_eq!(day.hours[6].reasons[0].check, Check::AirTemperature);
        // The day keeps one reason per check and outcome.
        let worst: Vec<_> = day.reasons.iter().map(|r| (r.check, r.kind)).collect();
        assert_eq!(
            worst,
            [
                (Check::Gust, ReasonKind::Fails),
                (Check::AirTemperature, ReasonKind::Fails),
                (Check::Visibility, ReasonKind::Fails),
                (Check::Wind, ReasonKind::Marginal),
                (Check::AirTemperature, ReasonKind::Marginal),
                (Check::Visibility, ReasonKind::Marginal),
            ]
        );
        // Of the two cold failures the colder hour is kept.
        let cold = day
            .reasons
            .iter()
            .find(|r| (r.check, r.kind) == (Check::AirTemperature, ReasonKind::Fails))
            .unwrap();
        assert_eq!(cold.measured, Some(-2.0));
    }

    #[test]
    fn thunder_is_a_no_go_only_when_the_limits_ask() {
        let snap = snapshot_with(|i, h| {
            if i == 10 {
                h.condition = Condition::Thunderstorm(Intensity::Light);
            }
            if i == 11 {
                h.condition = Condition::ThunderstormHail(Intensity::Heavy);
            }
        });
        let day = assess_day(&snap, date(2026, 11, 8), &wading(), before_day());
        assert_eq!(day.verdict, Verdict::NoGo);
        assert_eq!(day.hours[9].verdict, Verdict::Go);
        assert_eq!(day.hours[10].verdict, Verdict::NoGo);
        assert_eq!(day.hours[11].verdict, Verdict::NoGo);
        assert_eq!(day.hours[12].verdict, Verdict::Go);
        let r = &day.hours[10].reasons[0];
        assert_eq!((r.check, r.kind), (Check::Lightning, ReasonKind::Fails));

        let ok = assess_day(
            &snap,
            date(2026, 11, 8),
            &TripLimits::default(),
            before_day(),
        );
        assert_eq!(ok.verdict, Verdict::Go);
    }

    #[test]
    fn a_missing_value_is_unknown_never_go() {
        // No weather at all.
        let (start, _) = bounds(date(2026, 11, 8));
        let bare = SpotSnapshot::new(sample::spot(), UnixTime(start));
        let day = assess_day(&bare, date(2026, 11, 8), &wading(), before_day());
        assert_eq!(day.verdict, Verdict::Unknown);
        assert_eq!(kinds(&day.reasons, Check::Forecast), [ReasonKind::NoData]);

        // Weather without gusts, with a gust limit.
        let snap = snapshot_with(|_, h| h.wind_gust = None);
        let day = assess_day(&snap, date(2026, 11, 8), &kayak(), before_day());
        assert_eq!(day.verdict, Verdict::Unknown);
        assert!(kinds(&day.reasons, Check::Gust).contains(&ReasonKind::NoData));
        // Waves are not known either.
        assert!(kinds(&day.reasons, Check::Waves).contains(&ReasonKind::NoData));

        // Unknown conditions do not clear lightning.
        let snap = snapshot_with(|_, h| h.condition = Condition::Unknown);
        let day = assess_day(&snap, date(2026, 11, 8), &wading(), before_day());
        assert_eq!(day.verdict, Verdict::Unknown);

        // A failure beats missing data.
        let snap = snapshot_with(|i, h| {
            if i == 4 {
                h.wind_speed = Some(Mps(15.0));
            }
        });
        let day = assess_day(&snap, date(2026, 11, 8), &kayak(), before_day());
        assert_eq!(day.verdict, Verdict::NoGo);
        assert_eq!(day.hours[4].verdict, Verdict::NoGo);
    }

    #[test]
    fn current_speed_is_judged_from_the_tidal_curve() {
        use crate::model::{CurrentSeries, MarineConditions};
        let (start, _) = bounds(date(2026, 11, 8));
        let mut snap = calm();
        let samples = (0..30)
            .map(|i| Sample::new(UnixTime(start + i * 1_800), if i < 8 { -1.2 } else { 0.1 }))
            .collect();
        snap.marine = MarineConditions {
            tidal_current: Some(CurrentSeries {
                events: Vec::new(),
                curve: Some(Series::new(prov(SourceId::COOPS), samples)),
                flood_dir_deg: None,
                ebb_dir_deg: None,
                bin: None,
                depth_m: None,
                observed: None,
                provenance: prov(SourceId::COOPS),
            }),
            ..MarineConditions::default()
        };
        let limits = TripLimits {
            max_current_kn: Some(1.5),
            ..TripLimits::default()
        };
        let day = assess_day(&snap, date(2026, 11, 8), &limits, before_day());
        // 1.2 m/s is 2.3 kn, against the ebb.
        assert_eq!(day.hours[0].verdict, Verdict::NoGo);
        assert_eq!(day.hours[0].reasons[0].check, Check::Current);
        assert_eq!(day.hours[8].verdict, Verdict::Go);
    }

    #[test]
    fn hours_past_the_forecast_are_unknown() {
        let snap = calm();
        // The forecast covers 96 hours from 2026-11-08 local midnight: four local days.
        let inside = assess_day(&snap, date(2026, 11, 11), &wading(), before_day());
        assert_eq!(inside.verdict, Verdict::Go);
        let outside = assess_day(&snap, date(2026, 11, 12), &wading(), before_day());
        assert_eq!(outside.verdict, Verdict::Unknown);
        assert_eq!(
            kinds(&outside.reasons, Check::Forecast),
            [ReasonKind::OutsideForecast]
        );
        assert!(outside.hours.iter().all(|h| h.verdict == Verdict::Unknown));
        // A far date is no different: history is never used.
        let far = assess_day(&snap, date(2027, 7, 12), &wading(), before_day());
        assert_eq!(far.verdict, Verdict::Unknown);
    }

    #[test]
    fn an_advisory_beyond_the_forecast_still_rules() {
        let mut snap = calm();
        snap.marine.marine_alerts = vec![alert(
            "Small Craft Advisory",
            "2026-11-12T12:00:00Z",
            "2026-11-12T18:00:00Z",
        )];
        let limits = TripLimits {
            no_small_craft_advisory: true,
            ..TripLimits::default()
        };
        let day = assess_day(&snap, date(2026, 11, 12), &limits, before_day());
        assert_eq!(day.verdict, Verdict::NoGo);
    }

    #[test]
    fn a_day_that_is_over_is_unknown_and_hours_already_over_do_not_count() {
        let mut snap = calm();
        snap.marine.marine_alerts = vec![alert(
            "Gale Warning",
            "2026-11-08T06:00:00Z",
            "2026-11-08T09:00:00Z",
        )];
        let limits = wading();
        let noon = rfc3339("2026-11-08T17:00:00Z");
        let today = assess_day(&snap, date(2026, 11, 8), &limits, noon);
        // The gale is over: the day is judged on what is left.
        assert_eq!(today.verdict, Verdict::Go);
        assert_eq!(
            hour_at(&today, "2026-11-08T07:00:00Z").verdict,
            Verdict::NoGo
        );

        let over = assess_day(
            &snap,
            date(2026, 11, 8),
            &limits,
            rfc3339("2026-11-09T10:00:00Z"),
        );
        assert_eq!(over.verdict, Verdict::Unknown);
        assert_eq!(kinds(&over.reasons, Check::Forecast), [ReasonKind::Past]);
    }

    #[test]
    fn a_daylight_saving_day_has_its_real_hours() {
        let (start, end) = bounds(date(2026, 11, 1));
        assert_eq!((end - start) / HOUR, 25);
        let snap = calm();
        let day = assess_day(&snap, date(2026, 11, 1), &wading(), UnixTime(start - HOUR));
        assert_eq!(day.hours.len(), 25);
        let spring = assess_day(&snap, date(2027, 3, 14), &wading(), UnixTime(start - HOUR));
        assert_eq!(spring.hours.len(), 23);
    }

    fn storm_snapshot(
        outlook: whether_core::storm::StormOutlook,
        hurricanes: Vec<whether_core::storm::Hurricane>,
    ) -> SpotSnapshot {
        let now = storm_data::STORM_NOW;
        let mut snap = SpotSnapshot::new(sample::spot(), now);
        snap.weather = Some(weather_from(now.0 - 24 * HOUR, 24 * 12, |_, _| {}));
        snap.storm = Some(StormWatch::from_outlook(now, outlook, hurricanes));
        snap
    }

    fn local_day(t: UnixTime) -> LocalDate {
        let tz = time::zone("America/New_York").unwrap();
        time::local_date(t, &tz).unwrap()
    }

    #[test]
    fn a_storm_ahead_is_no_go_for_every_profile() {
        let outlook = storm_data::noreaster();
        let event = analysis::all_events(&outlook)
            .into_iter()
            .find(analysis::is_significant)
            .unwrap();
        let snap = storm_snapshot(outlook, Vec::new());
        let now = storm_data::STORM_NOW;
        let mid = UnixTime(i64::midpoint(event.start.0, event.end.0));

        for limits in [TripLimits::default(), wading(), kayak()] {
            let day = assess_day(&snap, local_day(mid), &limits, now);
            assert_eq!(day.verdict, Verdict::NoGo, "{}", limits.name);
            let r = day
                .reasons
                .iter()
                .find(|r| r.check == Check::StormAhead)
                .unwrap();
            assert_eq!(r.kind, ReasonKind::Fails);
            assert_eq!((r.at, r.until), (Some(event.start), Some(event.end)));
        }
        // The hour in the middle of the storm says so.
        let day = assess_day(&snap, local_day(mid), &TripLimits::default(), now);
        let inside = day
            .hours
            .iter()
            .find(|h| h.start <= mid && mid.0 < h.start.0 + HOUR)
            .unwrap();
        assert_eq!(inside.verdict, Verdict::NoGo);

        // A day well after the storm is not touched.
        let after = local_day(UnixTime(event.end.0 + 2 * 24 * HOUR));
        let clear = assess_day(&snap, after, &TripLimits::default(), now);
        assert!(clear.reasons.iter().all(|r| r.check != Check::StormAhead));
    }

    #[test]
    fn a_hurricane_cone_over_the_spot_is_no_go_with_its_link() {
        let hurricane = storm_data::hurricane();
        let now = storm_data::STORM_NOW;
        let spot = sample::spot();
        let place = (spot.latitude, spot.longitude);
        let (from, to) = hurricane_window(&hurricane, place, now, 7 * 24 * HOUR).unwrap();
        let snap = storm_snapshot(storm_data::dry_week(), vec![hurricane.clone()]);
        let mid = UnixTime(i64::midpoint(from.0, to.0));
        let day = assess_day(&snap, local_day(mid), &TripLimits::default(), now);
        assert_eq!(day.verdict, Verdict::NoGo);
        let r = day
            .reasons
            .iter()
            .find(|r| r.check == Check::Hurricane)
            .unwrap();
        assert_eq!(r.subject.as_deref(), Some(hurricane.name.as_str()));
        assert_eq!(
            r.link.as_deref(),
            Some(hurricane.advisory_url.as_deref().unwrap_or(NHC_LINK))
        );
        assert_eq!(r.source, Some(SourceId::new("nhc")));

        // A day after the cone has left is not touched.
        let after = local_day(UnixTime(to.0 + 2 * 24 * HOUR));
        let clear = assess_day(&snap, after, &TripLimits::default(), now);
        assert!(clear.reasons.iter().all(|r| r.check != Check::Hurricane));
    }

    #[test]
    fn verdicts_round_trip_through_json() {
        let mut snap = calm();
        snap.marine.marine_alerts = recorded_alerts();
        let day = assess_day(&snap, date(2026, 11, 8), &kayak(), before_day());
        let json = serde_json::to_string(&day).unwrap();
        assert_eq!(serde_json::from_str::<DayVerdict>(&json).unwrap(), day);
        assert!(json.contains("\"day\":\"2026-11-08\""));
        assert!(json.contains("\"verdict\":\"no_go\""));
    }
}
