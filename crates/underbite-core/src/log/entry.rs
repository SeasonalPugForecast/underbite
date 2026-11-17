//! A catch: when, where, what, and the conditions then.
//!
//! The types here are the stored form of the catch log, so they own their vocabulary instead of
//! borrowing the snapshot's: a later change to the model must not change what an old file means.
//! Values are in canonical units (metres, kilograms, m/s, m³/s, °C, hPa, FNU) and nothing is a
//! display string; the view layer formats them in the user's units and voice.
//!
//! Exact coordinates stay in the entry on the device. The core reads no clock: ids and times
//! come from the host.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::datum::Height;
use crate::derive::pressure::{Trend, forecast_trend, observed_trend};
use crate::derive::tide_stage::{self, tide_stage};
use crate::model::{
    CurrentSeries, MarineConditions, MoonPhase, PercentileClass, SkyDay, SolunarCause, SolunarKind,
    SpotSnapshot, TideKind, TideSourceKind, Turbidity, Wind,
};
use crate::provenance::{Provenance, SourceId};
use crate::series::{Sample, Series};
use crate::time::{UnixTime, local_date, zone};
use crate::trip::SpeciesId;

/// Values further than this from the catch time are left out of its conditions.
pub const MAX_CONDITION_AGE: Duration = Duration::from_hours(1);

/// A slack current is slower than this (0.05 m/s, about 0.1 knots).
const SLACK_MS: f32 = 0.05;

/// What became of the fish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fate {
    #[default]
    Released,
    Kept,
    /// Kept, and bound for the plate: a fun tag shown beside released and kept.
    KeptForSashimi,
}

/// Where a fish was caught.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatchLocation {
    /// Degrees north, exact.
    pub latitude: f64,
    /// Degrees east, exact.
    pub longitude: f64,
    /// The saved spot it was logged at, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_id: Option<String>,
    /// A place name the angler typed or the app resolved, such as "Lower Hudson".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
}

impl CatchLocation {
    /// A location with only coordinates.
    #[must_use]
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            spot_id: None,
            area: None,
        }
    }
}

/// A photo that stays in the app container: its file name, never image data.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PhotoRef(String);

/// A string that is not a bare file name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a plain file name")]
pub struct PhotoRefError(String);

impl PhotoRef {
    /// A reference to the file `name`.
    ///
    /// # Errors
    /// [`PhotoRefError`] when `name` is empty, is `.` or `..`, or contains a path separator or a
    /// NUL, so a stored reference can never point outside the app container.
    pub fn new(name: &str) -> Result<Self, PhotoRefError> {
        let bad =
            name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']);
        if bad {
            Err(PhotoRefError(name.to_owned()))
        } else {
            Ok(Self(name.to_owned()))
        }
    }

    /// The file name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PhotoRef {
    type Error = PhotoRefError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(&s)
    }
}

impl From<PhotoRef> for String {
    fn from(p: PhotoRef) -> Self {
        p.0
    }
}

/// A condition value with where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sourced<T> {
    pub value: T,
    pub source: SourceId,
    /// The station's id, when the value came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station_name: Option<String>,
    /// Seconds from the observation to the catch: positive when the value is older than the
    /// catch, negative when it is a prediction for a moment after it.
    pub age_s: i64,
}

impl<T> Sourced<T> {
    fn from_provenance(value: T, p: &Provenance, age_s: i64) -> Self {
        Self {
            value,
            source: p.source.clone(),
            station: p.station.as_ref().map(|s| s.id.clone()),
            station_name: p.station.as_ref().map(|s| s.name.clone()),
            age_s,
        }
    }

    fn from_source(value: T, source: SourceId, age_s: i64) -> Self {
        Self {
            value,
            source,
            station: None,
            station_name: None,
            age_s,
        }
    }
}

/// Whether the tide was coming in or going out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TideDirection {
    Rising,
    Falling,
}

/// A high or a low tide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnKind {
    High,
    Low,
}

/// The tide at the catch: stage, distance from the turns, and the height when known.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TideCondition {
    pub direction: TideDirection,
    /// What the tide last turned to.
    pub previous: TurnKind,
    pub hours_since_previous: f32,
    /// What it turns to next.
    pub next: TurnKind,
    pub hours_to_next: f32,
    /// The water level, with the datum it is against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Height>,
}

/// Which way a tidal current was setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentSet {
    Flood,
    Ebb,
    Slack,
}

