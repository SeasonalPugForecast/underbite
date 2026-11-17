//! Tide and current tables for any dates, computed from harmonic constants.
//!
//! These package the predictors of [`crate::astro::tide`] and [`crate::astro::current`] for a
//! window the caller names: a trip day, a week, or "tides for any date". Heights stay above the
//! harmonics' datum (the series says which), speeds are signed metres per second along the
//! major axis, and every table is `Computed` from `coops-harmonics`, never passed off as the
//! agency's published predictions.

use std::time::Duration;

use thiserror::Error;

use crate::astro::current::{
    CurrentError, CurrentHarmonics, CurrentPredictor, SubordinateCurrentOffsets,
};
use crate::astro::tide::{SubordinateTideOffsets, TideError, TideHarmonics, TideSeriesBuilder};
use crate::model::{CurrentSeries, TideSeries};
use crate::provenance::{Provenance, Quality, SourceId, ValueKind};
use crate::time::UnixTime;

/// The most curve points one table holds (a year at ten minutes is about 53,000).
pub const MAX_CURVE_POINTS: i64 = 400_000;

/// Why a table could not be computed.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum TableError {
    /// The tide predictor refused the constants or the window.
    #[error(transparent)]
    Tide(#[from] TideError),
    /// The current predictor refused the constants or the offsets.
    #[error(transparent)]
    Current(#[from] CurrentError),
    /// The step is zero, the window ends before it starts, or the curve would be too long.
    #[error("empty or reversed window, zero step, or more than {MAX_CURVE_POINTS} curve points")]
    BadWindow,
}

/// What a tide table is computed from.
#[derive(Clone, Copy, Debug)]
pub enum TideStation<'a> {
    /// A harmonic (reference) station.
    Harmonic(&'a TideHarmonics),
    /// A subordinate station: its reference station's harmonics and its own offsets. Heights
    /// are above MLLW, and the table holds highs and lows only, as CO-OPS publishes.
    Subordinate {
        /// The reference station's constants, heights above MLLW.
        reference: &'a TideHarmonics,
        /// The subordinate station's offsets.
        offsets: &'a SubordinateTideOffsets,
    },
}

/// What a current table is computed from.
#[derive(Clone, Copy, Debug)]
pub enum CurrentStation<'a> {
    /// A harmonic station and bin. The table's flood and ebb directions are the constants'
    /// own (`flood_dir_deg`, `ebb_dir_deg`), which the caller fills from the station's bin
    /// metadata when it has it.
    Harmonic(&'a CurrentHarmonics),
    /// A subordinate station: its reference bin's constants and its own offsets (events only,
    /// with the offsets' directions).
    Subordinate {
        /// The reference station and bin's constants.
        reference: &'a CurrentHarmonics,
        /// The subordinate station's offsets.
        offsets: &'a SubordinateCurrentOffsets,
    },
}

/// The provenance of a computed table: `base` says where the station is and how it relates to
/// the spot; the source, kind and quality are the computed ones.
fn computed(base: Provenance, quality: Quality) -> Provenance {
    Provenance {
        source: SourceId::COOPS_HARMONICS,
        kind: ValueKind::Computed,
        quality,
        ..base
    }
}

fn check(from: UnixTime, to: UnixTime, step: Duration) -> Result<i64, TableError> {
    let step = i64::try_from(step.as_secs()).map_err(|_| TableError::BadWindow)?;
    if step <= 0 || from > to || (to.0 - from.0) / step >= MAX_CURVE_POINTS {
        return Err(TableError::BadWindow);
    }
    Ok(step)
}

/// High and low tides from `from` to `to` (inclusive) and, for a harmonic station, a curve
/// every `step`.
///
/// A subordinate station has no curve: CO-OPS publishes its highs and lows only, and the table
/// does the same rather than draw one.
pub fn tide_table(
    station: TideStation<'_>,
    base: Provenance,
    from: UnixTime,
    to: UnixTime,
    step: Duration,
) -> Result<TideSeries, TableError> {
    check(from, to, step)?;
    let builder = TideSeriesBuilder {
        from,
        to,
        step,
        base,
    };
    Ok(match station {
        TideStation::Harmonic(h) => builder.harmonic(h)?,
        TideStation::Subordinate { reference, offsets } => {
            let mut series = builder.subordinate(reference, offsets)?;
            series.curve = None;
            series
        }
    })
}

/// Maximum floods, maximum ebbs and slacks from `from` to `to` (inclusive) and, for a harmonic
/// station, a signed speed curve every `step`.
pub fn current_table(
    station: CurrentStation<'_>,
    base: Provenance,
    from: UnixTime,
    to: UnixTime,
    step: Duration,
) -> Result<CurrentSeries, TableError> {
    let step_s = check(from, to, step)?;
    let provenance = computed(base, Quality::Approved);
    Ok(match station {
        CurrentStation::Harmonic(h) => {
            CurrentPredictor::new(h)?.series(from, to, step_s, provenance)
        }
        CurrentStation::Subordinate { reference, offsets } => CurrentSeries {
            events: CurrentPredictor::new(reference)?.subordinate_events(offsets, from, to)?,
            curve: None,
            flood_dir_deg: offsets.flood_dir_deg,
            ebb_dir_deg: offsets.ebb_dir_deg,
            bin: offsets.bin,
            depth_m: None,
            observed: None,
            provenance,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;
    use whether_core::time::days_from_civil;

    use super::*;
    use crate::astro::current::CurrentConstituent;
    use crate::astro::tide::{HarmonicConstituent, HeightAdjustment};
    use crate::datum::{DatumTable, VerticalDatum};
    use crate::model::{CurrentEvent, CurrentKind, TideEvent, TideKind, TideSourceKind};
    use crate::provenance::{Relation, StationKind, StationRef};
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

    const MINUTE: i64 = 60;

    fn utc(y: i32, m: u8, d: u8) -> UnixTime {
        UnixTime(days_from_civil(y, m, d) * 86_400)
    }

    fn coops_time(s: &str) -> UnixTime {
        let n = |r: std::ops::Range<usize>| s[r].parse::<i64>().unwrap();
        let day = days_from_civil(n(0..4) as i32, n(5..7) as u8, n(8..10) as u8);
        UnixTime(day * 86_400 + n(11..13) * 3600 + n(14..16) * MINUTE)
    }

    fn battery() -> TideHarmonics {
        let v: Value =
            serde_json::from_str(fixture!("coops-mdapi-8518750-harcon-metric.json")).unwrap();
        let constituents = v["HarmonicConstituents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| HarmonicConstituent {
                name: c["name"].as_str().unwrap().to_owned(),
                amplitude_m: c["amplitude"].as_f64().unwrap() as f32,
                phase_gmt_deg: c["phase_GMT"].as_f64().unwrap() as f32,
            })
            .collect();
        let d: Value =
            serde_json::from_str(fixture!("coops-mdapi-8518750-datums-metric.json")).unwrap();
        let values: BTreeMap<String, f32> = d["datums"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| Some((d["name"].as_str()?.to_owned(), d["value"].as_f64()? as f32)))
            .collect();
        TideHarmonics::from_datum_table(
            "8518750",
            constituents,
            &DatumTable::new("8518750", values),
            VerticalDatum::Mllw,
        )
        .unwrap()
    }

    fn tarrytown_offsets() -> SubordinateTideOffsets {
        let v: Value =
            serde_json::from_str(fixture!("coops-mdapi-8518919-tidepredoffsets.json")).unwrap();
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

    fn hilo(json: &str) -> Vec<TideEvent> {
        let v: Value = serde_json::from_str(json).unwrap();
        v["predictions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| TideEvent {
                time: coops_time(p["t"].as_str().unwrap()),
                kind: match p["type"].as_str().unwrap() {
                    "H" => TideKind::High,
                    _ => TideKind::Low,
                },
                height_m: Some(p["v"].as_str().unwrap().parse().unwrap()),
            })
            .collect()
    }

    fn base(id: &str, kind: StationKind) -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: Some(StationRef {
                source: SourceId::COOPS,
                id: id.into(),
                name: id.into(),
                kind,
                latitude: 40.7,
                longitude: -74.0,
                bin: None,
                reference_id: None,
            }),
            distance_m: None,
            relation: Relation::SameWaterBody,
            observed_at: utc(2026, 11, 8),
            kind: ValueKind::Predicted,
            quality: Quality::Provisional,
        }
    }

    const WEEK: i64 = 7 * 86_400;

    fn assert_events(label: &str, ours: &[TideEvent], theirs: &[TideEvent]) {
        assert_eq!(ours.len(), theirs.len(), "{label}: count");
        for (a, b) in ours.iter().zip(theirs) {
            assert_eq!(a.kind, b.kind, "{label}: kind at {:?}", b.time);
            let dt = (a.time.0 - b.time.0).abs() / MINUTE;
            let dh = (a.height_m.unwrap() - b.height_m.unwrap()).abs();
            assert!(
                dt <= 6 && dh <= 0.04,
                "{label}: {dt} min, {dh} m at {:?}",
                b.time
            );
        }
    }

    #[test]
    fn the_battery_table_matches_the_published_week() {
        let (from, to) = (utc(2026, 11, 8), UnixTime(utc(2026, 11, 8).0 + WEEK - 1));
        let table = tide_table(
            TideStation::Harmonic(&battery()),
            base("8518750", StationKind::TideHarmonic),
            from,
            UnixTime(to.0 + 6 * MINUTE),
            Duration::from_mins(6),
        )
        .unwrap();
        assert_eq!(table.datum, VerticalDatum::Mllw);
        assert_eq!(table.source_kind, TideSourceKind::Computed);
        assert_eq!(table.provenance.source, SourceId::COOPS_HARMONICS);
        assert_eq!(table.provenance.kind, ValueKind::Computed);
        let theirs = hilo(fixture!("coops-8518750-hilo-2026-11-08.json"));
        assert_eq!(theirs.len(), 27);
        assert_events("8518750", &table.events, &theirs);
        let curve = table.curve.unwrap();
        assert_eq!(curve.samples().len(), 1681);
    }

    #[test]
    fn tarrytown_from_the_battery_has_events_and_no_curve() {
        let (from, to) = (utc(2026, 11, 8), utc(2026, 11, 15));
        let reference = battery();
        let offsets = tarrytown_offsets();
        let table = tide_table(
            TideStation::Subordinate {
                reference: &reference,
                offsets: &offsets,
            },
            base("8518919", StationKind::TideSubordinate),
            from,
            UnixTime(to.0 - 1),
            Duration::from_mins(6),
        )
        .unwrap();
        assert_eq!(table.source_kind, TideSourceKind::SubordinateStation);
        assert_eq!(table.datum, VerticalDatum::Mllw);
        assert!(table.curve.is_none());
        let theirs: Vec<_> = hilo(fixture!("coops-8518919-hilo-2026-11-08.json"))
            .into_iter()
            .filter(|e| e.time >= from && e.time < to)
            .collect();
        assert_events("8518919", &table.events, &theirs);
    }

    fn nyh1927() -> CurrentHarmonics {
        let v: Value = serde_json::from_str(fixture!("coops-mdapi-NYH1927-harcon.json")).unwrap();
        let rows: Vec<&Value> = v["HarmonicConstituents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["binNbr"].as_u64() == Some(13))
            .collect();
        CurrentHarmonics {
            station: "NYH1927".into(),
            bin: 13,
            depth_m: rows[0]["binDepth"].as_f64().map(|d| d as f32),
            mean_major_cms: rows[0]["majorMeanSpeed"].as_f64().unwrap(),
            constituents: rows
                .iter()
                .map(|c| CurrentConstituent {
                    name: c["constituentName"].as_str().unwrap().into(),
                    major_amplitude_cms: c["majorAmplitude"].as_f64().unwrap(),
                    major_phase_gmt_deg: c["majorPhaseGMT"].as_f64().unwrap(),
                })
                .collect(),
            flood_dir_deg: Some(11.0),
            ebb_dir_deg: Some(183.0),
        }
    }

    fn nyh_events() -> Vec<CurrentEvent> {
        let v: Value =
            serde_json::from_str(fixture!("coops-NYH1927-bin13-max-slack-2026-11-08.json"))
                .unwrap();
        v["current_predictions"]["cp"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| CurrentEvent {
                time: parse_coops(p["Time"].as_str().unwrap()).unwrap(),
                kind: match p["Type"].as_str().unwrap() {
                    "flood" => CurrentKind::MaxFlood,
                    "ebb" => CurrentKind::MaxEbb,
                    _ => CurrentKind::Slack,
                },
                speed_ms: (p["Velocity_Major"].as_f64().unwrap() / 100.0) as f32,
            })
            .collect()
    }

    #[test]
    fn the_nyh1927_bin_13_table_matches_the_published_events() {
        let from = utc(2026, 11, 8);
        let to = UnixTime(from.0 + WEEK - 60);
        let table = current_table(
            CurrentStation::Harmonic(&nyh1927()),
            base("NYH1927", StationKind::CurrentHarmonic),
            from,
            to,
            Duration::from_mins(6),
        )
        .unwrap();
        assert_eq!(table.provenance.kind, ValueKind::Computed);
        assert_eq!(table.provenance.source, SourceId::COOPS_HARMONICS);
        assert_eq!(table.bin, Some(13));
        assert_eq!(
            (table.flood_dir_deg, table.ebb_dir_deg),
            (Some(11.0), Some(183.0))
        );
        let theirs = nyh_events();
        assert_eq!(theirs.len(), 55);
        assert_eq!(table.events.len(), theirs.len());
        for (a, b) in table.events.iter().zip(&theirs) {
            assert_eq!(a.kind, b.kind);
            let dt = (a.time.0 - b.time.0).abs();
            if a.kind == CurrentKind::Slack {
                assert!(dt <= 10 * 60, "slack off by {dt} s");
            } else {
                assert!(dt <= 20 * 60, "maximum off by {dt} s");
                assert!((a.speed_ms - b.speed_ms).abs() <= 0.10);
            }
        }
        assert_eq!(table.curve.unwrap().samples().len(), 1680);
    }

    #[test]
    fn a_subordinate_current_table_is_events_only() {
        let reference = nyh1927();
        let offsets = SubordinateCurrentOffsets {
            station: "ACT0001".into(),
            bin: Some(1),
            reference_id: "n03020".into(),
            reference_bin: 7,
            mfc_time_adj_min: 0,
            mec_time_adj_min: 0,
            sbf_time_adj_min: 0,
            sbe_time_adj_min: 0,
            mfc_amp_adj: 1.0,
            mec_amp_adj: 1.0,
            flood_dir_deg: Some(31.0),
            ebb_dir_deg: Some(205.0),
        };
        let from = utc(2026, 11, 8);
        let err = current_table(
            CurrentStation::Subordinate {
                reference: &reference,
                offsets: &offsets,
            },
            base("ACT0001", StationKind::CurrentSubordinate),
            from,
            UnixTime(from.0 + 86_400),
            Duration::from_mins(6),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            TableError::Current(CurrentError::ReferenceMismatch { .. })
        ));
        let matching = SubordinateCurrentOffsets {
            reference_id: "NYH1927".into(),
            reference_bin: 13,
            ..offsets
        };
        let table = current_table(
            CurrentStation::Subordinate {
                reference: &reference,
                offsets: &matching,
            },
            base("ACT0001", StationKind::CurrentSubordinate),
            from,
            UnixTime(from.0 + 86_400),
            Duration::from_mins(6),
        )
        .unwrap();
        assert!(table.curve.is_none());
        assert!(table.events.len() >= 6);
        assert_eq!(table.flood_dir_deg, Some(31.0));
        assert_eq!(table.bin, Some(1));
    }

    #[test]
    fn bad_windows_are_refused() {
        let b = battery();
        let t = utc(2026, 11, 8);
        let run = |from: UnixTime, to: UnixTime, step: Duration| {
            tide_table(
                TideStation::Harmonic(&b),
                base("8518750", StationKind::TideHarmonic),
                from,
                to,
                step,
            )
        };
        let hour = Duration::from_hours(1);
        assert_eq!(run(t, t, Duration::ZERO), Err(TableError::BadWindow));
        assert_eq!(run(UnixTime(t.0 + 1), t, hour), Err(TableError::BadWindow));
        assert_eq!(
            run(
                t,
                UnixTime(t.0 + 400 * 365 * 86_400),
                Duration::from_secs(1)
            ),
            Err(TableError::BadWindow)
        );
    }
}
