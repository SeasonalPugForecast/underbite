//! Fishing windows ("snack times"): the hours when the chosen factors line up.
//!
//! Seven factors are checked on a 15-minute grid: tide stage, current strength, low light, solunar
//! period, falling pressure, water temperature and river flow. A window is a run of grid steps at
//! which enough factors match; each window lists the factors that matched and the numbers behind
//! them. There is no score: the weights only decide whether a step qualifies and which windows are
//! kept when a day has more than four.
//!
//! The module returns data. Words ("incoming, 1 h 45 min before the 06:52 high") are the view's
//! job, and so are units: evidence is in canonical units (m/s, hPa, °C, m³/s).
//!
//! The configuration crate depends on this one, so the settings are [`WindowParams`]; the caller
//! builds them from its `[windows]` table.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::astro::moon::moon_events;
use crate::astro::solunar::{SolunarSettings, solunar_periods, sun_times};
use crate::derive::tide_stage::{TideDirection, tide_stage};
use crate::model::{
    CurrentSeries, SolunarCause, SolunarKind, SolunarPeriod, SpotSnapshot, TideKind, TideSeries,
    TideSourceKind,
};
use crate::series::{Sample, Series};
use crate::time::{self, LocalDate, UnixTime};

/// Seconds between evaluated instants.
pub const STEP_S: i64 = 900;
/// A run shorter than this is dropped, seconds.
pub const MIN_WINDOW_S: i64 = 1_800;
/// Runs separated by at most this many seconds merge.
pub const MERGE_GAP_S: i64 = 900;
/// Factors that must match at a step for it to count.
pub const MIN_FACTORS: usize = 2;
/// Summed weight of the matching factors a step needs.
pub const MIN_WEIGHT_SUM: f64 = 2.0;
/// Most windows kept for one local day.
pub const MAX_PER_DAY: usize = 4;
/// Longest span evaluated, in local days; more are ignored.
pub const MAX_DAYS: usize = 62;
/// Seconds in an hour: the margin of the low-light bands around sunrise and sunset.
const LIGHT_MARGIN_S: i64 = 3_600;
/// Hours the pressure change is taken over, and its threshold (whether's pressure trend).
const PRESSURE_HOURS: i64 = 3;
const PRESSURE_FALLING_HPA: f64 = -1.0;
/// Neighbouring pressure samples further apart than this are not interpolated across.
const PRESSURE_MAX_GAP: Duration = Duration::from_mins(90);
/// Water temperature and flow change slowly: the nearest sample within this many seconds stands
/// in where no sample brackets the instant.
const HOLD_S: i64 = 24 * 3_600;
/// Samples further apart than this are not interpolated across for water temperature and flow.
const LEVEL_MAX_GAP: Duration = Duration::from_hours(12);
/// Current curve samples further apart than this are not interpolated across.
const CURRENT_CURVE_GAP: Duration = Duration::from_mins(30);
/// Current events further apart than this are not interpolated between.
const CURRENT_EVENT_GAP_S: i64 = 8 * 3_600;
/// Metres per second in one knot.
const MS_PER_KNOT: f64 = 1_852.0 / 3_600.0;

/// A thing that can line up into a window.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Factor {
    TideStage,
    CurrentStrength,
    LowLight,
    Solunar,
    PressureFalling,
    WaterTemp,
    FlowInRange,
}

impl Factor {
    /// Every factor, in the order windows list them.
    pub const ALL: [Factor; 7] = [
        Factor::TideStage,
        Factor::CurrentStrength,
        Factor::LowLight,
        Factor::Solunar,
        Factor::PressureFalling,
        Factor::WaterTemp,
        Factor::FlowInRange,
    ];

    fn index(self) -> usize {
        Factor::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// How much each factor counts; zero (or less) turns it off.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FactorWeights {
    pub tide_stage: f64,
    pub current_strength: f64,
    pub low_light: f64,
    pub solunar: f64,
    pub pressure_falling: f64,
    pub water_temp: f64,
    pub flow_in_range: f64,
}

impl Default for FactorWeights {
    fn default() -> Self {
        Self {
            tide_stage: 1.0,
            current_strength: 1.0,
            low_light: 1.0,
            solunar: 0.5,
            pressure_falling: 0.7,
            water_temp: 1.0,
            flow_in_range: 1.0,
        }
    }
}

impl FactorWeights {
    /// The weight of one factor.
    pub fn get(&self, factor: Factor) -> f64 {
        match factor {
            Factor::TideStage => self.tide_stage,
            Factor::CurrentStrength => self.current_strength,
            Factor::LowLight => self.low_light,
            Factor::Solunar => self.solunar,
            Factor::PressureFalling => self.pressure_falling,
            Factor::WaterTemp => self.water_temp,
            Factor::FlowInRange => self.flow_in_range,
        }
    }

    /// Whether the factor takes part: a positive, finite weight.
    pub fn enabled(&self, factor: Factor) -> bool {
        let w = self.get(factor);
        w.is_finite() && w > 0.0
    }
}

/// Which way the tide moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TideFlow {
    /// Between a low and the next high.
    Incoming,
    /// Between a high and the next low.
    Outgoing,
}

/// The part of the tide that counts: the last `last_hours` of the incoming or outgoing tide.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TideRule {
    pub flow: TideFlow,
    pub last_hours: f64,
}

impl Default for TideRule {
    /// The last two hours of the flood.
    fn default() -> Self {
        Self {
            flow: TideFlow::Incoming,
            last_hours: 2.0,
        }
    }
}

/// The settings windows are computed with: the `[windows]` table of the configuration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowParams {
    pub weights: FactorWeights,
    pub tide: TideRule,
    /// Tidal current speed range that counts, knots `[min, max]`.
    pub current_range_kn: [f64; 2],
    /// Whether solunar periods take part at all.
    pub show_solunar: bool,
    pub solunar_major_minutes: u32,
    pub solunar_minor_minutes: u32,
}

impl Default for WindowParams {
    fn default() -> Self {
        Self {
            weights: FactorWeights::default(),
            tide: TideRule::default(),
            current_range_kn: [0.5, 2.5],
            show_solunar: true,
            solunar_major_minutes: 120,
            solunar_minor_minutes: 60,
        }
    }
}

impl WindowParams {
    fn enabled(&self, factor: Factor) -> bool {
        self.weights.enabled(factor) && (factor != Factor::Solunar || self.show_solunar)
    }
}

/// The ranges the user and the species set: absent means the factor has no data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ranges {
    /// The species' preferred water temperature, °C `[min, max]`.
    pub water_temp_c: Option<[f64; 2]>,
    /// The spot's best river flow, m³/s `[min, max]`.
    pub flow_m3s: Option<[f64; 2]>,
}