/// The tidal current at the catch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurrentCondition {
    /// Speed in m/s, never negative.
    pub speed_ms: f32,
    /// Absent when the flood and ebb directions are unknown, so the set cannot be told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<CurrentSet>,
    /// The direction it flows toward, degrees true, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toward_deg: Option<f32>,
}

/// River flow and how it compared with the date's normal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowCondition {
    pub discharge_m3s: f32,
    /// Percentile rank (0 to 100) for the date, when the source gave one for that day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_class: Option<PercentileClass>,
}

/// Where a river or lake stood.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GageCondition {
    pub height: Height,
}

/// Barometric pressure trend over three hours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureTrend {
    Rising,
    Falling,
    Steady,
}

impl From<Trend> for PressureTrend {
    fn from(t: Trend) -> Self {
        match t {
            Trend::Rising => Self::Rising,
            Trend::Falling => Self::Falling,
            Trend::Steady => Self::Steady,
        }
    }
}

/// Wind at the catch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindCondition {
    pub speed_ms: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gust_ms: Option<f32>,
    /// The direction it blows from, degrees true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_deg: Option<f32>,
}

/// The moon at the catch; computed, so it has no station.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoonCondition {
    pub phase: MoonPhase,
    /// Illuminated fraction, 0 to 1.
    pub illuminated_fraction: f32,
    /// Moon minus sun longitude, 0 to 360 degrees: tells waxing from waning.
    pub elongation_deg: f32,
}

/// A solunar period the catch fell in; computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolunarCondition {
    pub kind: SolunarKind,
    pub cause: SolunarCause,
    pub start: UnixTime,
    pub end: UnixTime,
}

/// The light at the catch; computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Light {
    /// From civil dawn to sunrise.
    Dawn,
    Day,
    /// From sunset to civil dusk.
    Dusk,
    Night,
}

/// The conditions at the catch, copied from the spot's snapshot. A value more than
/// [`MAX_CONDITION_AGE`] from `at` is left out, so a missing field means "not known then".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CatchConditions {
    /// The instant the conditions describe.
    pub at: UnixTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tide: Option<Sourced<TideCondition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Sourced<CurrentCondition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Sourced<FlowCondition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gage_height: Option<Sourced<GageCondition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_temperature_c: Option<Sourced<f32>>,
    /// Turbidity with the unit the source measured it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turbidity: Option<Sourced<Turbidity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure_hpa: Option<Sourced<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure_trend: Option<Sourced<PressureTrend>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind: Option<Sourced<WindCondition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air_temperature_c: Option<Sourced<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moon: Option<MoonCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solunar: Option<SolunarCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Light>,
}

/// The sample of `series` nearest to `at` within [`MAX_CONDITION_AGE`], with its age.
fn nearest<T>(series: &Series<T>, at: UnixTime) -> Option<(&Sample<T>, i64)> {
    let samples = series.samples();
    let i = samples.partition_point(|s| s.time <= at);
    let before = i.checked_sub(1).and_then(|j| samples.get(j));
    let after = samples.get(i);
    let pick = match (before, after) {
        (Some(b), Some(a)) if a.time.0 - at.0 < at.0 - b.time.0 => a,
        (Some(b), _) => b,
        (None, Some(a)) => a,
        (None, None) => return None,
    };
    let age = at.0.saturating_sub(pick.time.0);
    (age.unsigned_abs() <= MAX_CONDITION_AGE.as_secs()).then_some((pick, age))
}

/// A float rounded to two decimals, so stored hours do not carry noise.
fn round2(x: f64) -> f32 {
    ((x * 100.0).round() / 100.0) as f32
}

fn turn(kind: TideKind) -> TurnKind {
    match kind {
        TideKind::High => TurnKind::High,
        TideKind::Low => TurnKind::Low,
    }
}

