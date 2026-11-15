//! Tide prediction from harmonic constituents, highs and lows, and subordinate stations.
//!
//! The height above datum `D` of a harmonic station is
//! `h(t) = Z0 + Σ f·A·cos(V0(t) + u − g)`, with `A` and `g` the amplitude and Greenwich phase lag
//! of each constituent (CO-OPS `harcon`, `phase_GMT`), `Z0 = MSL − D` from the station's datum
//! table, and `f`, `u` the node factor and phase of the year. As in NOAA practice, `f` and `u`
//! are taken at the middle of each UTC year and `V0` at its start, advancing at the
//! constituent's speed.
//!
//! Subordinate stations have no constituents: their highs and lows are the reference station's
//! shifted in time and scaled (or raised) in height, and their curve is interpolated between those
//! events.

use std::f64::consts::PI;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::astro::constituents::{
    AstronomicalArguments, Constituent, utc_year, year_start, yearly_node_corrections,
};
use crate::datum::{DatumTable, VerticalDatum};
use crate::model::{TideEvent, TideKind, TideSeries, TideSourceKind};
use crate::provenance::{Provenance, Quality, Relation, SourceId, StationKind, ValueKind};
use crate::series::{Sample, Series};
use crate::time::UnixTime;

/// Spacing of the grid on which turning points are bracketed, seconds.
const SEARCH_STEP: i64 = 360;
/// Bisection steps: 360 s / 2^12 is below 0.1 s.
const BISECTIONS: u32 = 12;
/// Reference events computed beyond the window so that shifted events and curves cover it.
const SUBORDINATE_MARGIN: i64 = 86_400;

/// Why a tide could not be computed.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum TideError {
    /// The station has no harmonic constituents (subordinate stations have none).
    #[error("station {0} has no harmonic constituents")]
    NoConstituents(String),
    /// A constituent with a non-zero amplitude is not in the constituent table.
    #[error("unknown tidal constituent {0}")]
    UnknownConstituent(String),
    /// The station's datum table lacks MSL or the requested datum.
    #[error("station {station_id} has no {datum:?} or MSL in its datum table")]
    MissingDatum {
        /// The station.
        station_id: String,
        /// The datum asked for.
        datum: VerticalDatum,
    },
    /// The step is zero or the window ends before it starts.
    #[error("empty or reversed time window, or zero step")]
    BadWindow,
    /// Harmonics were requested for a subordinate station.
    #[error("subordinate stations are predicted from their reference station")]
    SubordinateStation,
    /// Subordinate offsets apply to heights above MLLW only.
    #[error("subordinate offsets need reference heights above MLLW, not {0:?}")]
    SubordinateDatum(VerticalDatum),
    /// The offsets name another reference station.
    #[error("offsets refer to station {expected}, not {actual}")]
    ReferenceMismatch {
        /// The reference station the offsets name.
        expected: String,
        /// The station of the harmonics given.
        actual: String,
    },
}

/// One constituent of a station, as CO-OPS `harcon` lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarmonicConstituent {
    /// CO-OPS name (`M2`, `LAM2`, `2SM2`, `RHO`, ...).
    pub name: String,
    /// Amplitude, metres.
    pub amplitude_m: f32,
    /// Phase lag against the Greenwich equilibrium argument, UTC, degrees (`phase_GMT`).
    pub phase_gmt_deg: f32,
}

/// The harmonic constants of a tide station, with the mean level above the prediction datum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TideHarmonics {
    /// The station id as CO-OPS writes it.
    pub station_id: String,
    /// Constituents; ones missing from the constituent table must have zero amplitude.
    pub constituents: Vec<HarmonicConstituent>,
    /// Mean sea level above `datum`, metres (`MSL − datum`).
    pub z0_m: f32,
    /// The datum predictions are relative to.
    pub datum: VerticalDatum,
}

impl TideHarmonics {
    /// Harmonics predicting heights above `datum`, with `Z0` from the station's datum table.
    ///
    /// Fails for a subordinate station, whose constituent list is empty and datum table null.
    pub fn from_datum_table(
        station_id: impl Into<String>,
        constituents: Vec<HarmonicConstituent>,
        table: &DatumTable,
        datum: VerticalDatum,
    ) -> Result<Self, TideError> {
        let station_id = station_id.into();
        if constituents.is_empty() {
            return Err(TideError::NoConstituents(station_id));
        }
        let Some(z0_m) = table.z0(datum) else {
            return Err(TideError::MissingDatum { station_id, datum });
        };
        Ok(Self {
            station_id,
            constituents,
            z0_m,
            datum,
        })
    }
}