/// What windows are computed from, for the local days `first` to `last` at one place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowInputs {
    pub latitude: f64,
    pub longitude: f64,
    /// IANA zone: days, "four per day" and the report are in it.
    pub time_zone: String,
    pub first: LocalDate,
    pub last: LocalDate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tide: Option<TideSeries>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentSeries>,
    /// Air pressure, hPa, observed then forecast, in time order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressure_hpa: Vec<Sample<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_temp_c: Option<Series<f32>>,
    /// River discharge, m³/s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discharge_m3s: Option<Series<f32>>,
    #[serde(default)]
    pub ranges: Ranges,
}

/// `observed` followed by the `forecast` samples after its last one, under the observed
/// provenance; either alone if the other is absent.
fn joined(observed: Option<&Series<f32>>, forecast: Option<&Series<f32>>) -> Option<Series<f32>> {
    match (observed, forecast) {
        (Some(o), Some(f)) => {
            let after = o.latest().map(|s| s.time);
            let mut samples: Vec<Sample<f32>> = o.samples().to_vec();
            samples.extend(
                f.samples()
                    .iter()
                    .filter(|s| after.is_none_or(|a| s.time > a))
                    .copied(),
            );
            Some(Series::new(o.provenance().clone(), samples))
        }
        (Some(s), None) | (None, Some(s)) => Some(s.clone()),
        (None, None) => None,
    }
}

impl WindowInputs {
    /// Inputs from a snapshot for the local days `first` to `last`.
    ///
    /// Pressure is the observed series followed by the forecast's hourly sea-level pressure;
    /// water temperature the water's or else the sea surface's, with the sea-surface forecast
    /// after it; discharge the observed series followed by the flow forecast.
    pub fn from_snapshot(
        snapshot: &SpotSnapshot,
        first: LocalDate,
        last: LocalDate,
        ranges: Ranges,
    ) -> Self {
        let mut pressure: Vec<Sample<f32>> = snapshot
            .marine
            .observed_pressure_hpa
            .as_ref()
            .map(|p| p.samples().to_vec())
            .unwrap_or_default();
        if let Some(weather) = &snapshot.weather {
            let after = pressure.last().map(|s| s.time);
            pressure.extend(
                weather
                    .hourly
                    .iter()
                    .filter(|h| after.is_none_or(|a| h.time > a))
                    .filter_map(|h| h.pressure_msl.map(|p| Sample::new(h.time, p.0))),
            );
        }
        pressure.sort_by_key(|s| s.time);
        pressure.dedup_by_key(|s| s.time);
        let water_temp = snapshot
            .water
            .temperature_c
            .as_ref()
            .or(snapshot.marine.sea_surface_temp_c.as_ref());
        let water_temp_forecast = if snapshot.water.temperature_c.is_some() {
            None
        } else {
            snapshot.marine.sea_surface_temp_forecast_c.as_ref()
        };
        Self {
            latitude: snapshot.spot.latitude,
            longitude: snapshot.spot.longitude,
            time_zone: snapshot.spot.time_zone.clone(),
            first,
            last,
            tide: snapshot.marine.tide.clone(),
            current: snapshot.marine.tidal_current.clone(),
            pressure_hpa: pressure,
            water_temp_c: joined(water_temp, water_temp_forecast),
            discharge_m3s: joined(
                snapshot.water.discharge_m3s.as_ref(),
                snapshot.water.discharge_forecast_m3s.as_ref(),
            ),
            ranges,
        }
    }
}

/// Which low-light band a step is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LightBand {
    /// Civil dawn to an hour after sunrise.
    Morning,
    /// An hour before sunset to civil dusk.
    Evening,
}

/// The numbers behind a matched factor, taken at the first step of the window where it matched.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    TideStage {
        flow: TideFlow,
        /// The turn the tide is heading for.
        next_kind: TideKind,
        next_at: UnixTime,
        /// Whole seconds until it.
        seconds_to_next: i64,
    },
    CurrentStrength {
        /// Speed along the major axis, m/s, never negative.
        speed_ms: f64,
        /// Setting toward the flood direction rather than the ebb.
        flooding: bool,
    },
    LowLight {
        band: LightBand,
        /// Sunrise for the morning band, sunset for the evening one.
        sun_event_at: UnixTime,
        /// Minutes from that event to the step, negative before it.
        minutes_from_event: i64,
    },
    Solunar {
        kind: SolunarKind,
        cause: SolunarCause,
        /// The moon event the period is centred on.
        centre: UnixTime,
    },
    PressureFalling {
        /// Change over the last three hours, hPa, negative.
        change_hpa: f64,
        hours: i64,
    },
    WaterTemp {
        celsius: f64,
        min_c: f64,
        max_c: f64,
    },
    FlowInRange {
        m3s: f64,
        min_m3s: f64,
        max_m3s: f64,
    },
}

/// A factor that matched in a window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FactorMatch {
    pub factor: Factor,
    pub weight: f64,
    pub evidence: Evidence,
}

/// A time range where the chosen factors line up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FishingWindow {
    /// The local day the window starts on.
    pub date: LocalDate,
    /// Start, UTC (the view shows it in the spot's zone).
    pub start: UnixTime,
    /// End, UTC.
    pub end: UnixTime,
    /// Every factor that matched at some step of the window, in [`Factor::ALL`] order, each with
    /// its evidence from the first step it matched.
    pub matched: Vec<FactorMatch>,
    /// The largest summed weight of the matching factors at any one step.
    pub weight_sum: f64,
    /// Enabled factors that had no data anywhere in the window.
    pub no_data: Vec<Factor>,
}

struct Band {
    kind: LightBand,
    start: i64,
    end: i64,
    event: i64,
}

fn bands(lat: f64, lon: f64, date: LocalDate) -> Vec<Band> {
    let s = sun_times(date, lat, lon);
    let mut out = Vec::new();
    if let Some(rise) = s.sunrise {
        out.push(Band {
            kind: LightBand::Morning,
            start: s.civil_dawn.unwrap_or(rise).0,
            end: rise.0 + LIGHT_MARGIN_S,
            event: rise.0,
        });
    }
    if let Some(set) = s.sunset {
        out.push(Band {
            kind: LightBand::Evening,
            start: set.0 - LIGHT_MARGIN_S,
            end: s.civil_dusk.unwrap_or(set).0,
            event: set.0,
        });
    }
    out
}

fn valid_range(r: [f64; 2]) -> Option<[f64; 2]> {
    (r[0].is_finite() && r[1].is_finite()).then(|| [r[0].min(r[1]), r[0].max(r[1])])
}

