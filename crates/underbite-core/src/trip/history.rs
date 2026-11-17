//! What a trip's dates have looked like in past years: typical flow, temperatures and waves.
//!
//! This is history, never a forecast and never an input to go/no-go. Every band says where it
//! came from ([`HistoryBasis`]: the source, whether it is a station, a reanalysis or a model, and
//! the years behind it). The module returns data in canonical units (m³/s, metres, °C); words and
//! units belong to the view.
//!
//! The core cannot depend on the provider crate, so the inputs are plain values. A caller maps
//! the provider results like this:
//!
//! - USGS Statistics API: each `DayNormals` of a `NormalsRange` becomes a [`DayThresholds`]
//!   (`day_of_year`, `thresholds` and `sample_count` carry over unchanged), and the range's
//!   `provenance.source` is the [`SeriesInput::normals_source`]. An empty or failed response is
//!   an empty slice.
//! - USGS daily values (the fallback): the site's standard-time daily means as
//!   `(LocalDate, value)` pairs in canonical units.
//! - Open-Meteo marine: collect each request's `MarineHistory.waves` and `.sea_surface_temp` and
//!   give them to [`wave_daily_means`] and [`daily_means`]; the result is a [`GriddedInput`].
//! - Weather: whether's `DayClimate` rows for the trip's dates, with the [`HistoryBasis`] of the
//!   years the query used.
//!
//! The trip is capped at [`MAX_TRIP_DAYS`] days, which equals whether's `MAX_QUERY_DAYS`.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whether_core::history::{DayClimate, YearRange};

use crate::derive::percentile::from_daily;
use crate::model::{PercentileBasis, PercentileThresholds, WaveSeries};
use crate::provenance::SourceId;
use crate::series::Series;
use crate::time::{self, LocalDate, day_key};
use crate::trip::model::MAX_TRIP_DAYS;

/// The fewest hourly samples a local day needs to give a daily mean.
pub const MIN_DAY_SAMPLES: usize = 12;

/// What kind of record a band comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryKind {
    /// Measured at a gauge or station.
    Station,
    /// A reanalysis (ERA5-Ocean at 0.5 degrees).
    Reanalysis,
    /// A forecast model's archive (Open-Meteo best match).
    Model,
}

/// Where a band of past values came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HistoryBasis {
    pub source: SourceId,
    pub kind: HistoryKind,
    /// The first and last year in the sample; absent when the source gives only a count (the
    /// Statistics API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub years: Option<YearRange>,
    /// How many years contribute to the day.
    pub years_sampled: u32,
    /// How a station's percentiles were made; absent for gridded history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentiles: Option<PercentileBasis>,
}

/// The 10th, 50th and 90th percentile of a day in past years, in canonical units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Band {
    pub p10: f32,
    pub median: f32,
    pub p90: f32,
    pub basis: HistoryBasis,
}

/// Whether a quantity was asked for and whether history exists for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// Not asked for (no source for this place).
    #[default]
    NotRequested,
    /// Asked for and nothing came back for any date ("no history here", not zero).
    Empty,
    /// At least one date has a band.
    Available,
}

/// One date as it looked in past years.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TypicalDay {
    /// River discharge, m³/s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Band>,
    /// Water temperature at the gauge, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_temp: Option<Band>,
    /// Sea surface temperature, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sst: Option<Band>,
    /// Significant wave height, metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waves: Option<Band>,
    /// Temperatures and precipitation (whether's climate for the date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather: Option<DayClimate>,
}

/// The history of every date of a trip.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TripHistory {
    #[serde(with = "days_by_date")]
    #[schemars(with = "BTreeMap<String, TypicalDay>")]
    pub days: BTreeMap<LocalDate, TypicalDay>,
    #[serde(default)]
    pub flow: Coverage,
    #[serde(default)]
    pub water_temp: Coverage,
    #[serde(default)]
    pub sst: Coverage,
    #[serde(default)]
    pub waves: Coverage,
    /// The years and source of the weather rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather_basis: Option<HistoryBasis>,
}

/// Day-of-year percentiles from the Statistics API, one day.
#[derive(Clone, Debug, PartialEq)]
pub struct DayThresholds {
    /// `MM-DD`.
    pub day_of_year: String,
    pub thresholds: PercentileThresholds,
    pub sample_count: u32,
}