/// How a subordinate station's heights follow the reference station's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeightAdjustment {
    /// The reference height times this ratio (CO-OPS `heightAdjustedType` `R`).
    Ratio(f32),
    /// The reference height plus this many metres (CO-OPS `F`).
    Add(f32),
}

impl HeightAdjustment {
    /// The adjusted height, metres.
    pub fn apply(self, reference_m: f32) -> f32 {
        match self {
            Self::Ratio(r) => reference_m * r,
            Self::Add(m) => reference_m + m,
        }
    }
}

/// The offsets of a subordinate tide station from its reference (CO-OPS `tidepredoffsets`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubordinateTideOffsets {
    /// The reference (harmonic) station.
    pub reference_id: String,
    /// Minutes added to the reference high-tide times.
    pub high_time_min: i32,
    /// Minutes added to the reference low-tide times.
    pub low_time_min: i32,
    /// Adjustment of high-tide heights.
    pub high_height: HeightAdjustment,
    /// Adjustment of low-tide heights.
    pub low_height: HeightAdjustment,
}

/// One constituent's term for a year: `f·A`, phase at the year's start and angular speed.
#[derive(Clone, Copy, Debug)]
struct Term {
    amplitude: f64,
    phase_rad: f64,
    speed_rad_s: f64,
}

/// The terms of one UTC year.
#[derive(Clone, Debug)]
struct Year {
    start: f64,
    terms: Vec<Term>,
}

/// A station's harmonics resolved into per-year terms over a span of years.
#[derive(Clone, Debug)]
struct Model {
    z0: f64,
    first_year: i32,
    years: Vec<Year>,
}

impl Model {
    fn new(h: &TideHarmonics, from: UnixTime, to: UnixTime) -> Result<Self, TideError> {
        if h.constituents.is_empty() {
            return Err(TideError::NoConstituents(h.station_id.clone()));
        }
        let mut resolved = Vec::new();
        for c in &h.constituents {
            match Constituent::from_coops_name(&c.name) {
                Some(k) => resolved.push((k, c)),
                None if c.amplitude_m == 0.0 => {}
                None => return Err(TideError::UnknownConstituent(c.name.clone())),
            }
        }
        let first_year = utc_year(UnixTime(from.0 - 2 * SEARCH_STEP));
        let last_year = utc_year(UnixTime(to.0 + 2 * SEARCH_STEP));
        let years = (first_year..=last_year)
            .map(|year| {
                let start = year_start(year);
                let args = AstronomicalArguments::at(start);
                let corrections = yearly_node_corrections(year);
                let terms = resolved
                    .iter()
                    .map(|&(k, c)| {
                        let (f, u) = corrections.get(&k).map_or((1.0, 0.0), |n| (n.f, n.u));
                        let phase = k.equilibrium_argument(&args) + u - f64::from(c.phase_gmt_deg);
                        Term {
                            amplitude: f * f64::from(c.amplitude_m),
                            phase_rad: phase.rem_euclid(360.0).to_radians(),
                            speed_rad_s: k.speed().to_radians() / 3600.0,
                        }
                    })
                    .collect();
                Year {
                    start: start.0 as f64,
                    terms,
                }
            })
            .collect();
        Ok(Self {
            z0: f64::from(h.z0_m),
            first_year,
            years,
        })
    }

    fn year(&self, t: f64) -> &Year {
        let offset = utc_year(UnixTime(t.floor() as i64)) - self.first_year;
        let last = self.years.len() - 1;
        // INVARIANT: `new` builds at least one year, so `last` is a valid index.
        &self.years[usize::try_from(offset).unwrap_or(0).min(last)]
    }

    /// Height at `t` (seconds since the epoch), metres above the datum.
    fn height(&self, t: f64) -> f64 {
        let year = self.year(t);
        let dt = t - year.start;
        self.z0
            + year
                .terms
                .iter()
                .map(|k| k.amplitude * (k.phase_rad + k.speed_rad_s * dt).cos())
                .sum::<f64>()
    }

    /// Rate of change at `t`, metres per second.
    fn rate(&self, t: f64) -> f64 {
        let year = self.year(t);
        let dt = t - year.start;
        -year
            .terms
            .iter()
            .map(|k| k.amplitude * k.speed_rad_s * (k.phase_rad + k.speed_rad_s * dt).sin())
            .sum::<f64>()
    }

