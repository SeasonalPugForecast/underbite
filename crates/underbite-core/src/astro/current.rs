//! Tidal current prediction from harmonic constituents.
//!
//! A harmonic current station publishes, per bin (depth), the amplitude and Greenwich phase of each
//! constituent along the major axis of the current ellipse, and the non-tidal mean speed along
//! that axis (CO-OPS `majorMeanSpeed`). The signed speed along the major axis, positive toward the
//! flood direction, is
//!
//! `v(t) = majorMeanSpeed + Σ f·A·cos(V0(t) + u − majorPhaseGMT)`
//!
//! with `f`, `u` and `V0` from [`crate::astro::constituents`]. CO-OPS publishes amplitudes in
//! cm/s; everything this module returns is in m/s. Only the major axis is predicted: the flood and
//! ebb directions are the station's mean directions (`meanFloodDir`, `meanEbbDir`).
//!
//! Events follow B. B. Parker, *Tidal Analysis and Prediction* (NOAA, 2007), §3.9.2: a slack is a
//! zero crossing of `v` (mean included, so slacks are not where the tidal part vanishes), and
//! between two slacks the strongest extremum is the maximum flood or ebb.
//!
//! Weak-and-variable stations (CO-OPS type `W`) have neither constituents nor offsets and get no
//! predictions.

use thiserror::Error;

use crate::astro::constituents::{AstronomicalArguments, Constituent, utc_year, year_start};
use crate::model::{CurrentEvent, CurrentKind, CurrentSeries};
use crate::provenance::Provenance;
use crate::series::{Sample, Series};
use crate::time::UnixTime;

/// Centimetres per metre: CO-OPS current constituents are in cm/s.
const CM_PER_M: f64 = 100.0;
/// Step of the scan for slacks and extrema, seconds.
const SCAN_S: i64 = 360;
/// Margin predicted on each side of a requested window so that the strongest extremum of a
/// flood or ebb that straddles the window's edge is chosen from the whole flood or ebb.
const PAD_S: i64 = 86_400;
/// Bisection stops when the bracket is this short, seconds.
const RESOLUTION_S: f64 = 0.5;

/// One constituent of a current station's major axis.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentConstituent {
    /// The CO-OPS name (`constituentName`), such as `M2`.
    pub name: String,
    /// Amplitude along the major axis, cm/s (`majorAmplitude`).
    pub major_amplitude_cms: f64,
    /// Phase lag referred to Greenwich, degrees (`majorPhaseGMT`, not the local `majorPhase`).
    pub major_phase_gmt_deg: f64,
}

/// The harmonic constants of one bin of a current station.
///
/// Bins differ in phase and amplitude: every constituent here must come from the same bin.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentHarmonics {
    /// Station id, such as `NYH1927`.
    pub station: String,
    /// The bin the constants belong to.
    pub bin: u16,
    /// Depth of the bin below chart datum, metres.
    pub depth_m: Option<f32>,
    /// Non-tidal mean speed along the major axis, cm/s (`majorMeanSpeed`), positive toward flood.
    pub mean_major_cms: f64,
    /// The constituents of the bin.
    pub constituents: Vec<CurrentConstituent>,
    /// Direction the flood sets toward, degrees true (`meanFloodDir`).
    pub flood_dir_deg: Option<f32>,
    /// Direction the ebb sets toward, degrees true (`meanEbbDir`).
    pub ebb_dir_deg: Option<f32>,
}

/// Harmonic constants that cannot be used for a prediction.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CurrentError {
    /// A constituent name outside the constituent table.
    #[error("unknown tidal constituent {0:?}")]
    UnknownConstituent(String),
    /// A constituent listed twice, as when entries of several bins are mixed.
    #[error("tidal constituent {0:?} listed twice (are bins mixed?)")]
    DuplicateConstituent(String),
}

/// A validated set of current harmonics, ready to predict.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentPredictor {
    station: String,
    bin: u16,
    depth_m: Option<f32>,
    flood_dir_deg: Option<f32>,
    ebb_dir_deg: Option<f32>,
    mean_cms: f64,
    /// Constituent, amplitude (cm/s) and Greenwich phase (degrees).
    terms: Vec<(Constituent, f64, f64)>,
}