/// The value at `t` linearly interpolated between neighbouring samples no more than `max_gap_s`
/// apart; `None` outside the samples.
fn interpolate(samples: &[Sample<f32>], at: i64, max_gap_s: u64) -> Option<f64> {
    let idx = samples.partition_point(|s| s.time.0 <= at);
    let before = samples.get(idx.checked_sub(1)?)?;
    if before.time.0 == at {
        return Some(f64::from(before.value));
    }
    let after = samples.get(idx)?;
    let span = after.time.0 - before.time.0;
    if u64::try_from(span).ok()? > max_gap_s {
        return None;
    }
    let frac = (at - before.time.0) as f64 / span as f64;
    let (v0, v1) = (f64::from(before.value), f64::from(after.value));
    Some(v0 + (v1 - v0) * frac)
}

/// A slowly changing value at `t`: interpolated between its neighbours, or the nearest sample
/// within a day.
fn level_at(series: &Series<f32>, t: UnixTime) -> Option<f64> {
    if let Some(v) = series.at(t, LEVEL_MAX_GAP) {
        return Some(f64::from(v)).filter(|v| v.is_finite());
    }
    let samples = series.samples();
    let i = samples.partition_point(|s| s.time < t);
    [i.checked_sub(1), Some(i)]
        .into_iter()
        .flatten()
        .filter_map(|j| samples.get(j))
        .min_by_key(|s| (s.time.0 - t.0).abs())
        .filter(|s| (s.time.0 - t.0).abs() <= HOLD_S)
        .map(|s| f64::from(s.value))
        .filter(|v| v.is_finite())
}

/// Signed current speed along the major axis at `t`, m/s: the curve where there is one, else a
/// half-cosine between the neighbouring slack and maximum events.
fn current_at(current: &CurrentSeries, t: UnixTime) -> Option<f64> {
    if let Some(v) = current
        .curve
        .as_ref()
        .and_then(|c| c.at(t, CURRENT_CURVE_GAP))
    {
        return Some(f64::from(v));
    }
    let prev = current
        .events
        .iter()
        .filter(|e| e.time <= t)
        .max_by_key(|e| e.time)?;
    let next = current
        .events
        .iter()
        .filter(|e| e.time > t)
        .min_by_key(|e| e.time)?;
    let span = next.time.0 - prev.time.0;
    if span <= 0 || span > CURRENT_EVENT_GAP_S {
        return None;
    }
    let f = (t.0 - prev.time.0) as f64 / span as f64;
    let (a, b) = (f64::from(prev.speed_ms), f64::from(next.speed_ms));
    Some(a + (b - a) * (1.0 - (std::f64::consts::PI * f).cos()) / 2.0)
}

struct Step {
    t: i64,
    matches: Vec<FactorMatch>,
    available: [bool; 7],
}

struct Eval<'a> {
    inputs: &'a WindowInputs,
    params: &'a WindowParams,
    solunar: Vec<SolunarPeriod>,
    centres: Vec<i64>,
    bands: Vec<Band>,
    current_range_ms: Option<[f64; 2]>,
}

impl Eval<'_> {
    fn step(&self, t: i64) -> Step {
        let mut step = Step {
            t,
            matches: Vec::new(),
            available: [false; 7],
        };
        for factor in Factor::ALL {
            if !self.params.enabled(factor) {
                continue;
            }
            if let Some(evidence) = self.factor(factor, t, &mut step.available[factor.index()]) {
                step.matches.push(FactorMatch {
                    factor,
                    weight: self.params.weights.get(factor),
                    evidence,
                });
            }
        }
        step
    }

    fn factor(&self, factor: Factor, t: i64, available: &mut bool) -> Option<Evidence> {
        let at = UnixTime(t);
        match factor {
            Factor::TideStage => {
                let series = self.inputs.tide.as_ref()?;
                if series.source_kind == TideSourceKind::ModelMean {
                    return None;
                }
                let stage = tide_stage(series, at)?;
                *available = true;
                let rule = self.params.tide;
                let direction = match rule.flow {
                    TideFlow::Incoming => TideDirection::Rising,
                    TideFlow::Outgoing => TideDirection::Falling,
                };
                stage
                    .in_last_hours(direction, rule.last_hours)
                    .then_some(Evidence::TideStage {
                        flow: rule.flow,
                        next_kind: stage.next.kind,
                        next_at: stage.next.time,
                        seconds_to_next: stage.to_next_s,
                    })
            }
            Factor::CurrentStrength => {
                let [lo, hi] = self.current_range_ms?;
                let v = current_at(self.inputs.current.as_ref()?, at)?;
                *available = true;
                let speed = v.abs();
                (speed >= lo && speed <= hi).then_some(Evidence::CurrentStrength {
                    speed_ms: speed,
                    flooding: v >= 0.0,
                })
            }
            Factor::LowLight => {
                *available = true;
                self.bands
                    .iter()
                    .find(|b| t >= b.start && t < b.end)
                    .map(|b| Evidence::LowLight {
                        band: b.kind,
                        sun_event_at: UnixTime(b.event),
                        minutes_from_event: (t - b.event).div_euclid(60),
                    })
            }
            Factor::Solunar => {
                *available = true;
                let i = self
                    .solunar
                    .iter()
                    .position(|p| t >= p.start.0 && t < p.end.0)?;
                let p = self.solunar[i];
                Some(Evidence::Solunar {
                    kind: p.kind,
                    cause: p.cause,
                    centre: UnixTime(self.centres[i]),
                })
            }
            Factor::PressureFalling => {
                let series = &self.inputs.pressure_hpa;
                let now = interpolate(series, t, PRESSURE_MAX_GAP.as_secs())?;
                let before = interpolate(
                    series,
                    t - PRESSURE_HOURS * 3_600,
                    PRESSURE_MAX_GAP.as_secs(),
                )?;
                *available = true;
                // Rounded to 0.01 hPa so a drop of exactly 1.0 hPa is not lost to float error.
                let change = ((now - before) * 100.0).round() / 100.0;
                (change <= PRESSURE_FALLING_HPA).then_some(Evidence::PressureFalling {
                    change_hpa: change,
                    hours: PRESSURE_HOURS,
                })
            }
            Factor::WaterTemp => {
                let [lo, hi] = valid_range(self.inputs.ranges.water_temp_c?)?;
                let v = level_at(self.inputs.water_temp_c.as_ref()?, at)?;
                *available = true;
                (v >= lo && v <= hi).then_some(Evidence::WaterTemp {
                    celsius: v,
                    min_c: lo,
                    max_c: hi,
                })
            }
            Factor::FlowInRange => {
                let [lo, hi] = valid_range(self.inputs.ranges.flow_m3s?)?;
                let v = level_at(self.inputs.discharge_m3s.as_ref()?, at)?;
                *available = true;
                (v >= lo && v <= hi).then_some(Evidence::FlowInRange {
                    m3s: v,
                    min_m3s: lo,
                    max_m3s: hi,
                })
            }
        }
    }
}

