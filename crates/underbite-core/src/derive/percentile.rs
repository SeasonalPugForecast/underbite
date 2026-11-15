//! Where a value sits among the normals for its day of the year.
//!
//! The Statistics API publishes the 5, 10, 25, 50, 75, 90 and 95th percentiles of the record for
//! a day of the year; [`rank`] places a current value among them and [`class`] names the band.
//! [`from_daily`] builds the same thresholds from daily means when that API is not available.
//!
//! # Class bounds
//!
//! The bounds follow the USGS `hyswap` package's `WDFN_stats` schema, which mirrors the National
//! Water Dashboard (`https://github.com/DOI-USGS/hyswap`, `hyswap/utils.py`,
//! `retrieve_schema`). Ranges are closed on the left and open on the right:
//! much below normal `[0, 10)`, below normal `[10, 25)`, normal `[25, 76)`, above normal
//! `[76, 90)`, much above normal `[90, 100]`. The retired `WaterWatch` table
//! (`https://waterwatch.usgs.gov/htmls/ptile_table_a.html`, archived at
//! `http://web.archive.org/web/20170501151620/https://waterwatch.usgs.gov/htmls/ptile_table_a.html`)
//! says "> 90th percentile" for its top class, so a rank of exactly 90 differs between the two;
//! `hyswap` is followed here.

use std::collections::BTreeSet;

use crate::model::{Percentile, PercentileBasis, PercentileClass, PercentileThresholds};
use crate::time::LocalDate;

/// The percentile points the Statistics API reports, with their known thresholds.
fn knots(t: &PercentileThresholds) -> Vec<(f32, f32)> {
    [
        (5.0, t.p05),
        (10.0, t.p10),
        (25.0, t.p25),
        (50.0, t.p50),
        (75.0, t.p75),
        (90.0, t.p90),
        (95.0, t.p95),
    ]
    .into_iter()
    .filter_map(|(p, v)| v.filter(|v| v.is_finite()).map(|v| (p, v)))
    .collect()
}

/// The percentile rank (0 to 100) of `value` among `thresholds`.
///
/// Between two known thresholds the rank interpolates linearly. A value equal to one or more
/// thresholds takes the mean of their ranks. Below the lowest threshold (normally the 5th) the
/// rank interpolates from 0 at `min` to the threshold's rank, and above the highest (normally the
/// 95th) from there to 100 at `max`, when `min` or `max` is given and lies beyond the
/// threshold; a value at or beyond `min` or `max` ranks 0 or 100. Without `min` or `max` the rank
/// is clamped to the band edge (5 below the 5th percentile, 95 above the 95th).
///
/// `None` when no threshold is known or `value` is not finite.
pub fn rank(
    value: f32,
    thresholds: &PercentileThresholds,
    min: Option<f32>,
    max: Option<f32>,
) -> Option<f32> {
    if !value.is_finite() {
        return None;
    }
    let knots = knots(thresholds);
    let (&(low_rank, low), &(high_rank, high)) = (knots.first()?, knots.last()?);

    let equal: Vec<f32> = knots
        .iter()
        .filter(|(_, v)| v.total_cmp(&value).is_eq())
        .map(|(p, _)| *p)
        .collect();
    if !equal.is_empty() {
        return Some(equal.iter().sum::<f32>() / equal.len() as f32);
    }
    if value < low {
        return Some(match min.filter(|m| m.is_finite() && *m < low) {
            Some(m) if value <= m => 0.0,
            Some(m) => low_rank * (value - m) / (low - m),
            None => low_rank,
        });
    }
    if value > high {
        return Some(match max.filter(|m| m.is_finite() && *m > high) {
            Some(m) if value >= m => 100.0,
            Some(m) => high_rank + (100.0 - high_rank) * (value - high) / (m - high),
            None => high_rank,
        });
    }
    knots.windows(2).find_map(|w| {
        let ((p0, v0), (p1, v1)) = (w[0], w[1]);
        (value > v0 && value < v1).then(|| p0 + (p1 - p0) * (value - v0) / (v1 - v0))
    })
}