impl CurrentPredictor {
    /// Resolves each constituent's name against the constituent table, whose speeds stand in for
    /// the `speed` the current `harcon` lacks.
    pub fn new(harmonics: &CurrentHarmonics) -> Result<Self, CurrentError> {
        let mut terms: Vec<(Constituent, f64, f64)> = Vec::new();
        for c in &harmonics.constituents {
            let constituent = Constituent::from_coops_name(&c.name)
                .ok_or_else(|| CurrentError::UnknownConstituent(c.name.clone()))?;
            if terms.iter().any(|&(k, _, _)| k == constituent) {
                return Err(CurrentError::DuplicateConstituent(c.name.clone()));
            }
            terms.push((constituent, c.major_amplitude_cms, c.major_phase_gmt_deg));
        }
        Ok(Self {
            station: harmonics.station.clone(),
            bin: harmonics.bin,
            depth_m: harmonics.depth_m,
            flood_dir_deg: harmonics.flood_dir_deg,
            ebb_dir_deg: harmonics.ebb_dir_deg,
            mean_cms: harmonics.mean_major_cms,
            terms,
        })
    }

    /// Signed speed along the major axis at one instant, m/s, positive toward flood.
    pub fn speed_ms(&self, time: UnixTime) -> f64 {
        Evaluator::new(self, time.0, time.0).value(time.0 as f64) / CM_PER_M
    }

    /// Signed major-axis speed in m/s every `step_s` seconds from `start` through `end`
    /// (a step below one second counts as one).
    pub fn predict(&self, start: UnixTime, end: UnixTime, step_s: i64) -> Vec<Sample<f32>> {
        let step = usize::try_from(step_s).unwrap_or(1).max(1);
        let evaluator = Evaluator::new(self, start.0, end.0);
        (start.0..=end.0)
            .step_by(step)
            .map(|t| Sample::new(UnixTime(t), (evaluator.value(t as f64) / CM_PER_M) as f32))
            .collect()
    }

    /// Maximum floods, maximum ebbs and slacks from `start` through `end`, in time order.
    ///
    /// Slack speeds are zero; maximum speeds are signed, in m/s.
    pub fn events(&self, start: UnixTime, end: UnixTime) -> Vec<CurrentEvent> {
        let (from, to) = (start.0 - PAD_S, end.0 + PAD_S);
        let evaluator = Evaluator::new(self, from, to);
        let mut turns = Vec::new();
        let mut a = from as f64;
        let (mut va, mut sa) = (evaluator.value(a), evaluator.slope(a));
        for i in 1..=(to - from + SCAN_S - 1) / SCAN_S {
            let b = (from + i * SCAN_S) as f64;
            let (vb, sb) = (evaluator.value(b), evaluator.slope(b));
            if (va < 0.0) != (vb < 0.0) {
                let t = bisect(a, b, |t| evaluator.value(t) < 0.0, va < 0.0);
                turns.push(Turn::Slack(t));
            }
            if (sa < 0.0) != (sb < 0.0) {
                let t = bisect(a, b, |t| evaluator.slope(t) < 0.0, sa < 0.0);
                let v = evaluator.value(t);
                // A rising-to-falling turn above zero is a flood maximum, a falling-to-rising
                // turn below zero an ebb maximum; the others are minima of a flood or ebb.
                if (sa >= 0.0 && v > 0.0) || (sa < 0.0 && v < 0.0) {
                    turns.push(Turn::Extremum(t, v));
                }
            }
            (a, va, sa) = (b, vb, sb);
        }
        turns.sort_by(|x, y| x.time().total_cmp(&y.time()));
        select_events(&turns)
            .into_iter()
            .filter(|e| (start..=end).contains(&e.time))
            .collect()
    }

    /// The station's predictions from `start` through `end`: events and a curve every `step_s`
    /// seconds, with the bin, its depth and the mean directions.
    pub fn series(
        &self,
        start: UnixTime,
        end: UnixTime,
        step_s: i64,
        provenance: Provenance,
    ) -> CurrentSeries {
        CurrentSeries {
            events: self.events(start, end),
            curve: Some(Series::new(
                provenance.clone(),
                self.predict(start, end, step_s),
            )),
            flood_dir_deg: self.flood_dir_deg,
            ebb_dir_deg: self.ebb_dir_deg,
            bin: Some(self.bin),
            depth_m: self.depth_m,
            observed: None,
            provenance,
        }
    }