    /// Turning points whose time, rounded to the minute, lies in `[from, to]`.
    fn extremes(&self, from: UnixTime, to: UnixTime) -> Vec<TideEvent> {
        let mut events = Vec::new();
        let mut t = from.0 - SEARCH_STEP;
        let mut rate = self.rate(t as f64);
        while t <= to.0 + SEARCH_STEP {
            let next = t + SEARCH_STEP;
            let next_rate = self.rate(next as f64);
            let kind = if rate > 0.0 && next_rate <= 0.0 {
                Some(TideKind::High)
            } else if rate < 0.0 && next_rate >= 0.0 {
                Some(TideKind::Low)
            } else {
                None
            };
            if let Some(kind) = kind {
                let exact = self.refine(t as f64, next as f64, rate > 0.0);
                let time = UnixTime((exact / 60.0).round() as i64 * 60);
                if (from..=to).contains(&time) {
                    events.push(TideEvent {
                        time,
                        kind,
                        height_m: Some(self.height(exact) as f32),
                    });
                }
            }
            (t, rate) = (next, next_rate);
        }
        events
    }

    /// Bisection for the zero of the rate between `a` and `b`; `rising_at_a` is its sign at `a`.
    fn refine(&self, mut a: f64, mut b: f64, rising_at_a: bool) -> f64 {
        for _ in 0..BISECTIONS {
            let mid = f64::midpoint(a, b);
            if (self.rate(mid) > 0.0) == rising_at_a {
                a = mid;
            } else {
                b = mid;
            }
        }
        f64::midpoint(a, b)
    }
}

fn check_window(from: UnixTime, to: UnixTime, step: Duration) -> Result<i64, TideError> {
    match i64::try_from(step.as_secs()) {
        Ok(s) if s > 0 && from <= to => Ok(s),
        _ => Err(TideError::BadWindow),
    }
}

fn harmonic_provenance(observed_at: UnixTime) -> Provenance {
    Provenance {
        source: SourceId::COOPS_HARMONICS,
        station: None,
        distance_m: None,
        relation: Relation::SameWaterBody,
        observed_at,
        kind: ValueKind::Computed,
        quality: Quality::Approved,
    }
}

fn predict_samples(
    h: &TideHarmonics,
    from: UnixTime,
    to: UnixTime,
    step: Duration,
) -> Result<Vec<Sample<f32>>, TideError> {
    let step = check_window(from, to, step)?;
    let model = Model::new(h, from, to)?;
    Ok((from.0..=to.0)
        .step_by(usize::try_from(step).map_err(|_| TideError::BadWindow)?)
        .map(|t| Sample::new(UnixTime(t), model.height(t as f64) as f32))
        .collect())
}

/// Heights from `from` to `to` (inclusive) every `step`, metres above `h.datum`.
///
/// The series is `Computed` from `coops-harmonics`, with no station attached.
pub fn predict(
    h: &TideHarmonics,
    from: UnixTime,
    to: UnixTime,
    step: Duration,
) -> Result<Series<f32>, TideError> {
    let samples = predict_samples(h, from, to, step)?;
    Ok(Series::new(harmonic_provenance(from), samples))
}

/// Highs and lows between `from` and `to` (inclusive), times rounded to the minute as CO-OPS
/// publishes them, heights above `h.datum` at the exact turning point.
pub fn extremes(
    h: &TideHarmonics,
    from: UnixTime,
    to: UnixTime,
) -> Result<Vec<TideEvent>, TideError> {
    check_window(from, to, Duration::from_secs(1))?;
    Ok(Model::new(h, from, to)?.extremes(from, to))
}

/// A subordinate station's highs and lows from its reference station's, sorted by time.
///
/// Heights stay relative to the reference's datum (MLLW for CO-OPS offsets).
pub fn subordinate(
    reference_events: &[TideEvent],
    offsets: &SubordinateTideOffsets,
) -> Vec<TideEvent> {
    let mut events: Vec<TideEvent> = reference_events
        .iter()
        .map(|e| {
            let (minutes, adjustment) = match e.kind {
                TideKind::High => (offsets.high_time_min, offsets.high_height),
                TideKind::Low => (offsets.low_time_min, offsets.low_height),
            };
            TideEvent {
                time: UnixTime(e.time.0 + i64::from(minutes) * 60),
                kind: e.kind,
                height_m: e.height_m.map(|h| adjustment.apply(h)),
            }
        })
        .collect();
    events.sort_by_key(|e| e.time);
    events
}