/// The class of a percentile rank; see the module documentation for the bounds. `None` for a
/// rank outside 0 to 100 or not a number.
pub fn class(rank: f32) -> Option<PercentileClass> {
    Some(match rank {
        r if !(0.0..=100.0).contains(&r) => return None,
        r if r < 10.0 => PercentileClass::MuchBelowNormal,
        r if r < 25.0 => PercentileClass::BelowNormal,
        r if r < 76.0 => PercentileClass::Normal,
        r if r < 90.0 => PercentileClass::AboveNormal,
        _ => PercentileClass::MuchAboveNormal,
    })
}

/// The default number of years a computed percentile needs.
pub const DEFAULT_MIN_YEARS: u32 = 10;

/// Percentile `p` (0 to 100) of ascending, non-empty `sorted` values by the Weibull plotting
/// position `m / (n + 1)`, interpolating linearly between neighbours and clamping to the extremes.
fn weibull(sorted: &[f32], percent: f64) -> f32 {
    let count = sorted.len();
    let pos = (percent / 100.0 * (count as f64 + 1.0)).clamp(1.0, count as f64);
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(count);
    let (below, above) = (f64::from(sorted[lo - 1]), f64::from(sorted[hi - 1]));
    (below + (pos - lo as f64) * (above - below)) as f32
}

