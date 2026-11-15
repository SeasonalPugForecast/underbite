//! Tide stage: rising, falling, slack, and time to the next turn.

use std::time::Duration;

use crate::model::{TideEvent, TideKind, TideSeries, TideSourceKind};
use crate::provenance::Quality;
use crate::time::UnixTime;

/// Half the span of the central difference that gives the rate.
const RATE_HALF_SPAN_S: i64 = 360;
/// Widest gap between curve samples the rate is taken across.
const RATE_MAX_GAP: Duration = Duration::from_hours(3);

/// Which way the water level is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TideDirection {
    /// Between a low and the next high (the incoming tide).
    Rising,
    /// Between a high and the next low (the outgoing tide).
    Falling,
}

/// How much to trust a rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateBasis {
    /// From a curve of harmonic predictions.
    Predicted,
    /// From an approximate cosine curve between events (a subordinate station, or a station
    /// served without a curve): indicative only.
    Approximate,
}

/// The rate of change of the water level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TideRate {
    /// Metres per hour, positive while rising.
    pub m_per_h: f32,
    pub basis: RateBasis,
}

/// The tide at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TideStage {
    pub direction: TideDirection,
    /// The last extreme at or before the instant.
    pub previous: TideEvent,
    /// The first extreme after the instant.
    pub next: TideEvent,
    /// How far through the interval between the extremes, 0 to 1, by time.
    pub fraction: f32,
    /// Seconds since the previous extreme.
    pub elapsed_s: i64,
    /// Seconds until the next extreme.
    pub to_next_s: i64,
    /// Absent without a curve and for a modelled mean.
    pub rate: Option<TideRate>,
    /// True when the times are a model's: direction and rough times only, no heights.
    pub approximate_times: bool,
}

impl TideStage {
    /// Hours until the next extreme.
    pub fn hours_to_next(&self) -> f64 {
        self.to_next_s as f64 / 3_600.0
    }

    /// Whether the tide is going `direction` and the next extreme is at most `hours` away: "the
    /// last two hours of the incoming" is `in_last_hours(TideDirection::Rising, 2.0)`.
    pub fn in_last_hours(&self, direction: TideDirection, hours: f64) -> bool {
        self.direction == direction && self.to_next_s as f64 <= hours * 3_600.0
    }

    /// Whether the tide is going `direction` and the previous extreme was at most `hours` ago:
    /// "the first hour of the ebb" is `in_first_hours(TideDirection::Falling, 1.0)`.
    pub fn in_first_hours(&self, direction: TideDirection, hours: f64) -> bool {
        self.direction == direction && self.elapsed_s as f64 <= hours * 3_600.0
    }

    /// Whether an extreme (slack water at the turn) is within `minutes` either side.
    pub fn near_turn(&self, minutes: f64) -> bool {
        let limit = minutes * 60.0;
        (self.elapsed_s as f64) <= limit || (self.to_next_s as f64) <= limit
    }
}

/// The stage of the tide at `now`, from the series' high and low events; `None` when `now` is not
/// between two events of opposite kinds (the series does not cover it).
///
/// At an extreme exactly, the stage has already turned. The rate is the slope of the curve over
/// six minutes either side of `now`, absent when the series has no curve or is a `ModelMean`.
pub fn tide_stage(series: &TideSeries, now: UnixTime) -> Option<TideStage> {
    let previous = series
        .events
        .iter()
        .filter(|e| e.time <= now)
        .max_by_key(|e| e.time)?;
    let next = series
        .events
        .iter()
        .filter(|e| e.time > now)
        .min_by_key(|e| e.time)?;
    if previous.kind == next.kind {
        return None;
    }
    let direction = match previous.kind {
        TideKind::Low => TideDirection::Rising,
        TideKind::High => TideDirection::Falling,
    };
    let span = next.time.0 - previous.time.0;
    let elapsed_s = now.0 - previous.time.0;
    let fraction = (elapsed_s as f64 / span as f64).clamp(0.0, 1.0) as f32;
    let model = series.source_kind == TideSourceKind::ModelMean;
    Some(TideStage {
        direction,
        previous: *previous,
        next: *next,
        fraction,
        elapsed_s,
        to_next_s: next.time.0 - now.0,
        rate: if model { None } else { rate(series, now) },
        approximate_times: model,
    })
}