/// An approximate curve through highs and lows: half a cosine from each event to the next,
/// sampled at the multiples of `step` between the first and last event with a height.
pub fn curve_from_events(events: &[TideEvent], step: Duration) -> Vec<Sample<f32>> {
    let mut points: Vec<(i64, f64)> = events
        .iter()
        .filter_map(|e| Some((e.time.0, f64::from(e.height_m?))))
        .collect();
    points.sort_by_key(|p| p.0);
    points.dedup_by_key(|p| p.0);
    let Ok(step) = i64::try_from(step.as_secs()) else {
        return Vec::new();
    };
    let (Some(&(first, _)), Some(&(last, _))) = (points.first(), points.last()) else {
        return Vec::new();
    };
    if step <= 0 {
        return Vec::new();
    }
    let mut samples = Vec::new();
    let mut t = first.div_euclid(step) * step;
    if t < first {
        t += step;
    }
    let mut i = 0;
    while t <= last {
        while i + 2 < points.len() && points[i + 1].0 < t {
            i += 1;
        }
        let (t0, h0) = points[i];
        let value = match points.get(i + 1) {
            Some(&(t1, h1)) => {
                let x = (t - t0) as f64 / (t1 - t0) as f64;
                h0 + (h1 - h0) * (1.0 - (PI * x).cos()) / 2.0
            }
            None => h0,
        };
        samples.push(Sample::new(UnixTime(t), value as f32));
        t += step;
    }
    samples
}

/// Builds the [`TideSeries`] of a spot for a window, from harmonics, from subordinate offsets or
/// from CO-OPS predictions.
///
/// `base` describes the station and how it relates to the spot; each builder sets the source,
/// value kind and quality that fit the case.
#[derive(Clone, Debug)]
pub struct TideSeriesBuilder {
    /// Start of the window, inclusive.
    pub from: UnixTime,
    /// End of the window, inclusive.
    pub to: UnixTime,
    /// Spacing of the curve.
    pub step: Duration,
    /// Station, distance, relation and issue time of the result.
    pub base: Provenance,
}

impl TideSeriesBuilder {
    fn is_subordinate(&self) -> bool {
        self.base
            .station
            .as_ref()
            .is_some_and(|s| s.kind == StationKind::TideSubordinate)
    }

    fn provenance(&self, source: SourceId, kind: ValueKind, quality: Quality) -> Provenance {
        Provenance {
            source,
            kind,
            quality,
            ..self.base.clone()
        }
    }

    /// A harmonic station computed on the device: `Computed` heights and highs and lows.
    pub fn harmonic(&self, h: &TideHarmonics) -> Result<TideSeries, TideError> {
        if self.is_subordinate() {
            return Err(TideError::SubordinateStation);
        }
        let provenance = self.provenance(
            SourceId::COOPS_HARMONICS,
            ValueKind::Computed,
            Quality::Approved,
        );
        let samples = predict_samples(h, self.from, self.to, self.step)?;
        Ok(TideSeries {
            source_kind: TideSourceKind::Computed,
            datum: h.datum,
            events: extremes(h, self.from, self.to)?,
            curve: Some(Series::new(provenance.clone(), samples)),
            observed: None,
            provenance,
        })
    }

    /// A subordinate station computed on the device from its reference station's harmonics:
    /// highs and lows by offsets, with an approximate (`Estimated`) curve between them.
    pub fn subordinate(
        &self,
        reference: &TideHarmonics,
        offsets: &SubordinateTideOffsets,
    ) -> Result<TideSeries, TideError> {
        if reference.station_id != offsets.reference_id {
            return Err(TideError::ReferenceMismatch {
                expected: offsets.reference_id.clone(),
                actual: reference.station_id.clone(),
            });
        }
        if reference.datum != VerticalDatum::Mllw {
            return Err(TideError::SubordinateDatum(reference.datum));
        }
        check_window(self.from, self.to, self.step)?;
        let shift = 60
            * i64::from(offsets.high_time_min.unsigned_abs())
                .max(i64::from(offsets.low_time_min.unsigned_abs()));
        let margin = SUBORDINATE_MARGIN + shift;
        let from = UnixTime(self.from.0 - margin);
        let to = UnixTime(self.to.0 + margin);
        let all = subordinate(&extremes(reference, from, to)?, offsets);
        let provenance = self.provenance(
            SourceId::COOPS_HARMONICS,
            ValueKind::Computed,
            Quality::Approved,
        );
        Ok(self.events_and_curve(
            TideSourceKind::SubordinateStation,
            VerticalDatum::Mllw,
            &all,
            provenance,
        ))
    }

    /// CO-OPS predictions passed through. Without a curve (subordinate stations serve `hilo`
    /// only) one is interpolated between the events and marked `Estimated`.
    pub fn coops(
        &self,
        datum: VerticalDatum,
        events: Vec<TideEvent>,
        curve: Option<Vec<Sample<f32>>>,
    ) -> TideSeries {
        let source_kind = if self.is_subordinate() {
            TideSourceKind::SubordinateStation
        } else {
            TideSourceKind::HarmonicStation
        };
        let provenance = self.provenance(SourceId::COOPS, ValueKind::Predicted, self.base.quality);
        match curve {
            Some(samples) => TideSeries {
                source_kind,
                datum,
                events,
                curve: Some(Series::new(provenance.clone(), samples)),
                observed: None,
                provenance,
            },
            None => self.events_and_curve(source_kind, datum, &events, provenance),
        }
    }