/// Flow or water temperature: the Statistics API's normals first, the daily record second.
#[derive(Clone, Copy, Debug)]
pub struct SeriesInput<'a> {
    /// Normals by day; empty when the request failed or the site has none.
    pub normals: &'a [DayThresholds],
    pub normals_source: &'a SourceId,
    /// Daily means in the site's standard-time days, for the fallback.
    pub daily: &'a [(LocalDate, f32)],
    pub daily_source: &'a SourceId,
    /// The fewest years the fallback needs.
    pub min_years: u32,
}

/// Sea surface temperature or waves from a gridded history.
#[derive(Clone, Copy, Debug)]
pub struct GriddedInput<'a> {
    /// Daily means in the trip's local days (see [`daily_means`]).
    pub daily: &'a [(LocalDate, f32)],
    pub source: &'a SourceId,
    pub kind: HistoryKind,
    /// The fewest years a date needs for a band.
    pub min_years: u32,
}

/// Everything [`TripHistory::build`] reads.
#[derive(Clone, Debug, Default)]
pub struct HistoryInputs<'a> {
    pub flow: Option<SeriesInput<'a>>,
    pub water_temp: Option<SeriesInput<'a>>,
    pub sst: Option<GriddedInput<'a>>,
    pub waves: Option<GriddedInput<'a>>,
    /// Climate rows for the trip's dates and the basis they share.
    pub weather: Option<(&'a [DayClimate], HistoryBasis)>,
}

/// Why history cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HistoryError {
    #[error("the last day is before the first")]
    Backwards,
    #[error("history covers at most {MAX_TRIP_DAYS} days")]
    TooLong,
}

fn band_of(t: &PercentileThresholds, basis: HistoryBasis) -> Option<Band> {
    let finite = |v: Option<f32>| v.filter(|v| v.is_finite());
    Some(Band {
        p10: finite(t.p10)?,
        median: finite(t.p50)?,
        p90: finite(t.p90)?,
        basis,
    })
}

fn year_range(daily: &[(LocalDate, f32)]) -> Option<YearRange> {
    daily
        .iter()
        .filter(|(_, v)| v.is_finite())
        .fold(None, |acc, (d, _)| match acc {
            None => Some(YearRange {
                first: d.year,
                last: d.year,
            }),
            Some(r) => Some(YearRange {
                first: r.first.min(d.year),
                last: r.last.max(d.year),
            }),
        })
}

/// The band of one date from the Statistics normals, else from the daily record.
///
/// The 10th and 90th percentiles are the ones the source gives or the fallback computes; nothing
/// is interpolated between the fixed percentiles. `None` when neither has the date.
pub fn series_band(date: LocalDate, input: &SeriesInput<'_>) -> Option<Band> {
    let key = day_key(date);
    if let Some(day) = input.normals.iter().find(|d| d.day_of_year == key) {
        let basis = HistoryBasis {
            source: input.normals_source.clone(),
            kind: HistoryKind::Station,
            years: None,
            years_sampled: day.sample_count,
            percentiles: Some(PercentileBasis::StatisticsApi),
        };
        if let Some(band) = band_of(&day.thresholds, basis) {
            return Some(band);
        }
    }
    let computed = from_daily(input.daily, (date.month, date.day), 0.0, input.min_years)?;
    let basis = HistoryBasis {
        source: input.daily_source.clone(),
        kind: HistoryKind::Station,
        years: year_range(input.daily),
        years_sampled: computed.sample_count,
        percentiles: Some(computed.basis),
    };
    band_of(&computed.thresholds, basis)
}

/// The band of one date from gridded daily means; 29 February also uses 28 and 1 March when too
/// few years have a 29th.
pub fn gridded_band(date: LocalDate, input: &GriddedInput<'_>) -> Option<Band> {
    let computed = from_daily(input.daily, (date.month, date.day), 0.0, input.min_years)?;
    let basis = HistoryBasis {
        source: input.source.clone(),
        kind: input.kind,
        years: year_range(input.daily),
        years_sampled: computed.sample_count,
        percentiles: None,
    };
    band_of(&computed.thresholds, basis)
}