/// The fishing windows for the local days `inputs.first` to `inputs.last`, in time order.
///
/// Every 15 minutes (from local midnight) each enabled factor with data is checked. A step counts
/// when at least [`MIN_FACTORS`] factors match and their weights sum to at least
/// [`MIN_WEIGHT_SUM`]. A window is a maximal run of counting steps; runs at most
/// [`MERGE_GAP_S`] apart merge, windows under [`MIN_WINDOW_S`] drop, and each local day keeps its
/// [`MAX_PER_DAY`] best (largest `weight_sum`, then longest, then earliest). Windows that ended
/// before `now` are left out. An unknown zone, reversed dates or no data give an empty list.
pub fn compute(inputs: &WindowInputs, params: &WindowParams, now: UnixTime) -> Vec<FishingWindow> {
    let Ok(tz) = time::zone(&inputs.time_zone) else {
        return Vec::new();
    };
    // Local days with their UTC bounds.
    let mut days = Vec::new();
    let mut date = inputs.first;
    while days.len() < MAX_DAYS && date_key(date) <= date_key(inputs.last) {
        let Ok((start, end)) = time::local_date_bounds(date, &tz) else {
            return Vec::new();
        };
        days.push((date, start.0, end.0));
        let Ok(next) = time::local_date(UnixTime(end.0), &tz) else {
            break;
        };
        if date_key(next) <= date_key(date) {
            break;
        }
        date = next;
    }
    let (Some(&(_, span_start, _)), Some(&(_, _, span_end))) = (days.first(), days.last()) else {
        return Vec::new();
    };

    let eval = prepare(inputs, params, &tz, &days, (span_start, span_end));
    let mut steps = Vec::new();
    for &(_, start, end) in &days {
        let mut t = start;
        while t < end {
            steps.push(eval.step(t));
            t += STEP_S;
        }
    }
    let mut windows = runs(&steps, params, &tz);
    // Four per local day, best first.
    let mut kept: Vec<FishingWindow> = Vec::new();
    for &(date, _, _) in &days {
        let mut day: Vec<FishingWindow> =
            windows.iter().filter(|w| w.date == date).cloned().collect();
        day.sort_by(|a, b| {
            b.weight_sum
                .total_cmp(&a.weight_sum)
                .then((b.end.0 - b.start.0).cmp(&(a.end.0 - a.start.0)))
                .then(a.start.cmp(&b.start))
        });
        day.truncate(MAX_PER_DAY);
        kept.extend(day);
    }
    windows.clear();
    kept.retain(|w| w.end.0 > now.0);
    kept.sort_by_key(|w| w.start);
    kept
}

fn date_key(d: LocalDate) -> (i32, u8, u8) {
    (d.year, d.month, d.day)
}

fn prepare<'a>(
    inputs: &'a WindowInputs,
    params: &'a WindowParams,
    tz: &jiff::tz::TimeZone,
    days: &[(LocalDate, i64, i64)],
    span: (i64, i64),
) -> Eval<'a> {
    let (lat, lon) = (inputs.latitude, inputs.longitude);
    let settings = SolunarSettings {
        major_minutes: params.solunar_major_minutes,
        minor_minutes: params.solunar_minor_minutes,
    };
    let mut periods: Vec<(SolunarPeriod, i64)> = Vec::new();
    if params.enabled(Factor::Solunar) && lat.is_finite() && lon.is_finite() {
        let reach = i64::from(settings.major_minutes.max(settings.minor_minutes)) * 30;
        let events = moon_events(lat, lon, UnixTime(span.0 - reach), UnixTime(span.1 + reach));
        for e in &events {
            for p in solunar_periods(std::slice::from_ref(e), settings) {
                periods.push((p, e.time.0));
            }
        }
        periods.sort_by_key(|(p, c)| (p.start, p.end, *c));
    }
    let (solunar, centres) = periods.into_iter().unzip();
    let mut bands_out = Vec::new();
    if params.enabled(Factor::LowLight) && lat.is_finite() && lon.is_finite() {
        let mut dates: Vec<LocalDate> = days.iter().map(|d| d.0).collect();
        if let Ok(d) = time::local_date(UnixTime(span.0 - 1), tz) {
            dates.push(d);
        }
        if let Ok(d) = time::local_date(UnixTime(span.1), tz) {
            dates.push(d);
        }
        for d in dates {
            bands_out.extend(bands(lat, lon, d));
        }
    }
    let current_range_ms =
        valid_range(params.current_range_kn).map(|[a, b]| [a * MS_PER_KNOT, b * MS_PER_KNOT]);
    Eval {
        inputs,
        params,
        solunar,
        centres,
        bands: bands_out,
        current_range_ms,
    }
}

/// Runs of qualifying steps, merged across short gaps.
fn runs(steps: &[Step], params: &WindowParams, tz: &jiff::tz::TimeZone) -> Vec<FishingWindow> {
    let mut windows = Vec::new();
    let mut current: Option<Vec<&Step>> = None;
    let finish = |run: Vec<&Step>, windows: &mut Vec<FishingWindow>| {
        if let Some(w) = window_of(&run, params, tz) {
            windows.push(w);
        }
    };
    for step in steps {
        let sum: f64 = step.matches.iter().map(|m| m.weight).sum();
        if step.matches.len() < MIN_FACTORS || sum < MIN_WEIGHT_SUM {
            continue;
        }
        match current.as_mut() {
            Some(run) if step.t - run.last().map_or(0, |s| s.t) - STEP_S <= MERGE_GAP_S => {
                run.push(step);
            }
            _ => {
                if let Some(run) = current.take() {
                    finish(run, &mut windows);
                }
                current = Some(vec![step]);
            }
        }
    }
    if let Some(run) = current.take() {
        finish(run, &mut windows);
    }
    windows
}

