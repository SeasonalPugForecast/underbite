//! Barometric pressure trend.
//!
//! Both trends use whether's [`Trend`] and its thresholds: a change of at least +1 hPa over three
//! hours is rising, at most -1 hPa falling, anything between steady.

use std::time::Duration;

pub use whether_core::derive::Trend;
use whether_core::derive::pressure_trend;
use whether_core::model::WeatherSnapshot;

use crate::series::Series;
use crate::time::UnixTime;

/// The window an observed trend looks back over.
const WINDOW_S: i64 = 3 * 3_600;
/// The newest observation may be at most this old for a trend "now".
const MAX_STALE_S: i64 = 3_600;
/// Neighbouring observations further apart than this are not interpolated across.
const MAX_GAP: Duration = Duration::from_mins(90);
/// The change that makes a trend rising or falling, hPa per three hours.
const THRESHOLD_HPA: f32 = 1.0;

/// The forecast pressure trend from the current step to three hours later, as computed by
/// `whether_core::derive::pressure_trend` over the snapshot's hourly steps.
pub fn forecast_trend(weather: &WeatherSnapshot, now: UnixTime) -> Option<Trend> {
    pressure_trend(&weather.hourly, now)
}

/// The trend over the last three hours of a station's pressure series in hPa (NDBC `PRES`,
/// CO-OPS `air_pressure`).
///
/// Compares the newest sample at or before `now` with the value three hours before it
/// (interpolated between neighbours no more than 90 minutes apart). `None` when the newest
/// sample is more than an hour older than `now`, or the series does not reach back three hours.
/// The change is rounded to 0.01 hPa so a reading pair differing by exactly 1.0 hPa is not
/// lost to float representation.
pub fn observed_trend(series: &Series<f32>, now: UnixTime) -> Option<Trend> {
    let end = series.samples().iter().rev().find(|s| s.time <= now)?;
    if now.0 - end.time.0 > MAX_STALE_S {
        return None;
    }
    let start = series.at(UnixTime(end.time.0 - WINDOW_S), MAX_GAP)?;
    let delta = ((end.value - start) * 100.0).round() / 100.0;
    Some(if delta >= THRESHOLD_HPA {
        Trend::Rising
    } else if delta <= -THRESHOLD_HPA {
        Trend::Falling
    } else {
        Trend::Steady
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{Provenance, Quality, Relation, SourceId, ValueKind};
    use crate::series::Sample;
    use whether_core::testdata::sample_snapshot;
    use whether_core::units::Hpa;

    fn prov() -> Provenance {
        Provenance {
            source: SourceId::new("ndbc"),
            station: None,
            distance_m: None,
            relation: Relation::Nearby,
            observed_at: UnixTime(0),
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        }
    }

    fn series(points: &[(i64, f32)]) -> Series<f32> {
        Series::new(
            prov(),
            points
                .iter()
                .map(|&(t, v)| Sample::new(UnixTime(t), v))
                .collect(),
        )
    }

    #[test]
    fn falling_series_is_falling() {
        let s = series(&[(0, 1012.0), (5_400, 1011.2), (10_800, 1010.5)]);
        assert_eq!(observed_trend(&s, UnixTime(10_800)), Some(Trend::Falling));
    }

    #[test]
    fn rising_steady_and_boundaries() {
        let rising = series(&[(0, 1010.0), (10_800, 1011.0)]);
        // The two samples are three hours apart, beyond the interpolation gap, but the start
        // is an exact sample time.
        assert_eq!(
            observed_trend(&rising, UnixTime(10_800)),
            Some(Trend::Rising)
        );
        let steady = series(&[(0, 1010.0), (10_800, 1010.9)]);
        assert_eq!(
            observed_trend(&steady, UnixTime(10_800)),
            Some(Trend::Steady)
        );
    }

    #[test]
    fn stale_or_short_series_has_no_trend() {
        let s = series(&[(0, 1010.0), (10_800, 1009.0)]);
        assert_eq!(observed_trend(&s, UnixTime(10_800 + 3_601)), None);
        assert_eq!(
            observed_trend(&s, UnixTime(10_800 + 3_600)),
            Some(Trend::Falling)
        );
        let short = series(&[(5_000, 1010.0), (10_800, 1009.0)]);
        assert_eq!(observed_trend(&short, UnixTime(10_800)), None);
        assert_eq!(observed_trend(&series(&[]), UnixTime(0)), None);
    }

    /// Reads the `PRES` column of an NDBC realtime `.txt` file (newest row first).
    fn read_pres(text: &str) -> Series<f32> {
        let mut samples = Vec::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let num = |i: usize| cols[i].parse::<i64>().ok();
            let (Some(year), Some(month), Some(day), Some(hour), Some(minute)) =
                (num(0), num(1), num(2), num(3), num(4))
            else {
                continue;
            };
            let Ok(pres) = cols[12].parse::<f32>() else {
                continue;
            };
            let days = whether_core::time::days_from_civil(year as i32, month as u8, day as u8);
            let time = days * 86_400 + hour * 3_600 + minute * 60;
            samples.push(Sample::new(UnixTime(time), pres));
        }
        Series::new(prov(), samples)
    }

    #[test]
    fn batn6_has_a_trend_at_its_newest_row() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/raw/ndbc/ndbc-BATN6-head300.txt"
        ))
        .unwrap();
        let s = read_pres(&text);
        assert!(s.samples().len() > 100);
        let newest = s.latest().unwrap().time;
        let trend = observed_trend(&s, newest);
        assert!(trend.is_some());
    }

    #[test]
    fn forecast_trend_uses_whether() {
        let mut snap = sample_snapshot();
        let first = snap.hourly[0].time;
        for (i, h) in snap.hourly.iter_mut().enumerate() {
            h.pressure_msl = Some(Hpa(1_010.0 - i as f32));
        }
        assert_eq!(forecast_trend(&snap, first), Some(Trend::Falling));
        snap.hourly.clear();
        assert_eq!(forecast_trend(&snap, first), None);
    }
}