fn means_by<T>(
    series: &[&Series<T>],
    zone: &TimeZone,
    value: impl Fn(&T) -> Option<f32>,
) -> Vec<(LocalDate, f32)> {
    let mut sums: BTreeMap<LocalDate, (f64, usize)> = BTreeMap::new();
    for s in series {
        for sample in s.samples() {
            let Some(v) = value(&sample.value).filter(|v| v.is_finite()) else {
                continue;
            };
            let Ok(date) = time::local_date(sample.time, zone) else {
                continue;
            };
            let e = sums.entry(date).or_default();
            e.0 += f64::from(v);
            e.1 += 1;
        }
    }
    sums.into_iter()
        .filter(|(_, (_, n))| *n >= MIN_DAY_SAMPLES)
        .map(|(d, (sum, n))| (d, (sum / n as f64) as f32))
        .collect()
}

/// Daily means of hourly series (one per request) in the trip's local days. A day with fewer
/// than [`MIN_DAY_SAMPLES`] samples (a request edge, or nulls) gives no mean.
pub fn daily_means(series: &[&Series<f32>], zone: &TimeZone) -> Vec<(LocalDate, f32)> {
    means_by(series, zone, |v| Some(*v))
}

/// Daily mean significant wave height of hourly wave series, in the trip's local days. Series
/// without heights (an all-null inshore cell) give nothing.
pub fn wave_daily_means(series: &[&WaveSeries], zone: &TimeZone) -> Vec<(LocalDate, f32)> {
    means_by(series, zone, |w| w.significant_height_m)
}

impl TripHistory {
    /// The history of each date from `first` to `last` inclusive.
    ///
    /// # Errors
    /// [`HistoryError`] when the dates run backwards or cover more than [`MAX_TRIP_DAYS`].
    pub fn build(
        first: LocalDate,
        last: LocalDate,
        inputs: &HistoryInputs<'_>,
    ) -> Result<Self, HistoryError> {
        let (a, b) = (time::day_number(first), time::day_number(last));
        if b < a {
            return Err(HistoryError::Backwards);
        }
        if b - a >= MAX_TRIP_DAYS {
            return Err(HistoryError::TooLong);
        }
        let mut out = Self {
            weather_basis: inputs.weather.as_ref().map(|(_, b)| b.clone()),
            ..Self::default()
        };
        let mut any = [false; 4];
        for n in a..=b {
            let (year, month, day) = whether_core::time::civil_from_days(n);
            let date = LocalDate { year, month, day };
            let typical = TypicalDay {
                flow: inputs.flow.as_ref().and_then(|i| series_band(date, i)),
                water_temp: inputs
                    .water_temp
                    .as_ref()
                    .and_then(|i| series_band(date, i)),
                sst: inputs.sst.as_ref().and_then(|i| gridded_band(date, i)),
                waves: inputs.waves.as_ref().and_then(|i| gridded_band(date, i)),
                weather: inputs
                    .weather
                    .as_ref()
                    .and_then(|(rows, _)| rows.iter().find(|r| r.date == date).copied()),
            };
            any[0] |= typical.flow.is_some();
            any[1] |= typical.water_temp.is_some();
            any[2] |= typical.sst.is_some();
            any[3] |= typical.waves.is_some();
            out.days.insert(date, typical);
        }
        let coverage = |asked: bool, found: bool| match (asked, found) {
            (false, _) => Coverage::NotRequested,
            (true, false) => Coverage::Empty,
            (true, true) => Coverage::Available,
        };
        out.flow = coverage(inputs.flow.is_some(), any[0]);
        out.water_temp = coverage(inputs.water_temp.is_some(), any[1]);
        out.sst = coverage(inputs.sst.is_some(), any[2]);
        out.waves = coverage(inputs.waves.is_some(), any[3]);
        Ok(out)
    }
}

/// `BTreeMap<LocalDate, _>` on the wire with `YYYY-MM-DD` keys.
mod days_by_date {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::TypicalDay;
    use crate::time::{LocalDate, parse_date};