fn tide_condition(marine: &MarineConditions, at: UnixTime) -> Option<Sourced<TideCondition>> {
    let series = marine.tide.as_ref()?;
    let stage = tide_stage(series, at)?;
    let height = if series.source_kind == TideSourceKind::ModelMean {
        None
    } else {
        series
            .observed
            .as_ref()
            .and_then(|o| nearest(o, at).map(|(s, _)| s.value))
            .or_else(|| {
                series
                    .curve
                    .as_ref()
                    .and_then(|c| c.at(at, MAX_CONDITION_AGE / 2))
            })
            .map(|metres| Height {
                datum: series.datum,
                metres,
            })
    };
    let direction = match stage.direction {
        tide_stage::TideDirection::Rising => TideDirection::Rising,
        tide_stage::TideDirection::Falling => TideDirection::Falling,
    };
    Some(Sourced::from_provenance(
        TideCondition {
            direction,
            previous: turn(stage.previous.kind),
            hours_since_previous: round2(stage.elapsed_s as f64 / 3_600.0),
            next: turn(stage.next.kind),
            hours_to_next: round2(stage.to_next_s as f64 / 3_600.0),
            height,
        },
        &series.provenance,
        0,
    ))
}

fn current_condition(marine: &MarineConditions, at: UnixTime) -> Option<Sourced<CurrentCondition>> {
    let c: &CurrentSeries = marine.tidal_current.as_ref()?;
    if let Some((s, age)) = c.observed.as_ref().and_then(|o| nearest(o, at)) {
        let v = s.value;
        // An observation has no sign: it is a flood when it sets nearer the flood direction.
        let set = if v.speed_ms < SLACK_MS {
            Some(CurrentSet::Slack)
        } else {
            match (c.flood_dir_deg, c.ebb_dir_deg) {
                (Some(f), Some(e)) if angle_gap(v.toward_deg, f) <= angle_gap(v.toward_deg, e) => {
                    Some(CurrentSet::Flood)
                }
                (Some(_), Some(_)) => Some(CurrentSet::Ebb),
                _ => None,
            }
        };
        return Some(Sourced::from_provenance(
            CurrentCondition {
                speed_ms: v.speed_ms,
                set,
                toward_deg: Some(v.toward_deg),
            },
            &c.provenance,
            age,
        ));
    }
    let signed = c.curve.as_ref()?.at(at, MAX_CONDITION_AGE / 2)?;
    let (set, toward) = if signed.abs() < SLACK_MS {
        (CurrentSet::Slack, None)
    } else if signed > 0.0 {
        (CurrentSet::Flood, c.flood_dir_deg)
    } else {
        (CurrentSet::Ebb, c.ebb_dir_deg)
    };
    let set = Some(set);
    Some(Sourced::from_provenance(
        CurrentCondition {
            speed_ms: signed.abs(),
            set,
            toward_deg: toward,
        },
        &c.provenance,
        0,
    ))
}

/// The smallest angle between two bearings, degrees.
fn angle_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn flow_condition(snapshot: &SpotSnapshot, at: UnixTime) -> Option<Sourced<FlowCondition>> {
    let series = snapshot.water.discharge_m3s.as_ref()?;
    let (s, age) = nearest(series, at)?;
    let tz = zone(&snapshot.spot.time_zone).ok()?;
    let day = local_date(at, &tz).ok()?;
    let key = format!("{:02}-{:02}", day.month, day.day);
    let p = snapshot
        .water
        .discharge_percentile_for_date
        .as_ref()
        .filter(|p| p.day_of_year == key);
    Some(Sourced::from_provenance(
        FlowCondition {
            discharge_m3s: s.value,
            percentile: p.and_then(|p| p.rank),
            percentile_class: p.and_then(|p| p.class),
        },
        series.provenance(),
        age,
    ))
}

fn gage_condition(snapshot: &SpotSnapshot, at: UnixTime) -> Option<Sourced<GageCondition>> {
    let h = snapshot.water.gage_height.as_ref()?;
    let (s, age) = nearest(&h.series, at)?;
    Some(Sourced::from_provenance(
        GageCondition {
            height: Height {
                datum: h.datum,
                metres: s.value,
            },
        },
        h.series.provenance(),
        age,
    ))
}

fn water_temperature(snapshot: &SpotSnapshot, at: UnixTime) -> Option<Sourced<f32>> {
    let series = snapshot
        .water
        .temperature_c
        .as_ref()
        .filter(|s| nearest(s, at).is_some())
        .or(snapshot.marine.sea_surface_temp_c.as_ref())?;
    let (s, age) = nearest(series, at)?;
    Some(Sourced::from_provenance(s.value, series.provenance(), age))
}

fn turbidity(snapshot: &SpotSnapshot, at: UnixTime) -> Option<Sourced<Turbidity>> {
    let series = snapshot.water.turbidity.as_ref()?;
    let (s, age) = nearest(series, at)?;
    Some(Sourced::from_provenance(s.value, series.provenance(), age))
}