    /// The station id.
    pub fn station(&self) -> &str {
        &self.station
    }
}

/// A candidate event found by the scan; times are unix seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Turn {
    Slack(f64),
    /// A flood maximum (positive speed) or an ebb maximum (negative), cm/s.
    Extremum(f64, f64),
}

impl Turn {
    fn time(self) -> f64 {
        match self {
            Self::Slack(t) | Self::Extremum(t, _) => t,
        }
    }
}

/// Keeps every slack and, between consecutive slacks, the strongest extremum.
fn select_events(turns: &[Turn]) -> Vec<CurrentEvent> {
    let mut events = Vec::new();
    let mut strongest: Option<(f64, f64)> = None;
    let flush = |strongest: &mut Option<(f64, f64)>, events: &mut Vec<CurrentEvent>| {
        if let Some((t, v)) = strongest.take() {
            events.push(CurrentEvent {
                time: UnixTime(t.round() as i64),
                kind: if v > 0.0 {
                    CurrentKind::MaxFlood
                } else {
                    CurrentKind::MaxEbb
                },
                speed_ms: (v / CM_PER_M) as f32,
            });
        }
    };
    for &turn in turns {
        match turn {
            Turn::Slack(t) => {
                flush(&mut strongest, &mut events);
                events.push(CurrentEvent {
                    time: UnixTime(t.round() as i64),
                    kind: CurrentKind::Slack,
                    speed_ms: 0.0,
                });
            }
            Turn::Extremum(t, v) => {
                if strongest.is_none_or(|(_, best)| v.abs() > best.abs()) {
                    strongest = Some((t, v));
                }
            }
        }
    }
    flush(&mut strongest, &mut events);
    events
}