    pub(super) fn serialize<S: Serializer>(
        days: &BTreeMap<LocalDate, TypicalDay>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        days.iter()
            .map(|(d, v)| (format!("{:04}-{:02}-{:02}", d.year, d.month, d.day), v))
            .collect::<BTreeMap<_, _>>()
            .serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<LocalDate, TypicalDay>, D::Error> {
        BTreeMap::<String, TypicalDay>::deserialize(d)?
            .into_iter()
            .map(|(k, v)| {
                parse_date(&k)
                    .map(|d| (d, v))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WaveSample;
    use crate::provenance::{Provenance, Quality, Relation, ValueKind};
    use crate::series::Sample;
    use crate::time::{UnixTime, UtcOffset, parse_date, parse_local_iso, zone};

    const RAW: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/raw/");

    fn date(s: &str) -> LocalDate {
        parse_date(s).unwrap()
    }

    fn json(path: &str) -> serde_json::Value {
        let text = std::fs::read_to_string(format!("{RAW}{path}")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn croton_normals() -> Vec<DayThresholds> {
        let v = json(
            "usgs/usgs-statistics-normals-croton-01375000-00060-doy-percentile-11-05_11-10.json",
        );
        v["features"][0]["properties"]["data"][0]["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                let get = |i: usize| e["values"][i].as_str().map(|s| s.parse::<f32>().unwrap());
                DayThresholds {
                    day_of_year: e["time_of_year"].as_str().unwrap().to_owned(),
                    thresholds: PercentileThresholds {
                        p05: get(0),
                        p10: get(1),
                        p25: get(2),
                        p50: get(3),
                        p75: get(4),
                        p90: get(5),
                        p95: get(6),
                    },
                    sample_count: u32::try_from(e["sample_count"].as_u64().unwrap()).unwrap(),
                }
            })
            .collect()
    }

    fn croton_daily() -> Vec<(LocalDate, f32)> {
        let v = json("usgs/usgs-daily-croton-01375000-00060-mean-2025-11-08_2026-11-07.json");
        v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let p = &f["properties"];
                (
                    date(p["time"].as_str().unwrap()),
                    p["value"].as_str().unwrap().parse().unwrap(),
                )
            })
            .collect()
    }

    fn provenance(source: SourceId) -> Provenance {
        Provenance {
            source,
            station: None,
            distance_m: None,
            relation: Relation::Gridded,
            observed_at: UnixTime(0),
            kind: ValueKind::Modelled,
            quality: Quality::Provisional,
        }
    }

    /// The non-null values of a marine history column, hourly.
    fn hourly(path: &str, column: &str, years_ahead: i32, add: f32) -> Vec<Sample<f32>> {
        let v = json(path);
        let times = v["hourly"]["time"].as_array().unwrap();
        let values = v["hourly"][column].as_array().unwrap();
        times
            .iter()
            .zip(values)
            .filter_map(|(t, x)| {
                let value = x.as_f64()? as f32 + add;
                let t = t.as_str().unwrap();
                let year: i32 = t[..4].parse().unwrap();
                let t = format!("{:04}{}", year + years_ahead, &t[4..]);
                let time = parse_local_iso(&format!("{t}:00"), UtcOffset(0)).unwrap();
                Some(Sample::new(time, value))
            })
            .collect()
    }

    const NULLS: &str = "open-meteo-marine/marine-40.25_-73.16-history-2021-12-15-nulls.json";
    const Y2024: &str = "open-meteo-marine/marine-40.25_-73.16-history-2024-07-01.json";

    fn station_input<'a>(
        normals: &'a [DayThresholds],
        daily: &'a [(LocalDate, f32)],
        usgs: &'a SourceId,
    ) -> SeriesInput<'a> {
        SeriesInput {
            normals,
            normals_source: usgs,
            daily,
            daily_source: usgs,
            min_years: 1,
        }
    }

    #[test]
    fn croton_bands_match_the_fixture_percentiles() {
        let normals = croton_normals();
        let usgs = SourceId::USGS;
        let inputs = HistoryInputs {
            flow: Some(station_input(&normals, &[], &usgs)),
            ..HistoryInputs::default()
        };
        let h = TripHistory::build(date("2026-11-05"), date("2026-11-10"), &inputs).unwrap();
        assert_eq!(h.days.len(), 6);
        assert_eq!(h.flow, Coverage::Available);
        let expected = [
            ("2026-11-05", 0.976, 12.0, 692.0),
            ("2026-11-06", 0.816, 11.1, 661.0),
            ("2026-11-07", 0.89, 13.0, 662.2),
            ("2026-11-08", 0.966, 15.0, 689.6),
            ("2026-11-09", 0.82, 16.9, 786.0),
            ("2026-11-10", 0.866, 13.0, 894.2),
        ];
        for (d, p10, median, p90) in expected {
            let band = h.days[&date(d)].flow.as_ref().unwrap();
            assert_eq!((band.p10, band.median, band.p90), (p10, median, p90), "{d}");
            assert_eq!(band.basis.kind, HistoryKind::Station);
            assert_eq!(band.basis.years_sampled, 93);
            assert_eq!(band.basis.percentiles, Some(PercentileBasis::StatisticsApi));
        }
        assert_eq!(h.sst, Coverage::NotRequested);
        assert!(h.days[&date("2026-11-05")].sst.is_none());
    }

    #[test]
    fn an_empty_statistics_response_falls_back_to_the_daily_record() {
        let daily = croton_daily();
        let usgs = SourceId::USGS;
        let inputs = HistoryInputs {
            flow: Some(station_input(&[], &daily, &usgs)),
            ..HistoryInputs::default()
        };
        let h = TripHistory::build(date("2026-11-05"), date("2026-11-10"), &inputs).unwrap();
        let band = h.days[&date("2026-11-08")].flow.as_ref().unwrap();
        assert_eq!(
            band.basis.percentiles,
            Some(PercentileBasis::ComputedFromDaily { years: 1 })
        );
        assert_eq!(
            band.basis.years,
            Some(YearRange {
                first: 2025,
                last: 2026
            })
        );
        // One year in the record: the 8 November value is all there is.
        assert_eq!((band.p10, band.median, band.p90), (80.7, 80.7, 80.7));
        assert_eq!(h.flow, Coverage::Available);

        // The default ten years is not met by one year of record.
        let strict = SeriesInput {
            min_years: 10,
            ..station_input(&[], &daily, &usgs)
        };
        assert!(series_band(date("2026-11-08"), &strict).is_none());
    }

    #[test]
    fn a_day_missing_from_the_normals_uses_the_daily_record() {
        let normals = croton_normals();
        let daily = croton_daily();
        let usgs = SourceId::USGS;
        let input = station_input(&normals, &daily, &usgs);
        let in_normals = series_band(date("2026-11-08"), &input).unwrap();
        assert_eq!(
            in_normals.basis.percentiles,
            Some(PercentileBasis::StatisticsApi)
        );
        let computed = series_band(date("2026-11-20"), &input).unwrap();
        assert_eq!(
            computed.basis.percentiles,
            Some(PercentileBasis::ComputedFromDaily { years: 1 })
        );
    }

    #[test]
    fn null_sst_gives_no_band() {
        let sst = Series::new(
            provenance(SourceId::OPEN_METEO_MARINE),
            hourly(NULLS, "sea_surface_temperature", 0, 0.0),
        );
        assert_eq!(sst.samples().len(), 0);
        let ny = zone("America/New_York").unwrap();
        let means = daily_means(&[&sst], &ny);
        assert_eq!(means.len(), 0);
        let source = SourceId::OPEN_METEO_MARINE;
        let inputs = HistoryInputs {
            sst: Some(GriddedInput {
                daily: &means,
                source: &source,
                kind: HistoryKind::Model,
                min_years: 2,
            }),
            ..HistoryInputs::default()
        };
        let h = TripHistory::build(date("2021-12-15"), date("2021-12-15"), &inputs).unwrap();
        assert!(h.days[&date("2021-12-15")].sst.is_none());
        assert_eq!(h.sst, Coverage::Empty);
    }

    #[test]
    fn sst_is_labelled_with_its_real_years() {
        let ny = zone("America/New_York").unwrap();
        let years: Vec<Series<f32>> = [(-1, 0.0), (0, 1.0), (1, 2.0)]
            .into_iter()
            .map(|(k, add)| {
                Series::new(
                    provenance(SourceId::OPEN_METEO_MARINE),
                    hourly(Y2024, "sea_surface_temperature", k, add),
                )
            })
            .collect();
        let refs: Vec<&Series<f32>> = years.iter().collect();
        let means = daily_means(&refs, &ny);
        let source = SourceId::OPEN_METEO_MARINE;
        let inputs = HistoryInputs {
            sst: Some(GriddedInput {
                daily: &means,
                source: &source,
                kind: HistoryKind::Model,
                min_years: 2,
            }),
            ..HistoryInputs::default()
        };
        let h = TripHistory::build(date("2026-07-01"), date("2026-07-02"), &inputs).unwrap();
        let band = h.days[&date("2026-07-01")].sst.as_ref().unwrap();
        assert_eq!(band.basis.kind, HistoryKind::Model);
        assert_eq!(
            band.basis.years,
            Some(YearRange {
                first: 2023,
                last: 2025
            })
        );
        assert_eq!(band.basis.years_sampled, 3);
        assert!(band.p10 <= band.median && band.median <= band.p90);
        // 2 July has no hourly data in the fixture.
        assert!(h.days[&date("2026-07-02")].sst.is_none());
    }

    #[test]
    fn an_all_null_wave_cell_says_no_history() {
        let ny = zone("America/New_York").unwrap();
        let hours = hourly(Y2024, "wave_height", 0, 0.0)
            .into_iter()
            .map(|s| Sample::new(s.time, WaveSample::default()))
            .collect();
        let waves: WaveSeries = Series::new(provenance(SourceId::OPEN_METEO_MARINE), hours);
        let means = wave_daily_means(&[&waves], &ny);
        assert_eq!(means.len(), 0);
        let source = SourceId::OPEN_METEO_MARINE;
        let inputs = HistoryInputs {
            waves: Some(GriddedInput {
                daily: &means,
                source: &source,
                kind: HistoryKind::Reanalysis,
                min_years: 10,
            }),
            ..HistoryInputs::default()
        };
        let h = TripHistory::build(date("2026-07-01"), date("2026-07-01"), &inputs).unwrap();
        assert_eq!(h.waves, Coverage::Empty);
        assert!(h.days[&date("2026-07-01")].waves.is_none());
    }

    #[test]
    fn waves_from_a_reanalysis_are_banded_across_years() {
        let ny = zone("America/New_York").unwrap();
        let series: Vec<WaveSeries> = (0..10_i16)
            .map(|k| {
                let hours = hourly(Y2024, "wave_height", i32::from(k) - 9, 0.0)
                    .into_iter()
                    .map(|s| {
                        Sample::new(
                            s.time,
                            WaveSample {
                                significant_height_m: Some(s.value + f32::from(k) * 0.1),
                                ..WaveSample::default()
                            },
                        )
                    })
                    .collect();
                Series::new(provenance(SourceId::OPEN_METEO_MARINE), hours)
            })
            .collect();
        let refs: Vec<&WaveSeries> = series.iter().collect();
        let means = wave_daily_means(&refs, &ny);
        let source = SourceId::OPEN_METEO_MARINE;
        let inputs = HistoryInputs {
            waves: Some(GriddedInput {
                daily: &means,
                source: &source,
                kind: HistoryKind::Reanalysis,
                min_years: 10,
            }),
            ..HistoryInputs::default()
        };
        let h = TripHistory::build(date("2026-07-01"), date("2026-07-01"), &inputs).unwrap();
        let band = h.days[&date("2026-07-01")].waves.as_ref().unwrap();
        assert_eq!(band.basis.kind, HistoryKind::Reanalysis);
        assert_eq!(band.basis.years_sampled, 10);
        assert_eq!(
            band.basis.years,
            Some(YearRange {
                first: 2015,
                last: 2024
            })
        );
        assert!(band.p10 < band.median && band.median < band.p90);
    }

    #[test]
    fn the_date_range_is_checked() {
        let none = HistoryInputs::default();
        assert_eq!(
            TripHistory::build(date("2026-11-10"), date("2026-11-09"), &none),
            Err(HistoryError::Backwards)
        );
        assert_eq!(
            TripHistory::build(date("2026-11-01"), date("2026-12-02"), &none),
            Err(HistoryError::TooLong)
        );
        let ok = TripHistory::build(date("2026-11-01"), date("2026-12-01"), &none).unwrap();
        assert_eq!(ok.days.len(), 31);
        assert_eq!(ok.flow, Coverage::NotRequested);
    }

    #[test]
    fn history_round_trips_and_matches_its_snapshot() {
        let normals = croton_normals();
        let usgs = SourceId::USGS;
        let inputs = HistoryInputs {
            flow: Some(station_input(&normals, &[], &usgs)),
            water_temp: Some(station_input(&[], &[], &usgs)),
            ..HistoryInputs::default()
        };
        let h = TripHistory::build(date("2026-11-05"), date("2026-11-07"), &inputs).unwrap();
        let text = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<TripHistory>(&text).unwrap(), h);
        insta::assert_json_snapshot!("croton_2026_11_05", h);
    }
}