/// One moment of the forecast provider's weather: now, or the nearest hourly step.
struct WeatherPoint {
    age_s: i64,
    pressure_hpa: Option<f32>,
    temperature_c: Option<f32>,
    wind_speed_ms: Option<f32>,
    wind_gust_ms: Option<f32>,
    wind_from_deg: Option<f32>,
}

fn weather_point(snapshot: &SpotSnapshot, at: UnixTime) -> Option<(WeatherPoint, SourceId)> {
    let w = snapshot.weather.as_ref()?;
    let limit = i64::try_from(MAX_CONDITION_AGE.as_secs()).unwrap_or(i64::MAX);
    let age_of = |t: UnixTime| at.0.saturating_sub(t.0);
    let current = w.current.as_ref().filter(|c| age_of(c.time).abs() <= limit);
    let point = if let Some(c) = current {
        WeatherPoint {
            age_s: age_of(c.time),
            pressure_hpa: c.pressure_msl.map(|v| v.0),
            temperature_c: c.temperature.map(|v| v.0),
            wind_speed_ms: c.wind_speed.map(|v| v.0),
            wind_gust_ms: c.wind_gust.map(|v| v.0),
            wind_from_deg: c.wind_dir.map(|v| v.0),
        }
    } else {
        let h = w
            .hourly
            .iter()
            .min_by_key(|h| (age_of(h.time).abs(), h.time))
            .filter(|h| age_of(h.time).abs() <= limit)?;
        WeatherPoint {
            age_s: age_of(h.time),
            pressure_hpa: h.pressure_msl.map(|v| v.0),
            temperature_c: h.temperature.map(|v| v.0),
            wind_speed_ms: h.wind_speed.map(|v| v.0),
            wind_gust_ms: h.wind_gust.map(|v| v.0),
            wind_from_deg: h.wind_dir.map(|v| v.0),
        }
    };
    Some((point, SourceId::new(w.provider.id())))
}

fn pressure(
    snapshot: &SpotSnapshot,
    at: UnixTime,
) -> (Option<Sourced<f32>>, Option<Sourced<PressureTrend>>) {
    let weather = weather_point(snapshot, at);
    let observed = snapshot.marine.observed_pressure_hpa.as_ref();
    let observed_value = observed.and_then(|s| {
        nearest(s, at)
            .map(|(sample, age)| Sourced::from_provenance(sample.value, s.provenance(), age))
    });
    let observed_trend = observed
        .and_then(|s| observed_trend(s, at).map(|t| (t, s.provenance())))
        .map(|(t, p)| Sourced::from_provenance(PressureTrend::from(t), p, 0));
    let weather_value = weather.as_ref().and_then(|(p, source)| {
        p.pressure_hpa
            .map(|v| Sourced::from_source(v, source.clone(), p.age_s))
    });
    let weather_trend = snapshot
        .weather
        .as_ref()
        .and_then(|w| forecast_trend(w, at))
        .zip(weather.as_ref())
        .map(|(t, (_, source))| Sourced::from_source(PressureTrend::from(t), source.clone(), 0));
    (
        observed_value.or(weather_value),
        observed_trend.or(weather_trend),
    )
}

fn wind(snapshot: &SpotSnapshot, at: UnixTime) -> Option<Sourced<WindCondition>> {
    if let Some(series) = snapshot.marine.observed_wind.as_ref()
        && let Some((s, age)) = nearest(series, at)
    {
        let Wind {
            speed_ms,
            gust_ms,
            from_deg,
        } = s.value;
        return Some(Sourced::from_provenance(
            WindCondition {
                speed_ms,
                gust_ms,
                from_deg,
            },
            series.provenance(),
            age,
        ));
    }
    let (p, source) = weather_point(snapshot, at)?;
    Some(Sourced::from_source(
        WindCondition {
            speed_ms: p.wind_speed_ms?,
            gust_ms: p.wind_gust_ms,
            from_deg: p.wind_from_deg,
        },
        source,
        p.age_s,
    ))
}

fn air_temperature(snapshot: &SpotSnapshot, at: UnixTime) -> Option<Sourced<f32>> {
    let (p, source) = weather_point(snapshot, at)?;
    Some(Sourced::from_source(p.temperature_c?, source, p.age_s))
}