/// The instant in `lo..hi` where `below` changes from `below_at_lo`, to [`RESOLUTION_S`].
fn bisect(mut lo: f64, mut hi: f64, below: impl Fn(f64) -> bool, below_at_lo: bool) -> f64 {
    while hi - lo > RESOLUTION_S {
        let mid = f64::midpoint(lo, hi);
        if below(mid) == below_at_lo {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    f64::midpoint(lo, hi)
}

/// One constituent for one year: `f·A`, the phase `V0 + u − κ` at the start of the year and the
/// speed.
#[derive(Clone, Copy, Debug)]
struct Term {
    amplitude_cms: f64,
    phase_deg: f64,
    speed_deg_h: f64,
}

/// The terms of every year that a time range touches. `f` and `u` are taken at the middle of
/// each year and `V0` at its start, advancing at the constituent's speed, as in NOAA practice.
#[derive(Debug)]
struct Evaluator {
    mean_cms: f64,
    first_year: i32,
    /// Start of each year, unix seconds, and its terms.
    years: Vec<(f64, Vec<Term>)>,
}

impl Evaluator {
    fn new(predictor: &CurrentPredictor, from: i64, to: i64) -> Self {
        let first_year = utc_year(UnixTime(from));
        let years = (first_year..=utc_year(UnixTime(to)).max(first_year))
            .map(|year| {
                let start = year_start(year);
                let at_start = AstronomicalArguments::at(start);
                let mid = AstronomicalArguments::mid_year(year);
                let terms = predictor
                    .terms
                    .iter()
                    .map(|&(c, amplitude, phase)| {
                        let node = c.node_correction(&mid);
                        Term {
                            amplitude_cms: node.f * amplitude,
                            phase_deg: c.equilibrium_argument(&at_start) + node.u - phase,
                            speed_deg_h: c.speed(),
                        }
                    })
                    .collect();
                (start.0 as f64, terms)
            })
            .collect();
        Self {
            mean_cms: predictor.mean_cms,
            first_year,
            years,
        }
    }

    /// The year's start and terms for an instant, clamped to the range built.
    fn year(&self, t: f64) -> (f64, &[Term]) {
        let index = utc_year(UnixTime(t.floor() as i64)) - self.first_year;
        let index = usize::try_from(index)
            .unwrap_or(0)
            .min(self.years.len() - 1);
        let (start, terms) = &self.years[index];
        (*start, terms)
    }

    /// Signed major-axis speed, cm/s.
    fn value(&self, t: f64) -> f64 {
        let (start, terms) = self.year(t);
        let hours = (t - start) / 3600.0;
        self.mean_cms
            + terms
                .iter()
                .map(|k| k.amplitude_cms * (k.phase_deg + k.speed_deg_h * hours).to_radians().cos())
                .sum::<f64>()
    }

    /// Rate of change of the speed, cm/s per hour.
    fn slope(&self, t: f64) -> f64 {
        let (start, terms) = self.year(t);
        let hours = (t - start) / 3600.0;
        -terms
            .iter()
            .map(|k| {
                k.amplitude_cms
                    * k.speed_deg_h.to_radians()
                    * (k.phase_deg + k.speed_deg_h * hours).to_radians().sin()
            })
            .sum::<f64>()
    }
}

#[cfg(test)]
mod tests {
    use whether_core::time::days_from_civil;

    use super::*;
    use crate::provenance::{Quality, Relation, SourceId, ValueKind};
    use crate::time::parse_coops;

    macro_rules! fixture {
        ($file:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/raw/coops/",
                $file
            ))
        };
    }

    const NYH1927_HARCON: &str = fixture!("coops-mdapi-NYH1927-harcon.json");
    const NYH1927_6MIN: &str =
        fixture!("coops-NYH1927-bin13-currents-predictions-6min-2026-11-08.json");
    const NYH1927_EVENTS: &str = fixture!("coops-NYH1927-bin13-max-slack-2026-11-08.json");
    const N03020_HARCON: &str = fixture!("coops-mdapi-n03020-harcon-bin7.json");
    const N03020_EVENTS: &str = fixture!("coops-n03020-bin7-max-slack-2026-11-08.json");

    /// Tolerances for currents: slack times, maximum times and maximum speeds.
    const SLACK_TOLERANCE_S: i64 = 10 * 60;
    const MAX_TOLERANCE_S: i64 = 20 * 60;
    const MAX_SPEED_TOLERANCE_MS: f32 = 0.10;

    fn utc(year: i32, month: u8, day: u8, hour: i64, minute: i64) -> UnixTime {
        UnixTime(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60)
    }

    /// The fixtures' week: 2026-11-08 00:00 through 2026-11-14 23:59 UTC.
    fn week() -> (UnixTime, UnixTime) {
        (utc(2026, 11, 8, 0, 0), utc(2026, 11, 14, 23, 59))
    }

    /// The constituents of one bin of a recorded `harcon` response.
    fn harmonics(json: &str, station: &str, bin: u16) -> CurrentHarmonics {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let entries: Vec<&serde_json::Value> = value["HarmonicConstituents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["binNbr"].as_u64() == Some(u64::from(bin)))
            .collect();
        CurrentHarmonics {
            station: station.into(),
            bin,
            depth_m: entries[0]["binDepth"].as_f64().map(|d| d as f32),
            mean_major_cms: entries[0]["majorMeanSpeed"].as_f64().unwrap(),
            constituents: entries
                .iter()
                .map(|c| CurrentConstituent {
                    name: c["constituentName"].as_str().unwrap().into(),
                    major_amplitude_cms: c["majorAmplitude"].as_f64().unwrap(),
                    major_phase_gmt_deg: c["majorPhaseGMT"].as_f64().unwrap(),
                })
                .collect(),
            flood_dir_deg: None,
            ebb_dir_deg: None,
        }
    }

    /// The points of a recorded `currents_predictions` response, speeds in m/s.
    fn coops_points(json: &str) -> Vec<(UnixTime, Option<CurrentKind>, f32)> {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        value["current_predictions"]["cp"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let kind = p["Type"].as_str().map(|t| match t {
                    "flood" => CurrentKind::MaxFlood,
                    "ebb" => CurrentKind::MaxEbb,
                    "slack" => CurrentKind::Slack,
                    other => panic!("unexpected type {other}"),
                });
                let speed = p["Velocity_Major"].as_f64().unwrap() / CM_PER_M;
                let time = parse_coops(p["Time"].as_str().unwrap()).unwrap();
                (time, kind, speed as f32)
            })
            .collect()
    }

    fn coops_events(json: &str) -> Vec<CurrentEvent> {
        coops_points(json)
            .into_iter()
            .map(|(time, kind, speed_ms)| CurrentEvent {
                time,
                kind: kind.unwrap(),
                speed_ms,
            })
            .collect()
    }

    /// Asserts the same sequence of kinds within the tolerances and prints the worst slack time,
    /// maximum time and maximum speed errors.
    fn compare_events(label: &str, ours: &[CurrentEvent], theirs: &[CurrentEvent]) {
        assert_eq!(ours.len(), theirs.len(), "{label}: event count");
        let (mut slack, mut max_time, mut max_speed) = (0_i64, 0_i64, 0_f32);
        for (a, b) in ours.iter().zip(theirs) {
            assert_eq!(a.kind, b.kind, "{label}: kind at {:?}", b.time);
            let dt = (a.time.0 - b.time.0).abs();
            if a.kind == CurrentKind::Slack {
                slack = slack.max(dt);
            } else {
                max_time = max_time.max(dt);
                max_speed = max_speed.max((a.speed_ms - b.speed_ms).abs());
            }
        }
        eprintln!(
            "{label}: {} events, worst slack {slack} s, worst maximum {max_time} s and {:.1} cm/s",
            ours.len(),
            max_speed * 100.0
        );
        assert!(
            slack <= SLACK_TOLERANCE_S,
            "{label}: slack off by {slack} s"
        );
        assert!(
            max_time <= MAX_TOLERANCE_S,
            "{label}: maximum off by {max_time} s"
        );
        assert!(
            max_speed <= MAX_SPEED_TOLERANCE_MS,
            "{label}: maximum off by {max_speed} m/s"
        );
    }

    #[test]
    fn nyh1927_bin13_curve_matches_coops_six_minute_predictions() {
        let predictor = CurrentPredictor::new(&harmonics(NYH1927_HARCON, "NYH1927", 13)).unwrap();
        let theirs = coops_points(NYH1927_6MIN);
        assert_eq!(theirs.len(), 1680);
        let (start, end) = (theirs[0].0, theirs[theirs.len() - 1].0);
        let ours = predictor.predict(start, end, 360);
        assert_eq!(ours.len(), theirs.len());
        let mut sum = 0.0_f64;
        let mut worst = 0.0_f64;
        for (a, (time, _, speed)) in ours.iter().zip(&theirs) {
            assert_eq!(a.time, *time);
            let diff = f64::from(a.value - speed) * CM_PER_M;
            sum += diff * diff;
            worst = worst.max(diff.abs());
        }
        let rms = (sum / ours.len() as f64).sqrt();
        eprintln!("NYH1927 bin 13: RMS {rms:.2} cm/s, worst {worst:.2} cm/s");
        assert!(rms <= 8.0, "RMS {rms} cm/s");
    }

    #[test]
    fn nyh1927_bin13_events_match_coops_max_slack() {
        let predictor = CurrentPredictor::new(&harmonics(NYH1927_HARCON, "NYH1927", 13)).unwrap();
        let (start, end) = week();
        let theirs = coops_events(NYH1927_EVENTS);
        assert_eq!(theirs.len(), 55);
        compare_events("NYH1927 bin 13", &predictor.events(start, end), &theirs);
    }

    #[test]
    fn n03020_bin7_events_match_coops_max_slack() {
        let predictor = CurrentPredictor::new(&harmonics(N03020_HARCON, "n03020", 7)).unwrap();
        let (start, end) = week();
        let theirs = coops_events(N03020_EVENTS);
        assert_eq!(theirs.len(), 54);
        compare_events("n03020 bin 7", &predictor.events(start, end), &theirs);
    }

    fn pure(mean_cms: f64, constituents: &[(&str, f64, f64)]) -> CurrentHarmonics {
        CurrentHarmonics {
            station: "TEST".into(),
            bin: 1,
            depth_m: Some(2.0),
            mean_major_cms: mean_cms,
            constituents: constituents
                .iter()
                .map(|&(name, a, k)| CurrentConstituent {
                    name: name.into(),
                    major_amplitude_cms: a,
                    major_phase_gmt_deg: k,
                })
                .collect(),
            flood_dir_deg: Some(11.0),
            ebb_dir_deg: Some(183.0),
        }
    }

    #[test]
    fn unknown_and_repeated_constituents_are_rejected() {
        assert_eq!(
            CurrentPredictor::new(&pure(0.0, &[("M2", 1.0, 0.0), ("Z9", 1.0, 0.0)])),
            Err(CurrentError::UnknownConstituent("Z9".into()))
        );
        assert_eq!(
            CurrentPredictor::new(&pure(0.0, &[("M2", 1.0, 0.0), ("M2", 2.0, 0.0)])),
            Err(CurrentError::DuplicateConstituent("M2".into()))
        );
    }

    #[test]
    fn slacks_are_zero_crossings_of_the_speed_including_its_mean() {
        // With a mean of -20 cm/s on a 100 cm/s S2 (f = 1, u = 0), the ebb lasts longer than
        // the flood and the slacks sit where the tidal part is +20 cm/s.
        let predictor = CurrentPredictor::new(&pure(-20.0, &[("S2", 100.0, 40.0)])).unwrap();
        let (start, end) = (utc(2026, 3, 2, 0, 0), utc(2026, 3, 3, 0, 0));
        let events = predictor.events(start, end);
        assert_eq!(events.len(), 8, "{events:?}");
        for e in &events {
            let v = predictor.speed_ms(e.time);
            match e.kind {
                CurrentKind::Slack => assert!(v.abs() < 0.002, "{e:?} at {v}"),
                CurrentKind::MaxFlood => assert!((f64::from(e.speed_ms) - 0.8).abs() < 1e-4),
                CurrentKind::MaxEbb => assert!((f64::from(e.speed_ms) + 1.2).abs() < 1e-4),
            }
        }
        let slacks: Vec<&CurrentEvent> = events
            .iter()
            .filter(|e| e.kind == CurrentKind::Slack)
            .collect();
        let flood = slacks
            .windows(2)
            .find(|w| predictor.speed_ms(UnixTime(w[0].time.0 + 60)) > 0.0)
            .map(|w| w[1].time.0 - w[0].time.0)
            .unwrap();
        // acos(0.2) on either side of the crest: 2 · 78.463° at 30°/h is 5.231 h.
        assert!((flood - 18_831).abs() <= 2, "{flood}");
    }

    #[test]
    fn the_strongest_extremum_between_two_slacks_is_the_maximum() {
        let turns = [
            Turn::Slack(0.0),
            Turn::Extremum(3_600.0, 80.0),
            Turn::Extremum(10_800.0, 95.0),
            Turn::Slack(18_000.0),
            Turn::Extremum(21_600.0, -120.0),
            Turn::Extremum(25_200.0, -60.0),
            Turn::Slack(36_000.0),
        ];
        let summary: Vec<(i64, CurrentKind, f32)> = select_events(&turns)
            .iter()
            .map(|e| (e.time.0, e.kind, e.speed_ms))
            .collect();
        assert_eq!(
            summary,
            [
                (0, CurrentKind::Slack, 0.0),
                (10_800, CurrentKind::MaxFlood, 0.95),
                (18_000, CurrentKind::Slack, 0.0),
                (21_600, CurrentKind::MaxEbb, -1.2),
                (36_000, CurrentKind::Slack, 0.0),
            ]
        );
    }

    fn provenance() -> Provenance {
        Provenance {
            source: SourceId::COOPS_HARMONICS,
            station: None,
            distance_m: None,
            relation: Relation::Nearby,
            observed_at: utc(2026, 11, 8, 0, 0),
            kind: ValueKind::Predicted,
            quality: Quality::Approved,
        }
    }

    #[test]
    fn series_carry_events_curve_bin_and_directions() {
        // The window crosses a year boundary, where f, u and V0 switch years.
        let predictor = CurrentPredictor::new(&pure(-20.0, &[("S2", 100.0, 0.0)])).unwrap();
        let (start, end) = (utc(2026, 12, 31, 18, 0), utc(2027, 1, 1, 6, 0));
        let series = predictor.series(start, end, 600, provenance());
        let curve = series.curve.as_ref().unwrap();
        assert_eq!(curve.samples().len(), 73);
        assert_eq!(curve.provenance(), &provenance());
        assert!(curve.samples().iter().all(|s| s.value.abs() <= 1.21));
        assert!(series.events.len() >= 4, "{:?}", series.events);
        assert_eq!((series.bin, series.depth_m), (Some(1), Some(2.0)));
        assert_eq!(
            (series.flood_dir_deg, series.ebb_dir_deg),
            (Some(11.0), Some(183.0))
        );
        assert_eq!(predictor.station(), "TEST");
    }
}