/// Computes the day-of-year normals from daily means and places `value` among them.
///
/// `values` are `(date, daily mean)` pairs in the site's standard-time days (the caller
/// converts); `day` is `(month, day)` and `value` the current reading in the same unit. The
/// percentiles use the Weibull plotting position `m / (n + 1)` with linear interpolation, which
/// is not necessarily the Statistics API's method, so the numbers are labelled
/// [`PercentileBasis::ComputedFromDaily`] and are not expected to match the API exactly. Only
/// the same calendar day of each year counts (no window), except 29 February: values dated
/// 29 February are used when at least `min_years` years have one, otherwise 28 February and
/// 1 March of every year are pooled. A year that contributes two pooled values counts once.
///
/// `None` when `day` is not a calendar day, fewer than `min_years` years contribute, or `value`
/// is not finite. The record may include provisional values, so `approved_only` is `false`. The
/// caller should leave the current day out of `values`.
pub fn from_daily(
    values: &[(LocalDate, f32)],
    day: (u8, u8),
    value: f32,
    min_years: u32,
) -> Option<Percentile> {
    let (month, dom) = day;
    // 2000 is a leap year, so 02-29 is accepted.
    if !(1..=12).contains(&month)
        || dom == 0
        || dom > whether_core::time::days_in_month(2000, month)
        || !value.is_finite()
    {
        return None;
    }
    let pick = |days: &[(u8, u8)]| -> Vec<(i32, f32)> {
        values
            .iter()
            .filter(|(d, v)| v.is_finite() && days.contains(&(d.month, d.day)))
            .map(|(d, v)| (d.year, *v))
            .collect()
    };
    let years_of = |picked: &[(i32, f32)]| picked.iter().map(|(y, _)| *y).collect::<BTreeSet<_>>();

    let mut picked = pick(&[day]);
    let mut years = years_of(&picked);
    if day == (2, 29) && years.len() < min_years as usize {
        picked = pick(&[(2, 28), (3, 1)]);
        years = years_of(&picked);
    }
    let years = u32::try_from(years.len()).ok()?;
    if years < min_years.max(1) {
        return None;
    }

    let mut sorted: Vec<f32> = picked.iter().map(|(_, v)| *v).collect();
    sorted.sort_by(f32::total_cmp);
    let thresholds = PercentileThresholds {
        p05: Some(weibull(&sorted, 5.0)),
        p10: Some(weibull(&sorted, 10.0)),
        p25: Some(weibull(&sorted, 25.0)),
        p50: Some(weibull(&sorted, 50.0)),
        p75: Some(weibull(&sorted, 75.0)),
        p90: Some(weibull(&sorted, 90.0)),
        p95: Some(weibull(&sorted, 95.0)),
    };
    let (min, max) = (sorted[0], sorted[sorted.len() - 1]);
    let mean = (sorted.iter().map(|v| f64::from(*v)).sum::<f64>() / sorted.len() as f64) as f32;
    let rank = rank(value, &thresholds, Some(min), Some(max));
    Some(Percentile {
        day_of_year: format!("{month:02}-{dom:02}"),
        thresholds,
        min: Some(min),
        max: Some(max),
        mean: Some(mean),
        value,
        rank,
        class: rank.and_then(class),
        basis: PercentileBasis::ComputedFromDaily { years },
        sample_count: years,
        approved_only: false,
        datum: None,
        provenance: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/raw/usgs/"
    );

    fn thresholds_1108() -> PercentileThresholds {
        let text = std::fs::read_to_string(format!(
            "{RAW}usgs-statistics-normals-croton-01375000-00060-doy-percentile-11-05_11-10.json"
        ))
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let entry = json["features"][0]["properties"]["data"][0]["values"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["time_of_year"] == "11-08")
            .unwrap();
        let get = |i: usize| {
            entry["values"][i]
                .as_str()
                .map(|s| s.parse::<f32>().unwrap())
        };
        PercentileThresholds {
            p05: get(0),
            p10: get(1),
            p25: get(2),
            p50: get(3),
            p75: get(4),
            p90: get(5),
            p95: get(6),
        }
    }

    fn date(year: i32, month: u8, day: u8) -> LocalDate {
        LocalDate { year, month, day }
    }

    #[test]
    fn croton_november_eighth() {
        let t = thresholds_1108();
        assert_eq!(t.p05, Some(0.634));
        assert_eq!(t.p50, Some(15.0));
        assert_eq!(t.p95, Some(1180.0));
        let r = rank(12.4, &t, None, None).unwrap();
        assert!(r > 25.0 && r < 50.0, "{r}");
        assert_eq!(class(r), Some(PercentileClass::Normal));
    }

    #[test]
    fn rank_interpolates_and_clamps() {
        let t = PercentileThresholds {
            p05: Some(10.0),
            p10: Some(20.0),
            p25: Some(30.0),
            p50: Some(40.0),
            p75: Some(50.0),
            p90: Some(60.0),
            p95: Some(70.0),
        };
        assert_eq!(rank(15.0, &t, None, None), Some(7.5));
        assert_eq!(rank(40.0, &t, None, None), Some(50.0));
        assert_eq!(rank(45.0, &t, None, None), Some(62.5));
        // Clamped to the band edges without min and max.
        assert_eq!(rank(1.0, &t, None, None), Some(5.0));
        assert_eq!(rank(500.0, &t, None, None), Some(95.0));
        // Interpolated against min and max when given.
        assert_eq!(rank(5.0, &t, Some(0.0), Some(170.0)), Some(2.5));
        assert_eq!(rank(120.0, &t, Some(0.0), Some(170.0)), Some(97.5));
        assert_eq!(rank(0.0, &t, Some(0.0), Some(170.0)), Some(0.0));
        assert_eq!(rank(200.0, &t, Some(0.0), Some(170.0)), Some(100.0));
        // A min that is not below the 5th percentile is ignored.
        assert_eq!(rank(1.0, &t, Some(12.0), None), Some(5.0));
        assert_eq!(rank(f32::NAN, &t, None, None), None);
        assert_eq!(
            rank(1.0, &PercentileThresholds::default(), None, None),
            None
        );
    }

    #[test]
    fn rank_with_ties_and_gaps() {
        let t = PercentileThresholds {
            p05: Some(0.0),
            p10: Some(0.0),
            p50: Some(4.0),
            p95: Some(8.0),
            ..Default::default()
        };
        assert_eq!(rank(0.0, &t, None, None), Some(7.5));
        // Between the 10th and 50th, the 25th and 75th being absent.
        assert_eq!(rank(2.0, &t, None, None), Some(30.0));
    }

    #[test]
    fn class_bounds() {
        use PercentileClass::*;
        for (r, c) in [
            (0.0, MuchBelowNormal),
            (9.99, MuchBelowNormal),
            (10.0, BelowNormal),
            (24.9, BelowNormal),
            (25.0, Normal),
            (75.5, Normal),
            (76.0, AboveNormal),
            (89.9, AboveNormal),
            (90.0, MuchAboveNormal),
            (100.0, MuchAboveNormal),
        ] {
            assert_eq!(class(r), Some(c), "{r}");
        }
        assert_eq!(class(-1.0), None);
        assert_eq!(class(100.5), None);
        assert_eq!(class(f32::NAN), None);
    }

    /// Twenty years of one value each, 1 to 20, on `month-day`.
    fn twenty_years(month: u8, day: u8) -> Vec<(LocalDate, f32)> {
        (1..=20)
            .map(|i| (date(2000 + i, month, day), i as f32))
            .collect()
    }

    #[test]
    fn from_daily_known_distribution() {
        let mut values = twenty_years(6, 15);
        // Other days must not count.
        values.push((date(2021, 6, 16), 1_000.0));
        let p = from_daily(&values, (6, 15), 10.5, 10).unwrap();
        let t = p.thresholds;
        let close = |a: Option<f32>, b: f32| (a.unwrap() - b).abs() < 1e-4;
        // Weibull positions p (n + 1) / 100 with n = 20: 1.05, 2.1, 5.25, 10.5, 15.75, 18.9, 19.95.
        assert!(close(t.p05, 1.05), "{t:?}");
        assert!(close(t.p10, 2.1));
        assert!(close(t.p25, 5.25));
        assert!(close(t.p50, 10.5));
        assert!(close(t.p75, 15.75));
        assert!(close(t.p90, 18.9));
        assert!(close(t.p95, 19.95));
        assert_eq!((p.min, p.max, p.mean), (Some(1.0), Some(20.0), Some(10.5)));
        assert_eq!(p.day_of_year, "06-15");
        assert_eq!(p.basis, PercentileBasis::ComputedFromDaily { years: 20 });
        assert_eq!(p.sample_count, 20);
        assert!(!p.approved_only);
        assert_eq!(p.rank, Some(50.0));
        assert_eq!(p.class, Some(PercentileClass::Normal));
        // A value above the record ranks 100.
        let high = from_daily(&values, (6, 15), 99.0, 10).unwrap();
        assert_eq!(high.class, Some(PercentileClass::MuchAboveNormal));
        assert_eq!(high.rank, Some(100.0));
    }

    #[test]
    fn from_daily_needs_min_years() {
        let values = twenty_years(6, 15);
        assert!(from_daily(&values, (6, 15), 5.0, 21).is_none());
        assert!(from_daily(&values, (6, 14), 5.0, 1).is_none());
        assert!(from_daily(&values, (2, 30), 5.0, 1).is_none());
        assert!(from_daily(&values, (6, 15), f32::NAN, 10).is_none());
    }

    #[test]
    fn leap_day_uses_its_own_values_or_pools_neighbours() {
        // Ten leap years with a value on 02-29: used alone.
        let mut own: Vec<(LocalDate, f32)> = (0..10)
            .map(|i| (date(1980 + 4 * i, 2, 29), 100.0 + i as f32))
            .collect();
        own.extend((1980..2020).map(|y| (date(y, 2, 28), 1.0)));
        let p = from_daily(&own, (2, 29), 105.0, 10).unwrap();
        assert_eq!(p.sample_count, 10);
        assert_eq!(p.min, Some(100.0));

        // Only three leap days: pool 02-28 and 03-01 over all years.
        let mut few: Vec<(LocalDate, f32)> =
            (0..3).map(|i| (date(2016 + 4 * i, 2, 29), 500.0)).collect();
        for y in 2005..2025 {
            few.push((date(y, 2, 28), 10.0));
            few.push((date(y, 3, 1), 20.0));
        }
        let p = from_daily(&few, (2, 29), 15.0, 10).unwrap();
        assert_eq!(p.sample_count, 20);
        assert_eq!((p.min, p.max), (Some(10.0), Some(20.0)));
        assert_eq!(p.thresholds.p50, Some(15.0));
    }

    #[test]
    fn one_year_of_daily_values_is_not_enough() {
        let text = std::fs::read_to_string(format!(
            "{RAW}usgs-daily-croton-01375000-00060-mean-2025-11-08_2026-11-07.json"
        ))
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let values: Vec<(LocalDate, f32)> = json["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| {
                let props = &f["properties"];
                let time = props["time"].as_str()?;
                let value = props["value"].as_str()?.parse::<f32>().ok()?;
                let mut parts = time.split('-').map(|p| p.parse::<i32>().ok());
                let year = parts.next()??;
                let month = u8::try_from(parts.next()??).ok()?;
                let day = u8::try_from(parts.next()??).ok()?;
                Some((date(year, month, day), value))
            })
            .collect();
        assert!(values.len() > 300);
        assert!(from_daily(&values, (11, 8), 12.4, DEFAULT_MIN_YEARS).is_none());
        assert!(from_daily(&values, (11, 8), 12.4, 1).is_some());
    }
}
