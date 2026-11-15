//! Tide prediction from harmonic constituents: heights, highs and lows.
//!
//! The height above datum `D` of a harmonic station is
//! `h(t) = Z0 + Σ f·A·cos(V0(t) + u − g)`, with `A` and `g` the amplitude and Greenwich phase lag
//! of each constituent (CO-OPS `harcon`, `phase_GMT`), `Z0 = MSL − D` from the station's datum
//! table, and `f`, `u` the node factor and phase of the year. As in NOAA practice, `f` and `u`
//! are taken at the middle of each UTC year and `V0` at its start, advancing at the
//! constituent's speed.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::astro::constituents::{
    AstronomicalArguments, Constituent, utc_year, year_start, yearly_node_corrections,
};
use crate::datum::{DatumTable, VerticalDatum};
use crate::model::{TideEvent, TideKind};
use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
use crate::series::{Sample, Series};
use crate::time::UnixTime;

/// Spacing of the grid on which turning points are bracketed, seconds.
const SEARCH_STEP: i64 = 360;
/// Bisection steps: 360 s / 2^12 is below 0.1 s.
const BISECTIONS: u32 = 12;

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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;
    use whether_core::time::days_from_civil;

    use super::*;

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
    }
}