    /// Events in the window and a cosine curve through all of them, cut to the window.
    fn events_and_curve(
        &self,
        source_kind: TideSourceKind,
        datum: VerticalDatum,
        events: &[TideEvent],
        provenance: Provenance,
    ) -> TideSeries {
        let window = self.from..=self.to;
        let curve: Vec<_> = curve_from_events(events, self.step)
            .into_iter()
            .filter(|s| window.contains(&s.time))
            .collect();
        let curve_provenance = Provenance {
            source: provenance.source.clone(),
            kind: ValueKind::Computed,
            quality: Quality::Estimated,
            ..self.base.clone()
        };
        TideSeries {
            source_kind,
            datum,
            events: events
                .iter()
                .filter(|e| window.contains(&e.time))
                .copied()
                .collect(),
            curve: (!curve.is_empty()).then(|| Series::new(curve_provenance, curve)),
            observed: None,
            provenance,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;
    use whether_core::time::days_from_civil;

    use super::*;
    use crate::provenance::StationRef;

    macro_rules! fixture {
        ($file:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/raw/coops/",
                $file
            ))
        };
    }

    const MINUTE: i64 = 60;

    fn utc(y: i32, m: u8, d: u8) -> UnixTime {
        UnixTime(days_from_civil(y, m, d) * 86_400)
    }

    /// `YYYY-MM-DD HH:MM`, UTC.
    fn coops_time(s: &str) -> UnixTime {
        let n = |r: std::ops::Range<usize>| s[r].parse::<i64>().unwrap();
        let day = days_from_civil(n(0..4) as i32, n(5..7) as u8, n(8..10) as u8);
        UnixTime(day * 86_400 + n(11..13) * 3600 + n(14..16) * MINUTE)
    }