fn rate(series: &TideSeries, now: UnixTime) -> Option<TideRate> {
    let curve = series.curve.as_ref()?;
    let before = curve.at(UnixTime(now.0 - RATE_HALF_SPAN_S), RATE_MAX_GAP)?;
    let after = curve.at(UnixTime(now.0 + RATE_HALF_SPAN_S), RATE_MAX_GAP)?;
    let hours = (2 * RATE_HALF_SPAN_S) as f64 / 3_600.0;
    Some(TideRate {
        m_per_h: ((f64::from(after) - f64::from(before)) / hours) as f32,
        // A cosine curve interpolated between events is marked Estimated whatever the station.
        basis: if series.source_kind == TideSourceKind::SubordinateStation
            || curve.provenance().quality == Quality::Estimated
        {
            RateBasis::Approximate
        } else {
            RateBasis::Predicted
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datum::VerticalDatum;
    use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
    use crate::series::{Sample, Series};
    use crate::time::parse_coops;

    fn provenance() -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: None,
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(0),
            kind: ValueKind::Predicted,
            quality: Quality::Approved,
        }
    }

    fn fixture(name: &str) -> Vec<(UnixTime, f32, Option<char>)> {
        let path = format!(
            "{}/../../tests/fixtures/raw/coops/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["predictions"]
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

    fn battery(source_kind: TideSourceKind, with_curve: bool) -> TideSeries {
        let events = fixture("coops-8518750-hilo-2026-11-08.json")
            .into_iter()
            .map(|(time, v, kind)| TideEvent {
                time,
                kind: if kind == Some('H') {
                    TideKind::High
                } else {
                    TideKind::Low
                },
                height_m: Some(v),
            })
            .collect();
        let curve = with_curve.then(|| {
            Series::new(
                provenance(),
                fixture("coops-8518750-predictions-6min-2026-11-08.json")
                    .into_iter()
                    .map(|(t, v, _)| Sample::new(t, v))
                    .collect(),
            )
        });
        TideSeries {
            source_kind,
            datum: VerticalDatum::Mllw,
            events,
            curve,
            observed: None,
            provenance: provenance(),
        }
    }

    fn at(s: &str) -> UnixTime {
        parse_coops(s).unwrap()
    }

    #[test]
    fn rising_between_the_morning_low_and_high() {
        let series = battery(TideSourceKind::HarmonicStation, true);
        let st = tide_stage(&series, at("2026-11-08 09:00")).unwrap();
        assert_eq!(st.direction, TideDirection::Rising);
        assert_eq!(st.previous.kind, TideKind::Low);
        assert_eq!(st.previous.time, at("2026-11-08 05:59"));
        assert_eq!(st.next.kind, TideKind::High);
        assert_eq!(st.next.time, at("2026-11-08 12:07"));
        assert!((st.fraction - 0.49).abs() < 0.01, "{}", st.fraction);
        assert_eq!(st.to_next_s, 3 * 3_600 + 7 * 60);
        let rate = st.rate.unwrap();
        assert_eq!(rate.basis, RateBasis::Predicted);
        // The range of 1.7 m over six hours: a rate near the middle of the rise.
        assert!((0.2..0.6).contains(&rate.m_per_h), "{}", rate.m_per_h);
        assert!(!st.approximate_times);
    }

    #[test]
    fn falling_has_a_negative_rate() {
        let series = battery(TideSourceKind::HarmonicStation, true);
        let st = tide_stage(&series, at("2026-11-08 15:30")).unwrap();
        assert_eq!(st.direction, TideDirection::Falling);
        assert!(st.rate.unwrap().m_per_h < -0.2);
    }

    #[test]
    fn the_stage_turns_at_the_extreme_instant() {
        let series = battery(TideSourceKind::HarmonicStation, true);
        let at_high = tide_stage(&series, at("2026-11-08 12:07")).unwrap();
        assert_eq!(at_high.direction, TideDirection::Falling);
        assert_eq!(at_high.fraction, 0.0);
        assert_eq!(at_high.elapsed_s, 0);
        let before = tide_stage(&series, UnixTime(at("2026-11-08 12:07").0 - 1)).unwrap();
        assert_eq!(before.direction, TideDirection::Rising);
        assert!(before.fraction > 0.999);
        // Slack: the rate is near zero.
        assert!(at_high.rate.unwrap().m_per_h.abs() < 0.05);
    }

    #[test]
    fn helpers_for_windows() {
        let series = battery(TideSourceKind::HarmonicStation, false);
        let st = tide_stage(&series, at("2026-11-08 10:30")).unwrap();
        assert!(st.in_last_hours(TideDirection::Rising, 2.0));
        assert!(!st.in_last_hours(TideDirection::Rising, 1.0));
        assert!(!st.in_last_hours(TideDirection::Falling, 6.0));
        assert!(st.in_first_hours(TideDirection::Rising, 5.0));
        assert!(!st.in_first_hours(TideDirection::Rising, 4.0));
        assert!((st.hours_to_next() - 1.6166).abs() < 0.01);
        assert!(!st.near_turn(30.0));
        assert!(st.near_turn(100.0));
        assert_eq!(st.rate, None);
    }

    #[test]
    fn subordinate_rates_are_labelled_approximate() {
        let series = battery(TideSourceKind::SubordinateStation, true);
        let st = tide_stage(&series, at("2026-11-08 09:00")).unwrap();
        assert_eq!(st.rate.unwrap().basis, RateBasis::Approximate);
    }

    #[test]
    fn an_interpolated_curve_gives_an_approximate_rate_whatever_the_station() {
        let mut series = battery(TideSourceKind::HarmonicStation, true);
        let estimated = Provenance {
            kind: ValueKind::Computed,
            quality: Quality::Estimated,
            ..provenance()
        };
        let curve = series.curve.take().unwrap();
        series.curve = Some(Series::new(estimated, curve.samples().to_vec()));
        let st = tide_stage(&series, at("2026-11-08 09:00")).unwrap();
        assert_eq!(st.rate.unwrap().basis, RateBasis::Approximate);
    }

    #[test]
    fn a_model_mean_has_direction_and_times_only() {
        let mut series = battery(TideSourceKind::ModelMean, true);
        for e in &mut series.events {
            e.height_m = None;
        }
        let st = tide_stage(&series, at("2026-11-08 09:00")).unwrap();
        assert_eq!(st.direction, TideDirection::Rising);
        assert_eq!(st.rate, None);
        assert_eq!(st.previous.height_m, None);
        assert!(st.approximate_times);
    }

    #[test]
    fn outside_the_events_there_is_no_stage() {
        let series = battery(TideSourceKind::HarmonicStation, false);
        assert_eq!(tide_stage(&series, at("2026-11-08 05:00")), None);
        assert_eq!(tide_stage(&series, at("2026-11-15 00:00")), None);
        let mut gap = series.clone();
        gap.events.retain(|e| e.kind == TideKind::High);
        assert_eq!(tide_stage(&gap, at("2026-11-08 15:00")), None);
    }
}