fn window_of(
    run: &[&Step],
    params: &WindowParams,
    tz: &jiff::tz::TimeZone,
) -> Option<FishingWindow> {
    let start = run.first()?.t;
    let end = run.last()?.t + STEP_S;
    if end - start < MIN_WINDOW_S {
        return None;
    }
    let mut matched: Vec<FactorMatch> = Vec::new();
    let mut weight_sum = 0.0_f64;
    for step in run {
        weight_sum = weight_sum.max(step.matches.iter().map(|m| m.weight).sum());
        for m in &step.matches {
            if !matched.iter().any(|x| x.factor == m.factor) {
                matched.push(*m);
            }
        }
    }
    matched.sort_by_key(|m| m.factor);
    let no_data = Factor::ALL
        .into_iter()
        .filter(|f| params.enabled(*f) && !run.iter().any(|s| s.available[f.index()]))
        .collect();
    Some(FishingWindow {
        date: time::local_date(UnixTime(start), tz).ok()?,
        start: UnixTime(start),
        end: UnixTime(end),
        matched,
        weight_sum,
        no_data,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::assert_is_empty, clippy::format_push_string)]

    use super::*;
    use crate::datum::VerticalDatum;
    use crate::model::{CurrentEvent, CurrentKind, Spot, TideEvent, WaterType};
    use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
    use crate::time::{parse_coops, parse_date, parse_rfc3339};

    const NY: &str = "America/New_York";
    /// The Battery, New York Harbor.
    const BATTERY: (f64, f64) = (40.7006, -74.0142);

    fn prov(kind: ValueKind) -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: None,
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(0),
            kind,
            quality: Quality::Approved,
        }
    }

    fn fixture(path: &str) -> serde_json::Value {
        let full = format!(
            "{}/../../tests/fixtures/raw/{path}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"));
        serde_json::from_str(&text).unwrap()
    }

    fn coops(path: &str, key: &str) -> Vec<(UnixTime, f32, Option<char>)> {
        fixture(path)[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    parse_coops(p["t"].as_str().unwrap()).unwrap(),
                    p["v"].as_str().unwrap().parse().unwrap(),
                    p["type"].as_str().and_then(|s| s.chars().next()),
                )
            })
            .collect()
    }

    fn battery_tide(kind: TideSourceKind) -> TideSeries {
        let events = coops("coops/coops-8518750-hilo-2026-11-08.json", "predictions")
            .into_iter()
            .map(|(time, v, k)| TideEvent {
                time,
                kind: if k == Some('H') {
                    TideKind::High
                } else {
                    TideKind::Low
                },
                height_m: Some(v),
            })
            .collect();
        let curve = coops(
            "coops/coops-8518750-predictions-6min-2026-11-08.json",
            "predictions",
        )
        .into_iter()
        .map(|(t, v, _)| Sample::new(t, v))
        .collect();
        TideSeries {
            source_kind: kind,
            datum: VerticalDatum::Mllw,
            events,
            curve: Some(Series::new(prov(ValueKind::Predicted), curve)),
            observed: None,
            provenance: prov(ValueKind::Predicted),
        }
    }

    fn series_of(path: &str) -> Vec<Sample<f32>> {
        coops(path, "data")
            .into_iter()
            .map(|(t, v, _)| Sample::new(t, v))
            .collect()
    }

    fn battery_inputs(first: &str, last: &str) -> WindowInputs {
        WindowInputs {
            latitude: BATTERY.0,
            longitude: BATTERY.1,
            time_zone: NY.into(),
            first: parse_date(first).unwrap(),
            last: parse_date(last).unwrap(),
            tide: Some(battery_tide(TideSourceKind::HarmonicStation)),
            current: None,
            pressure_hpa: series_of("coops/coops-8518750-air-pressure-2026-11-07.json"),
            water_temp_c: Some(Series::new(
                prov(ValueKind::Observed),
                series_of("coops/coops-8518750-water-temperature-2026-11-07.json"),
            )),
            discharge_m3s: None,
            ranges: Ranges {
                water_temp_c: Some([10.0, 20.0]),
                flow_m3s: None,
            },
        }
    }

    fn set_weight(w: &mut FactorWeights, f: Factor, v: f64) {
        match f {
            Factor::TideStage => w.tide_stage = v,
            Factor::CurrentStrength => w.current_strength = v,
            Factor::LowLight => w.low_light = v,
            Factor::Solunar => w.solunar = v,
            Factor::PressureFalling => w.pressure_falling = v,
            Factor::WaterTemp => w.water_temp = v,
            Factor::FlowInRange => w.flow_in_range = v,
        }
    }

    /// The default parameters with only the named factors on.
    fn only(factors: &[Factor]) -> WindowParams {
        let mut p = WindowParams::default();
        for f in Factor::ALL {
            if !factors.contains(&f) {
                set_weight(&mut p.weights, f, 0.0);
            }
        }
        p
    }

    fn render(windows: &[FishingWindow], zone: &str) -> String {
        let tz = time::zone(zone).unwrap();
        let local = |t: UnixTime| {
            jiff::Timestamp::from_second(t.0)
                .unwrap()
                .to_zoned(tz.clone())
                .strftime("%m-%d %H:%M")
                .to_string()
        };
        let mut out = String::new();
        for w in windows {
            out += &format!(
                "{} to {} (day {}-{:02}-{:02}) weight_sum {:.1}\n",
                local(w.start),
                local(w.end),
                w.date.year,
                w.date.month,
                w.date.day,
                w.weight_sum
            );
            for m in &w.matched {
                out += &format!(
                    "  {:?} {:.1} {}\n",
                    m.factor,
                    m.weight,
                    serde_json::to_string(&m.evidence).unwrap()
                );
            }
            if !w.no_data.is_empty() {
                out += &format!("  no data: {:?}\n", w.no_data);
            }
        }
        out
    }

    fn start_of(inputs: &WindowInputs) -> UnixTime {
        let tz = time::zone(&inputs.time_zone).unwrap();
        time::local_date_bounds(inputs.first, &tz).unwrap().0
    }

    fn has(w: &FishingWindow, f: Factor) -> bool {
        w.matched.iter().any(|m| m.factor == f)
    }

    #[test]
    fn tide_windows_start_in_the_last_two_hours_before_the_flood_high() {
        let inputs = battery_inputs("2026-11-08", "2026-11-08");
        let params = only(&[Factor::TideStage, Factor::WaterTemp]);
        let windows = compute(&inputs, &params, start_of(&inputs));
        let highs: Vec<UnixTime> = inputs
            .tide
            .as_ref()
            .unwrap()
            .events
            .iter()
            .filter(|e| e.kind == TideKind::High)
            .map(|e| e.time)
            .collect();
        // The 12:07 UTC high, and the 00:27 UTC one that is still the evening of the 8th in New
        // York (the water temperature record holds for a day after its last sample).
        assert_eq!(windows.len(), 2, "{}", render(&windows, NY));
        for w in &windows {
            let Evidence::TideStage {
                flow,
                next_kind,
                next_at,
                seconds_to_next,
            } = w
                .matched
                .iter()
                .find(|m| m.factor == Factor::TideStage)
                .unwrap()
                .evidence
            else {
                panic!("tide evidence");
            };
            assert_eq!(flow, TideFlow::Incoming);
            assert_eq!(next_kind, TideKind::High);
            assert!(highs.contains(&next_at));
            // Windows sit on the 15-minute grid, so the start is up to a step after the exact one.
            let early = next_at.0 - 7_200;
            assert!((0..STEP_S).contains(&(w.start.0 - early)));
            // The water temperature record ends at 23:54 UTC and holds a day past it only.
            assert!(w.end.0 <= next_at.0 + STEP_S);
            assert!((7_200 - STEP_S..=7_200).contains(&seconds_to_next));
        }
    }

    #[test]
    fn battery_day_with_default_factors() {
        let inputs = battery_inputs("2026-11-08", "2026-11-08");
        let windows = compute(&inputs, &WindowParams::default(), start_of(&inputs));
        assert!(windows.iter().any(|w| has(w, Factor::TideStage)));
        assert!(
            windows
                .iter()
                .all(|w| w.matched.len() >= 2 && w.weight_sum >= 2.0)
        );
        // The pressure record ends the day before: listed as having no data.
        assert!(
            windows
                .iter()
                .all(|w| w.no_data.contains(&Factor::PressureFalling))
        );
        insta::assert_snapshot!("battery_2026_11_08", render(&windows, NY));
    }

    #[test]
    fn croton_flow_factor() {
        // 12 ft3/s on the 5th, against a best range of 10 to 15 ft3/s.
        const M3S_PER_CFS: f64 = 0.028_316_846_6;
        let doc = fixture("usgs/usgs-continuous-croton-01375000-00060-2026-11-01_2026-11-08.json");
        let samples: Vec<Sample<f32>> = doc["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let p = &f["properties"];
                let cfs: f64 = p["value"].as_str().unwrap().parse().unwrap();
                Sample::new(
                    parse_rfc3339(p["time"].as_str().unwrap()).unwrap(),
                    (cfs * M3S_PER_CFS) as f32,
                )
            })
            .collect();
        let inputs = WindowInputs {
            latitude: 41.2,
            longitude: -73.86,
            time_zone: NY.into(),
            first: parse_date("2026-11-05").unwrap(),
            last: parse_date("2026-11-05").unwrap(),
            tide: None,
            current: None,
            pressure_hpa: Vec::new(),
            water_temp_c: None,
            discharge_m3s: Some(Series::new(prov(ValueKind::Observed), samples)),
            ranges: Ranges {
                water_temp_c: None,
                flow_m3s: Some([10.0 * M3S_PER_CFS, 15.0 * M3S_PER_CFS]),
            },
        };
        let windows = compute(&inputs, &WindowParams::default(), start_of(&inputs));
        assert_eq!(windows.len(), 2, "dawn and dusk: {}", render(&windows, NY));
        assert!(
            windows
                .iter()
                .all(|w| has(w, Factor::FlowInRange) && has(w, Factor::LowLight))
        );
        insta::assert_snapshot!("croton_2026_11_05", render(&windows, NY));
        // A range the river is outside of takes the factor out of every window.
        let mut high = inputs.clone();
        high.ranges.flow_m3s = Some([1.0, 2.0]);
        assert!(compute(&high, &WindowParams::default(), start_of(&high)).is_empty());
    }

    fn coops_currents() -> Vec<CurrentEvent> {
        let doc = fixture("coops/coops-NYH1927-bin13-max-slack-2026-11-08.json");
        doc["current_predictions"]["cp"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| CurrentEvent {
                time: parse_coops(e["Time"].as_str().unwrap()).unwrap(),
                kind: match e["Type"].as_str().unwrap() {
                    "flood" => CurrentKind::MaxFlood,
                    "ebb" => CurrentKind::MaxEbb,
                    _ => CurrentKind::Slack,
                },
                speed_ms: (e["Velocity_Major"].as_f64().unwrap() / 100.0) as f32,
            })
            .collect()
    }

    fn rich_inputs() -> WindowInputs {
        let mut inputs = battery_inputs("2026-11-08", "2026-11-08");
        inputs.current = Some(CurrentSeries {
            events: coops_currents(),
            curve: None,
            flood_dir_deg: Some(11.0),
            ebb_dir_deg: Some(183.0),
            bin: Some(13),
            depth_m: None,
            observed: None,
            provenance: prov(ValueKind::Predicted),
        });
        // Pressure falling 0.6 hPa an hour from midday UTC on the 7th.
        let start = parse_coops("2026-11-07 12:00").unwrap();
        inputs.pressure_hpa = (0..60)
            .map(|h| Sample::new(UnixTime(start.0 + h * 3_600), 1_020.0 - 0.6 * h as f32))
            .collect();
        inputs.discharge_m3s = Some(Series::new(
            prov(ValueKind::Observed),
            vec![
                Sample::new(start, 5.0),
                Sample::new(UnixTime(start.0 + 72 * 3_600), 5.0),
            ],
        ));
        inputs.ranges.flow_m3s = Some([4.0, 6.0]);
        inputs
    }

    #[test]
    fn every_factor_can_be_switched_off() {
        let inputs = rich_inputs();
        let now = start_of(&inputs);
        let all = compute(&inputs, &WindowParams::default(), now);
        for f in Factor::ALL {
            assert!(
                all.iter().any(|w| has(w, f)),
                "{f:?} never matched:\n{}",
                render(&all, NY)
            );
        }
        for f in Factor::ALL {
            let mut params = WindowParams::default();
            set_weight(&mut params.weights, f, 0.0);
            let windows = compute(&inputs, &params, now);
            assert!(!windows.is_empty(), "{f:?} off leaves windows");
            for w in &windows {
                assert!(!has(w, f), "{f:?} off but listed");
                assert!(!w.no_data.contains(&f), "{f:?} off is not 'no data'");
            }
        }
        // `show_solunar = false` is the same as weight 0.
        let params = WindowParams {
            show_solunar: false,
            ..WindowParams::default()
        };
        assert!(
            compute(&inputs, &params, now)
                .iter()
                .all(|w| !has(w, Factor::Solunar))
        );
        insta::assert_snapshot!("battery_all_factors", render(&all, NY));
    }

    #[test]
    fn model_mean_tide_is_inactive() {
        let mut inputs = battery_inputs("2026-11-08", "2026-11-08");
        inputs.tide = Some(battery_tide(TideSourceKind::ModelMean));
        let windows = compute(&inputs, &WindowParams::default(), start_of(&inputs));
        assert!(windows.iter().all(|w| !has(w, Factor::TideStage)));
        assert!(
            windows
                .iter()
                .all(|w| w.no_data.contains(&Factor::TideStage))
        );
    }

    #[test]
    fn hilo_only_series_still_gives_tide_windows() {
        let mut inputs = battery_inputs("2026-11-08", "2026-11-08");
        let mut tide = battery_tide(TideSourceKind::SubordinateStation);
        tide.curve = None;
        inputs.tide = Some(tide);
        let params = only(&[Factor::TideStage, Factor::WaterTemp]);
        assert_eq!(compute(&inputs, &params, start_of(&inputs)).len(), 2);
    }

    #[test]
    fn without_data_nothing_matches_and_nothing_panics() {
        let inputs = WindowInputs {
            latitude: BATTERY.0,
            longitude: BATTERY.1,
            time_zone: NY.into(),
            first: parse_date("2026-11-08").unwrap(),
            last: parse_date("2026-11-09").unwrap(),
            tide: None,
            current: None,
            pressure_hpa: Vec::new(),
            water_temp_c: None,
            discharge_m3s: None,
            ranges: Ranges::default(),
        };
        // Low light (1.0) and solunar (0.5) never reach 2.0.
        assert!(compute(&inputs, &WindowParams::default(), UnixTime(0)).is_empty());
        let mut bad = inputs.clone();
        bad.time_zone = "Not/AZone".into();
        assert!(compute(&bad, &WindowParams::default(), UnixTime(0)).is_empty());
        let mut backwards = inputs;
        backwards.first = parse_date("2026-11-10").unwrap();
        assert!(compute(&backwards, &WindowParams::default(), UnixTime(0)).is_empty());
    }

    /// Water temperature and flow both in range for days around `date`.
    fn steady(date: &str, zone: &str, lat: f64, lon: f64) -> WindowInputs {
        let first = parse_date(date).unwrap();
        let midnight = parse_rfc3339(&format!("{date}T00:00:00Z")).unwrap();
        let start = UnixTime(midnight.0 - 86_400);
        let flat = |v: f32| {
            Series::new(
                prov(ValueKind::Observed),
                (0..24)
                    .map(|i| Sample::new(UnixTime(start.0 + i * 6 * 3_600), v))
                    .collect(),
            )
        };
        WindowInputs {
            latitude: lat,
            longitude: lon,
            time_zone: zone.into(),
            first,
            last: first,
            tide: None,
            current: None,
            pressure_hpa: Vec::new(),
            water_temp_c: Some(flat(15.0)),
            discharge_m3s: Some(flat(5.0)),
            ranges: Ranges {
                water_temp_c: Some([10.0, 20.0]),
                flow_m3s: Some([4.0, 6.0]),
            },
        }
    }

    #[test]
    fn local_days_of_23_and_25_hours() {
        let params = only(&[Factor::WaterTemp, Factor::FlowInRange]);
        for (date, hours) in [("2026-03-08", 23), ("2026-11-01", 25), ("2026-06-01", 24)] {
            let inputs = steady(date, NY, BATTERY.0, BATTERY.1);
            let windows = compute(&inputs, &params, UnixTime(0));
            assert_eq!(windows.len(), 1, "{date}");
            assert_eq!(
                windows[0].end.0 - windows[0].start.0,
                hours * 3_600,
                "{date}"
            );
        }
    }

    #[test]
    fn at_most_four_windows_a_day() {
        // Water temperature alternates in and out of range every hour; flow stays in range.
        let mut inputs = steady("2026-06-01", NY, BATTERY.0, BATTERY.1);
        let start = parse_coops("2026-05-31 00:00").unwrap();
        let samples = (0..96 * 4)
            .map(|i| {
                let t = UnixTime(start.0 + i * 900);
                let hour = (t.0 - start.0) / 3_600;
                Sample::new(t, if hour % 2 == 0 { 15.0 } else { 30.0 })
            })
            .collect();
        inputs.water_temp_c = Some(Series::new(prov(ValueKind::Observed), samples));
        let params = only(&[Factor::WaterTemp, Factor::FlowInRange]);
        let windows = compute(&inputs, &params, UnixTime(0));
        assert_eq!(windows.len(), MAX_PER_DAY, "{}", render(&windows, NY));
        assert!(windows.windows(2).all(|p| p[0].end <= p[1].start));
        assert!(windows.iter().all(|w| w.date == inputs.first));
    }

    #[test]
    fn windows_that_ended_are_left_out() {
        let inputs = battery_inputs("2026-11-08", "2026-11-08");
        let params = only(&[Factor::TideStage, Factor::WaterTemp]);
        let all = compute(&inputs, &params, start_of(&inputs));
        assert_eq!(compute(&inputs, &params, all[0].end), all[1..].to_vec());
        // One in progress stays.
        let inside = compute(&inputs, &params, UnixTime(all[0].start.0 + 60));
        assert_eq!(inside, all);
    }

    #[test]
    fn falling_pressure_needs_a_drop_of_one_hpa_in_three_hours() {
        let falling = |per_hour: f32| {
            let mut inputs = steady("2026-06-01", NY, BATTERY.0, BATTERY.1);
            let start = parse_coops("2026-05-31 12:00").unwrap();
            inputs.pressure_hpa = (0..60)
                .map(|h| Sample::new(UnixTime(start.0 + h * 3_600), 1_015.0 - per_hour * h as f32))
                .collect();
            inputs
        };
        let params = only(&[
            Factor::PressureFalling,
            Factor::WaterTemp,
            Factor::FlowInRange,
        ]);
        let drop = compute(&falling(1.0 / 3.0), &params, UnixTime(0));
        assert_eq!(drop.len(), 1);
        let Evidence::PressureFalling { change_hpa, hours } = drop[0].matched[0].evidence else {
            panic!("pressure evidence");
        };
        assert_eq!(hours, 3);
        assert!((change_hpa + 1.0).abs() < 1e-3, "{change_hpa}");
        for per_hour in [0.3, -1.0] {
            let windows = compute(&falling(per_hour), &params, UnixTime(0));
            assert!(windows.iter().all(|w| !has(w, Factor::PressureFalling)));
        }
    }

    #[test]
    fn current_strength_inside_the_range() {
        let mut inputs = rich_inputs();
        inputs.ranges = Ranges::default();
        inputs.pressure_hpa = Vec::new();
        inputs.discharge_m3s = None;
        inputs.water_temp_c = None;
        let params = only(&[Factor::CurrentStrength, Factor::LowLight]);
        let windows = compute(&inputs, &params, UnixTime(0));
        assert!(!windows.is_empty());
        for w in &windows {
            let Evidence::CurrentStrength { speed_ms, .. } = w
                .matched
                .iter()
                .find(|m| m.factor == Factor::CurrentStrength)
                .unwrap()
                .evidence
            else {
                panic!("current evidence");
            };
            assert!((0.5 * MS_PER_KNOT - 1e-9..=2.5 * MS_PER_KNOT + 1e-9).contains(&speed_ms));
        }
        // A range no current reaches.
        let mut params = params;
        params.current_range_kn = [5.0, 6.0];
        assert!(compute(&inputs, &params, UnixTime(0)).is_empty());
    }

    #[test]
    fn missing_civil_twilight_falls_back_to_sunrise() {
        // At 62 N in late June the sun never gets 6 degrees below the horizon.
        let mut inputs = steady("2026-06-21", "Europe/Helsinki", 62.0, 25.0);
        inputs.discharge_m3s = None;
        let params = only(&[Factor::LowLight, Factor::WaterTemp]);
        let windows = compute(&inputs, &params, UnixTime(0));
        assert!(!windows.is_empty());
        let w = &windows[0];
        let Evidence::LowLight {
            band,
            sun_event_at,
            minutes_from_event,
        } = w
            .matched
            .iter()
            .find(|m| m.factor == Factor::LowLight)
            .unwrap()
            .evidence
        else {
            panic!("light evidence");
        };
        assert_eq!(band, LightBand::Morning);
        assert!(
            (0..15).contains(&minutes_from_event),
            "{minutes_from_event}"
        );
        assert!(w.start.0 >= sun_event_at.0);
    }

    #[test]
    fn polar_night_and_midnight_sun_have_no_low_light() {
        for date in ["2026-12-21", "2026-06-21"] {
            let mut inputs = steady(date, "Arctic/Longyearbyen", 78.2, 15.6);
            inputs.discharge_m3s = None;
            let params = only(&[Factor::LowLight, Factor::WaterTemp]);
            assert!(compute(&inputs, &params, UnixTime(0)).is_empty(), "{date}");
        }
    }

    fn step(t: i64, factors: &[(Factor, f64)]) -> Step {
        Step {
            t,
            matches: factors
                .iter()
                .map(|&(factor, weight)| FactorMatch {
                    factor,
                    weight,
                    evidence: Evidence::WaterTemp {
                        celsius: 0.0,
                        min_c: 0.0,
                        max_c: 0.0,
                    },
                })
                .collect(),
            available: [true; 7],
        }
    }

    type Pattern<'a> = Option<&'a [(Factor, f64)]>;

    fn steps(pattern: &[Pattern<'_>]) -> Vec<Step> {
        pattern
            .iter()
            .enumerate()
            .map(|(i, f)| step(i64::try_from(i).unwrap() * STEP_S, f.unwrap_or(&[])))
            .collect()
    }

    #[test]
    fn runs_merge_short_gaps_and_drop_short_windows() {
        let tz = time::zone("UTC").unwrap();
        let p = WindowParams::default();
        let good: &[(Factor, f64)] = &[(Factor::WaterTemp, 1.0), (Factor::FlowInRange, 1.0)];
        let weak: &[(Factor, f64)] = &[(Factor::Solunar, 0.5), (Factor::PressureFalling, 0.7)];
        let one: &[(Factor, f64)] = &[(Factor::WaterTemp, 1.0)];
        // Two steps (30 minutes) make a window; one does not.
        let w = runs(&steps(&[Some(good), Some(good)]), &p, &tz);
        assert_eq!((w[0].start.0, w[0].end.0), (0, 1_800));
        assert!(runs(&steps(&[Some(good), None, None]), &p, &tz).is_empty());
        // A gap of one step (15 minutes) merges; two steps do not.
        let w = runs(&steps(&[Some(good), Some(good), None, Some(good)]), &p, &tz);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].end.0, 4 * STEP_S);
        let w = runs(
            &steps(&[Some(good), Some(good), None, None, Some(good), Some(good)]),
            &p,
            &tz,
        );
        assert_eq!(w.len(), 2);
        // A step needs two factors and a weight sum of 2.0.
        assert!(runs(&steps(&[Some(weak), Some(weak)]), &p, &tz).is_empty());
        assert!(runs(&steps(&[Some(one), Some(one)]), &p, &tz).is_empty());
        let three: &[(Factor, f64)] = &[
            (Factor::Solunar, 0.5),
            (Factor::PressureFalling, 0.7),
            (Factor::LowLight, 1.0),
        ];
        let w = runs(&steps(&[Some(three), Some(three)]), &p, &tz);
        assert!((w[0].weight_sum - 2.2).abs() < 1e-9);
    }

    #[test]
    fn from_snapshot_joins_observed_and_forecast() {
        let spot = Spot {
            id: "battery".into(),
            name: "The Battery".into(),
            latitude: BATTERY.0,
            longitude: BATTERY.1,
            water_type: WaterType::TidalRiverEstuary,
            water_body: None,
            time_zone: NY.into(),
        };
        let mut snapshot = SpotSnapshot::new(spot, UnixTime(0));
        snapshot.marine.tide = Some(battery_tide(TideSourceKind::HarmonicStation));
        snapshot.marine.observed_pressure_hpa = Some(Series::new(
            prov(ValueKind::Observed),
            series_of("coops/coops-8518750-air-pressure-2026-11-07.json"),
        ));
        let first = parse_date("2026-11-08").unwrap();
        let inputs = WindowInputs::from_snapshot(&snapshot, first, first, Ranges::default());
        assert_eq!(inputs.time_zone, NY);
        assert!(inputs.tide.is_some());
        assert!(!inputs.pressure_hpa.is_empty());
        assert!(
            inputs
                .pressure_hpa
                .windows(2)
                .all(|p| p[0].time < p[1].time)
        );
        assert!(inputs.water_temp_c.is_none());
        // Outputs serialize for the view and the FFI.
        let windows = compute(&inputs, &WindowParams::default(), UnixTime(0));
        let json = serde_json::to_string(&windows).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<FishingWindow>>(&json).unwrap(),
            windows
        );
    }

    #[test]
    fn defaults_follow_the_documented_constants() {
        let p = WindowParams::default();
        assert_eq!(p.weights, FactorWeights::default());
        assert_eq!(
            p.tide,
            TideRule {
                flow: TideFlow::Incoming,
                last_hours: 2.0
            }
        );
        assert_eq!((STEP_S, MIN_WINDOW_S, MERGE_GAP_S), (900, 1_800, 900));
        assert_eq!((MIN_FACTORS, MAX_PER_DAY), (2, 4));
        assert!((MIN_WEIGHT_SUM - 2.0).abs() < f64::EPSILON);
    }
}