fn light(day: &SkyDay, at: UnixTime) -> Option<Light> {
    use crate::model::Polar;
    let sun = &day.sun;
    match sun.polar {
        Some(Polar::AlwaysUp) => return Some(Light::Day),
        Some(Polar::AlwaysDown) => return Some(Light::Night),
        None => {}
    }
    let (rise, set) = (sun.sunrise?, sun.sunset?);
    Some(if at >= rise && at < set {
        Light::Day
    } else if at < rise {
        if sun.civil_dawn.is_some_and(|d| at >= d) {
            Light::Dawn
        } else {
            Light::Night
        }
    } else if sun.civil_dusk.is_some_and(|d| at < d) {
        Light::Dusk
    } else {
        Light::Night
    })
}

impl CatchConditions {
    /// The conditions at `at` from `snapshot`, leaving out any value more than
    /// [`MAX_CONDITION_AGE`] from `at`.
    ///
    /// The tide stage is worked out for `at` from the predicted turns; the moon, solunar period
    /// and light come from the sky day that contains `at` in the spot's time zone. Pressure and
    /// wind prefer the observing station over the forecast provider's weather.
    #[must_use]
    pub fn from_snapshot(snapshot: &SpotSnapshot, at: UnixTime) -> Self {
        let day = zone(&snapshot.spot.time_zone)
            .ok()
            .and_then(|tz| local_date(at, &tz).ok())
            .and_then(|date| snapshot.sky.days.iter().find(|d| d.date == date));
        let solunar = snapshot
            .sky
            .days
            .iter()
            .flat_map(|d| d.solunar.iter())
            .find(|p| p.start <= at && at <= p.end)
            .map(|p| SolunarCondition {
                kind: p.kind,
                cause: p.cause,
                start: p.start,
                end: p.end,
            });
        let (pressure_hpa, pressure_trend) = pressure(snapshot, at);
        Self {
            at,
            tide: tide_condition(&snapshot.marine, at),
            current: current_condition(&snapshot.marine, at),
            flow: flow_condition(snapshot, at),
            gage_height: gage_condition(snapshot, at),
            water_temperature_c: water_temperature(snapshot, at),
            turbidity: turbidity(snapshot, at),
            pressure_hpa,
            pressure_trend,
            wind: wind(snapshot, at),
            air_temperature_c: air_temperature(snapshot, at),
            moon: day.map(|d| MoonCondition {
                phase: d.moon.phase,
                illuminated_fraction: d.moon.illuminated_fraction,
                elongation_deg: d.moon.elongation_deg,
            }),
            solunar,
            light: day.and_then(|d| light(d, at)),
        }
    }

    /// Whether nothing at all was known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let Self {
            at: _,
            tide,
            current,
            flow,
            gage_height,
            water_temperature_c,
            turbidity,
            pressure_hpa,
            pressure_trend,
            wind,
            air_temperature_c,
            moon,
            solunar,
            light,
        } = self;
        tide.is_none()
            && current.is_none()
            && flow.is_none()
            && gage_height.is_none()
            && water_temperature_c.is_none()
            && turbidity.is_none()
            && pressure_hpa.is_none()
            && pressure_trend.is_none()
            && wind.is_none()
            && air_temperature_c.is_none()
            && moon.is_none()
            && solunar.is_none()
            && light.is_none()
    }
}

/// One catch. `id`, `created_at` and `modified_at` are the identity and clock of the record: the
/// host makes them, and a later sync merges on them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatchEntry {
    /// A stable unique id the host makes (a UUID string, say).
    pub id: String,
    pub created_at: UnixTime,
    /// The last edit; last writer wins on this.
    pub modified_at: UnixTime,
    /// A tombstone: the entry was deleted and only its identity is kept.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// When the fish was caught.
    pub caught_at: UnixTime,
    /// IANA zone name at the catch, so local time can be shown as it was.
    pub time_zone: String,
    pub location: CatchLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub species: Option<SpeciesId>,
    /// The angler's own words for the species, for fish not in the catalogue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub species_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_m: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_kg: Option<f32>,
    #[serde(default)]
    pub fate: Fate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub photos: Vec<PhotoRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<CatchConditions>,
}