    fn harcon(json: &str, to_metres: f32) -> Vec<HarmonicConstituent> {
        let v: Value = serde_json::from_str(json).unwrap();
        v["HarmonicConstituents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| HarmonicConstituent {
                name: c["name"].as_str().unwrap().to_owned(),
                amplitude_m: c["amplitude"].as_f64().unwrap() as f32 * to_metres,
                phase_gmt_deg: c["phase_GMT"].as_f64().unwrap() as f32,
            })
            .collect()
    }

    fn datums(station: &str, json: &str) -> DatumTable {
        let v: Value = serde_json::from_str(json).unwrap();
        let values: BTreeMap<String, f32> = v["datums"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|d| {
                        Some((d["name"].as_str()?.to_owned(), d["value"].as_f64()? as f32))
                    })
                    .collect()
            })
            .unwrap_or_default();
        DatumTable::new(station, values)
    }

    /// `(time, height, kind)` of a CO-OPS `predictions` response.
    fn predictions(json: &str) -> Vec<(UnixTime, f32, Option<TideKind>)> {
        let v: Value = serde_json::from_str(json).unwrap();
        v["predictions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let kind = p["type"].as_str().map(|k| match k {
                    "H" => TideKind::High,
                    "L" => TideKind::Low,
                    other => panic!("type {other}"),
                });
                (
                    coops_time(p["t"].as_str().unwrap()),
                    p["v"].as_str().unwrap().parse().unwrap(),
                    kind,
                )
            })
            .collect()
    }

    fn ev(time: i64, kind: TideKind, height_m: f32) -> TideEvent {
        let height_m = Some(height_m);
        let time = UnixTime(time);
        TideEvent {
            time,
            kind,
            height_m,
        }
    }

    fn events(json: &str) -> Vec<TideEvent> {
        let all = predictions(json).into_iter();
        all.map(|(t, h, kind)| ev(t.0, kind.unwrap(), h)).collect()
    }

    fn station(id: &str, harcon_json: &str, datums_json: &str) -> TideHarmonics {
        let (constituents, table) = (harcon(harcon_json, 1.0), datums(id, datums_json));
        TideHarmonics::from_datum_table(id, constituents, &table, VerticalDatum::Mllw).unwrap()
    }

    fn battery() -> TideHarmonics {
        let harcon = fixture!("coops-mdapi-8518750-harcon-metric.json");
        station(
            "8518750",
            harcon,
            fixture!("coops-mdapi-8518750-datums-metric.json"),
        )
    }

    fn pensacola() -> TideHarmonics {
        let harcon = fixture!("coops-mdapi-8729840-harcon-metric.json");
        station(
            "8729840",
            harcon,
            fixture!("coops-mdapi-8729840-datums-metric.json"),
        )
    }

    fn tarrytown_offsets() -> SubordinateTideOffsets {
        let v: Value =
            serde_json::from_str(fixture!("coops-mdapi-8518919-tidepredoffsets.json")).unwrap();
        assert_eq!(v["heightAdjustedType"], "R");
        let ratio = |key: &str| HeightAdjustment::Ratio(v[key].as_f64().unwrap() as f32);
        let minutes = |key: &str| i32::try_from(v[key].as_i64().unwrap()).unwrap();
        SubordinateTideOffsets {
            reference_id: v["refStationId"].as_str().unwrap().to_owned(),
            high_time_min: minutes("timeOffsetHighTide"),
            low_time_min: minutes("timeOffsetLowTide"),
            high_height: ratio("heightOffsetHighTide"),
            low_height: ratio("heightOffsetLowTide"),
        }
    }

    /// Worst time error (minutes) and worst height error (metres) of `ours` against `theirs`,
    /// matched one-to-one in order and by kind.
    fn event_errors(label: &str, ours: &[TideEvent], theirs: &[TideEvent]) -> (i64, f32) {
        assert_eq!(ours.len(), theirs.len(), "{label}: event count");
        let mut worst = (0, 0.0_f32);
        for (a, b) in ours.iter().zip(theirs) {
            assert_eq!(a.kind, b.kind, "{label}: kind at {:?}", b.time);
            let dt = (a.time.0 - b.time.0).abs() / MINUTE;
            let dh = (a.height_m.unwrap() - b.height_m.unwrap()).abs();
            worst = (worst.0.max(dt), worst.1.max(dh));
        }
        worst
    }

    /// Checks a week against the CO-OPS 6-minute and `hilo` predictions with the project's
    /// tolerances, printing the measured errors.
    fn check_week(label: &str, h: &TideHarmonics, six_minute: &str, hilo: &str, count: usize) {
        let expected = predictions(six_minute);
        let (from, to) = (expected[0].0, expected[expected.len() - 1].0);
        let series = predict(h, from, to, Duration::from_mins(6)).unwrap();
        assert_eq!(series.samples().len(), expected.len(), "{label}: points");
        let (mut sum, mut max) = (0.0_f64, 0.0_f64);
        for (s, &(t, v, _)) in series.samples().iter().zip(&expected) {
            assert_eq!(s.time, t);
            let e = f64::from(s.value - v).abs();
            sum += e * e;
            max = max.max(e);
        }
        let rms = (sum / expected.len() as f64).sqrt();

        let theirs = events(hilo);
        assert_eq!(theirs.len(), count, "{label}: fixture events");
        let ours = extremes(h, from, UnixTime(to.0 + 6 * MINUTE)).unwrap();
        let (dt, dh) = event_errors(label, &ours, &theirs);
        println!(
            "{label}: rms {:.1} mm, max {:.1} mm; {} events, times within {dt} min, \
             heights within {:.1} mm",
            rms * 1000.0,
            max * 1000.0,
            ours.len(),
            dh * 1000.0
        );
        assert!(rms <= 0.03 && max <= 0.06, "{label}: rms {rms}, max {max}");
        assert!(dt <= 6 && dh <= 0.04, "{label}: {dt} min, {dh} m");
    }

    #[test]
    fn battery_week_2026() {
        check_week(
            "8518750 2026-11-08",
            &battery(),
            fixture!("coops-8518750-predictions-6min-2026-11-08.json"),
            fixture!("coops-8518750-hilo-2026-11-08.json"),
            27,
        );
    }

    #[test]
    fn battery_week_2034() {
        check_week(
            "8518750 2034-06-01",
            &battery(),
            fixture!("coops-8518750-predictions-6min-2034-06-01.json"),
            fixture!("coops-8518750-hilo-2034-06-01.json"),
            28,
        );
    }

    #[test]
    fn pensacola_diurnal_week() {
        check_week(
            "8729840 2026-11-08",
            &pensacola(),
            fixture!("coops-8729840-predictions-6min-2026-11-08.json"),
            fixture!("coops-8729840-hilo-2026-11-08.json"),
            14,
        );
    }

    #[test]
    fn harcon_in_feet_gives_the_same_heights() {
        let feet = TideHarmonics {
            constituents: harcon(fixture!("coops-mdapi-8518750-harcon.json"), 0.3048),
            ..battery()
        };
        let (from, to) = (utc(2026, 11, 8), utc(2026, 11, 15));
        let step = Duration::from_hours(1);
        let a = predict(&battery(), from, to, step).unwrap();
        let b = predict(&feet, from, to, step).unwrap();
        for (x, y) in a.samples().iter().zip(b.samples()) {
            assert!((x.value - y.value).abs() < 0.003, "{:?}", x.time);
        }
        assert_eq!(a.provenance().kind, ValueKind::Computed);
        assert_eq!(a.provenance().source, SourceId::COOPS_HARMONICS);
    }

    #[test]
    fn tarrytown_from_the_recorded_battery_events_matches_coops() {
        let battery_events = events(fixture!("coops-8518750-hilo-2026-11-08.json"));
        let expected = events(fixture!("coops-8518919-hilo-2026-11-08.json"));
        let ours = subordinate(&battery_events, &tarrytown_offsets());
        // The first Tarrytown events come from Battery events before the recorded week, and the
        // last Battery events shift past its end: compare where both exist.
        let (first, last) = (ours[0].time, expected[expected.len() - 1].time);
        let ours: Vec<_> = ours.into_iter().filter(|e| e.time <= last).collect();
        let theirs: Vec<_> = expected.into_iter().filter(|e| e.time >= first).collect();
        assert!(ours.len() >= 25, "{}", ours.len());
        assert_eq!(ours.len(), theirs.len());
        for (a, b) in ours.iter().zip(&theirs) {
            assert_eq!((a.time, a.kind), (b.time, b.kind));
            let dh = (a.height_m.unwrap() - b.height_m.unwrap()).abs();
            assert!(dh <= 0.0011, "{:?}: {dh}", b.time);
        }
    }

    #[test]
    fn tarrytown_from_computed_battery_extremes() {
        let expected = events(fixture!("coops-8518919-hilo-2026-11-08.json"));
        let reference = extremes(&battery(), utc(2026, 11, 7), utc(2026, 11, 16)).unwrap();
        let window = utc(2026, 11, 8)..utc(2026, 11, 15);
        let ours: Vec<_> = subordinate(&reference, &tarrytown_offsets())
            .into_iter()
            .filter(|e| window.contains(&e.time))
            .collect();
        let (dt, dh) = event_errors("8518919", &ours, &expected);
        println!(
            "8518919 2026-11-08: {} events, times within {dt} min, heights within {:.1} mm",
            ours.len(),
            dh * 1000.0
        );
        assert!(dt <= 6 && dh <= 0.04, "{dt} min, {dh} m");
    }

    #[test]
    fn additive_offsets_raise_heights() {
        let offsets = SubordinateTideOffsets {
            reference_id: "8518750".into(),
            high_time_min: -30,
            low_time_min: 15,
            high_height: HeightAdjustment::Add(0.25),
            low_height: HeightAdjustment::Add(-0.1),
        };
        let reference = [ev(3600, TideKind::Low, 0.2), ev(4000, TideKind::High, 1.5)];
        let out = subordinate(&reference, &offsets);
        assert_eq!(out[0].kind, TideKind::High);
        assert_eq!(out[0].time, UnixTime(4000 - 1800));
        assert!((out[0].height_m.unwrap() - 1.75).abs() < 1e-6);
        assert_eq!(out[1].time, UnixTime(3600 + 900));
        assert!((out[1].height_m.unwrap() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn cosine_curve_passes_through_the_events() {
        let events = [
            ev(0, TideKind::Low, 0.0),
            ev(21_600, TideKind::High, 2.0),
            ev(43_200, TideKind::Low, 0.4),
        ];
        let curve = curve_from_events(&events, Duration::from_mins(30));
        assert_eq!(curve.len(), 25);
        let at = |t: i64| curve.iter().find(|s| s.time.0 == t).unwrap().value;
        assert!(at(0).abs() < 1e-6);
        assert!((at(10_800) - 1.0).abs() < 1e-6);
        assert!((at(21_600) - 2.0).abs() < 1e-6);
        assert!((at(32_400) - 1.2).abs() < 1e-6);
        assert!((at(43_200) - 0.4).abs() < 1e-6);
        assert!(curve.windows(2).take(12).all(|w| w[0].value <= w[1].value));
    }

    fn base(kind: StationKind) -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: Some(StationRef {
                source: SourceId::COOPS,
                id: "8518919".into(),
                name: "Tarrytown".into(),
                kind,
                latitude: 41.0733,
                longitude: -73.8833,
                bin: None,
                reference_id: Some("8518750".into()),
            }),
            distance_m: Some(500.0),
            relation: Relation::SameWaterBody,
            observed_at: utc(2026, 11, 8),
            kind: ValueKind::Predicted,
            quality: Quality::Approved,
        }
    }

    fn builder(kind: StationKind) -> TideSeriesBuilder {
        TideSeriesBuilder {
            from: utc(2026, 11, 8),
            to: utc(2026, 11, 9),
            step: Duration::from_mins(10),
            base: base(kind),
        }
    }

    #[test]
    fn subordinate_stations_have_no_harmonics() {
        let empty = harcon(fixture!("coops-mdapi-8518919-harcon.json"), 0.3048);
        let table = datums("8518919", fixture!("coops-mdapi-8518919-datums.json"));
        assert_eq!(
            TideHarmonics::from_datum_table("8518919", empty, &table, VerticalDatum::Mllw),
            Err(TideError::NoConstituents("8518919".into()))
        );
        let constituents = battery().constituents;
        assert!(matches!(
            TideHarmonics::from_datum_table("8518919", constituents, &table, VerticalDatum::Mllw),
            Err(TideError::MissingDatum { .. })
        ));
        assert_eq!(
            builder(StationKind::TideSubordinate).harmonic(&battery()),
            Err(TideError::SubordinateStation)
        );
    }

    #[test]
    fn builder_marks_each_case() {
        let harmonic = builder(StationKind::TideHarmonic)
            .harmonic(&battery())
            .unwrap();
        assert_eq!(harmonic.source_kind, TideSourceKind::Computed);
        assert_eq!(harmonic.datum, VerticalDatum::Mllw);
        assert_eq!(harmonic.provenance.kind, ValueKind::Computed);
        assert_eq!(harmonic.provenance.source, SourceId::COOPS_HARMONICS);
        assert_eq!(harmonic.provenance.distance_m, Some(500.0));
        assert_eq!(harmonic.events.len(), 3);
        assert_eq!(harmonic.curve.as_ref().unwrap().samples().len(), 145);

        let sub = builder(StationKind::TideSubordinate);
        let computed = sub.subordinate(&battery(), &tarrytown_offsets()).unwrap();
        assert_eq!(computed.source_kind, TideSourceKind::SubordinateStation);
        assert_eq!(computed.provenance.kind, ValueKind::Computed);
        assert_eq!(computed.events[0].kind, TideKind::High);
        assert_eq!(computed.events.len(), 4);
        let curve = computed.curve.as_ref().unwrap();
        assert_eq!(curve.provenance().quality, Quality::Estimated);
        assert_eq!(curve.samples().len(), 145);

        let recorded = events(fixture!("coops-8518919-hilo-2026-11-08.json"));
        let passed = sub.coops(VerticalDatum::Mllw, recorded.clone(), None);
        assert_eq!(passed.source_kind, TideSourceKind::SubordinateStation);
        assert_eq!(passed.provenance.source, SourceId::COOPS);
        assert_eq!(passed.provenance.kind, ValueKind::Predicted);
        assert_eq!(passed.events.len(), 4);
        let curve = passed.curve.unwrap();
        assert_eq!(curve.provenance().quality, Quality::Estimated);

        let six = vec![Sample::new(utc(2026, 11, 8), 1.453)];
        let station =
            builder(StationKind::TideHarmonic).coops(VerticalDatum::Mllw, recorded, Some(six));
        assert_eq!(station.source_kind, TideSourceKind::HarmonicStation);
        assert_eq!(
            station.curve.unwrap().provenance().kind,
            ValueKind::Predicted
        );
    }

    #[test]
    fn rejects_bad_inputs() {
        let h = battery();
        let t = utc(2026, 11, 8);
        let minute = Duration::from_mins(1);
        assert_eq!(predict(&h, t, t, Duration::ZERO), Err(TideError::BadWindow));
        assert_eq!(
            predict(&h, UnixTime(t.0 + 1), t, minute),
            Err(TideError::BadWindow)
        );
        let unknown = TideHarmonics {
            constituents: vec![HarmonicConstituent {
                name: "XX9".into(),
                amplitude_m: 0.1,
                phase_gmt_deg: 0.0,
            }],
            ..battery()
        };
        assert_eq!(
            extremes(&unknown, t, t),
            Err(TideError::UnknownConstituent("XX9".into()))
        );
        let msl = TideHarmonics {
            datum: VerticalDatum::Msl,
            ..battery()
        };
        assert_eq!(
            builder(StationKind::TideSubordinate).subordinate(&msl, &tarrytown_offsets()),
            Err(TideError::SubordinateDatum(VerticalDatum::Msl))
        );
    }
}