impl CatchEntry {
    /// A released catch with nothing else filled in; `now` is both its creation and edit time.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        caught_at: UnixTime,
        time_zone: impl Into<String>,
        location: CatchLocation,
        now: UnixTime,
    ) -> Self {
        Self {
            id: id.into(),
            created_at: now,
            modified_at: now,
            deleted: false,
            caught_at,
            time_zone: time_zone.into(),
            location,
            species: None,
            species_text: None,
            length_m: None,
            weight_kg: None,
            fate: Fate::Released,
            lure: None,
            method: None,
            notes: None,
            photos: Vec::new(),
            conditions: None,
        }
    }

    /// The tombstone of this entry, written at `now` (never earlier than the entry's last edit).
    ///
    /// It keeps the id, the times and the catch time (for stable ordering) and drops everything
    /// else, including the position: a deleted catch leaves nothing private behind.
    #[must_use]
    pub(crate) fn tombstone(&self, now: UnixTime) -> Self {
        Self {
            modified_at: UnixTime(now.0.max(self.modified_at.0)),
            deleted: true,
            ..Self::new(
                self.id.clone(),
                self.caught_at,
                self.time_zone.clone(),
                CatchLocation::new(0.0, 0.0),
                self.created_at,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_refs_are_plain_file_names() {
        assert!(PhotoRef::new("IMG_0001.heic").is_ok());
        for bad in ["", ".", "..", "a/b.jpg", "..\\x", "a\0b"] {
            assert!(PhotoRef::new(bad).is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<PhotoRef>("\"../x.jpg\"").is_err());
    }

    #[test]
    fn nearest_prefers_the_closer_side_and_respects_the_limit() {
        use crate::provenance::{Quality, Relation, ValueKind};
        let p = Provenance {
            source: SourceId::COOPS,
            station: None,
            distance_m: None,
            relation: Relation::Nearby,
            observed_at: UnixTime(0),
            kind: ValueKind::Observed,
            quality: Quality::Approved,
        };
        let s = Series::new(
            p,
            vec![
                Sample::new(UnixTime(1_000), 1.0_f32),
                Sample::new(UnixTime(2_000), 2.0),
            ],
        );
        assert_eq!(
            nearest(&s, UnixTime(1_400)).map(|(x, a)| (x.value, a)),
            Some((1.0, 400))
        );
        assert_eq!(
            nearest(&s, UnixTime(1_600)).map(|(x, a)| (x.value, a)),
            Some((2.0, -400))
        );
        assert!(nearest(&s, UnixTime(2_000 + 3_601)).is_none());
        assert!(nearest(&s, UnixTime(1_000 - 3_601)).is_none());
        assert!(nearest(&s, UnixTime(2_000 + 3_600)).is_some());
    }

    #[test]
    fn light_follows_the_sun() {
        use crate::model::{MoonDay, SunTimes};
        let sun = SunTimes {
            civil_dawn: Some(UnixTime(100)),
            sunrise: Some(UnixTime(200)),
            sunset: Some(UnixTime(800)),
            civil_dusk: Some(UnixTime(900)),
            ..SunTimes::default()
        };
        let day = SkyDay {
            date: crate::time::LocalDate {
                year: 2026,
                month: 11,
                day: 8,
            },
            sun,
            moon: MoonDay {
                rise: None,
                set: None,
                upper_transit: None,
                lower_transit: None,
                elongation_deg: 0.0,
                illuminated_fraction: 0.0,
                phase: MoonPhase::New,
                nearest_principal: None,
            },
            solunar: Vec::new(),
        };
        let at = |t| light(&day, UnixTime(t));
        assert_eq!(at(50), Some(Light::Night));
        assert_eq!(at(100), Some(Light::Dawn));
        assert_eq!(at(200), Some(Light::Day));
        assert_eq!(at(799), Some(Light::Day));
        assert_eq!(at(800), Some(Light::Dusk));
        assert_eq!(at(900), Some(Light::Night));
    }

    #[test]
    fn a_tombstone_keeps_identity_and_drops_the_rest() {
        let mut e = CatchEntry::new(
            "a",
            UnixTime(10),
            "America/New_York",
            CatchLocation::new(40.7, -74.0),
            UnixTime(20),
        );
        e.notes = Some("secret spot".into());
        e.photos.push(PhotoRef::new("a.heic").unwrap());
        let t = e.tombstone(UnixTime(15));
        assert!(t.deleted);
        assert_eq!(t.modified_at, UnixTime(20));
        assert_eq!(t.caught_at, UnixTime(10));
        assert_eq!(t.created_at, UnixTime(20));
        assert!(t.notes.is_none() && t.photos.is_empty());
        assert_eq!(t.location, CatchLocation::new(0.0, 0.0));
    }
}
