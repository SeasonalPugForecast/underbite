//! USGS Statistics API: day-of-year normals.
//!
//! `observationNormals` (v0, beta, approved data only) gives the 5, 10, 25, 50, 75, 90 and 95th
//! percentiles of a series' daily means for a day of the year. [`Usgs::statistics_request`]
//! asks for one day of one series and carries the current value, so that the parse can place
//! it among the thresholds and return a [`Percentile`].
//!
//! When the API fails, changes shape or has no normals for the site, the parse finishes without
//! a value and the follow-up is a `daily` request over the last years. Its parse computes the
//! same thresholds with [`from_daily`]. A request that fell back says so in the error it ends
//! with, which names the basis ("statistics API: …; computed from daily means: …").

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use underbite_core::datum::VerticalDatum;
use underbite_core::derive::percentile::{DEFAULT_MIN_YEARS, class, from_daily, rank_detail};
use underbite_core::model::{Measurement, Percentile, PercentileBasis, PercentileThresholds};
use underbite_core::provenance::{Provenance, Quality, SourceId, ValueKind};
use underbite_core::time::{LocalDate, UnixTime, UtcOffset, day_key, parse_date};
use underbite_core::units::{Canonical, to_canonical};

use super::Usgs;
use super::ogc::{BASE, site_id, with_key};
use crate::request::{Place, QueryBuilder, Request, Resource, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, SourceError, check_response};

/// The Statistics API endpoint.
pub const STATISTICS_URL: &str = "https://api.waterdata.usgs.gov/statistics/v0/observationNormals";

/// Years of daily means the fallback asks for (about 25,600 values, under the 50,000 limit).
const FALLBACK_YEARS: i32 = 70;

/// Most daily values one page may hold (the API's maximum).
const DAILY_LIMIT: u32 = 50_000;

/// The statistic of a daily mean.
const DAILY_MEAN: &str = "00003";

/// Properties asked of the `daily` fallback.
const DAILY_PROPERTIES: &str = "time_series_id,monitoring_location_id,parameter_code,statistic_id,time,value,unit_of_measure,approval_status,qualifier";

/// A parameter whose normals the app reads.
#[derive(Clone, Copy, Debug)]
struct Series {
    code: &'static str,
    measurement: Measurement,
    /// The unit assumed when a response names none.
    unit: &'static str,
    datum: Option<VerticalDatum>,
}

const SERIES: &[Series] = &[
    Series {
        code: "00060",
        measurement: Measurement::Discharge,
        unit: "ft^3/s",
        datum: None,
    },
    Series {
        code: "62614",
        measurement: Measurement::LakeLevel,
        unit: "ft",
        datum: Some(VerticalDatum::Ngvd29),
    },
    Series {
        code: "62615",
        measurement: Measurement::LakeLevel,
        unit: "ft",
        datum: Some(VerticalDatum::Navd88),
    },
    Series {
        code: "62616",
        measurement: Measurement::LakeLevel,
        unit: "m",
        datum: Some(VerticalDatum::Ngvd29),
    },
    // Reservoir elevation: USGS does not state the datum.
    Series {
        code: "00062",
        measurement: Measurement::LakeLevel,
        unit: "ft",
        datum: Some(VerticalDatum::Gauge),
    },
];

fn series(code: &str) -> Option<&'static Series> {
    SERIES.iter().find(|s| s.code == code)
}

/// A current value to place among the normals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// `Discharge` or `LakeLevel`.
    pub measurement: Measurement,
    /// The datum of a lake level (selects the parameter); ignored for discharge.
    pub datum: Option<VerticalDatum>,
    /// The value in canonical units (m³/s or metres).
    pub value: f32,
    /// When the value was observed.
    pub at: UnixTime,
}

/// What a request carries to its parse. Never sent.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Carry {
    code: String,
    value: f32,
    at: i64,
    /// The site's standard-time offset from UTC, seconds.
    offset_s: i32,
    /// The standard-time day, `MM-DD`.
    day: String,
    /// Set on the `daily` request that stands in for the Statistics API.
    #[serde(default)]
    fallback: bool,
    /// Why the Statistics API was not used.
    #[serde(default)]
    reason: Option<String>,
}

impl Carry {
    fn read(request: &Request) -> Result<Self, SourceError> {
        let text = request
            .carry()
            .ok_or_else(|| SourceError::Parse("statistics request without its reading".into()))?;
        serde_json::from_str(text).map_err(|e| SourceError::Parse(format!("carried reading: {e}")))
    }

    fn text(&self) -> Result<String, SourceError> {
        serde_json::to_string(self).map_err(|e| SourceError::Parse(e.to_string()))
    }

    fn series(&self) -> Result<&'static Series, SourceError> {
        series(&self.code).ok_or_else(|| SourceError::Parse(format!("parameter {}", self.code)))
    }

    fn offset(&self) -> UtcOffset {
        UtcOffset(self.offset_s)
    }
}

impl Usgs {
    /// The request for the normals of the standard-time day of `reading` at the target's site,
    /// one series and one day (1 to 2 KB). `site_offset` is the site's standard-time offset
    /// (`underbite_core::time::standard_offset`): on 8 November at 23:30 EST the day is `11-08`
    /// whatever the UTC date is. Parse the response with [`Source::parse`]; its fragment is a
    /// `Percentile` for the reading's measurement.
    ///
    /// [`Source::parse`]: crate::source::Source::parse
    ///
    /// # Errors
    /// [`SourceError::Unsupported`] when the target is not a station, the measurement has no
    /// normals or the datum matches no USGS parameter.
    pub fn statistics_request(
        &self,
        target: &Target,
        reading: &Reading,
        site_offset: UtcOffset,
        ctx: &FetchContext,
    ) -> Result<Request, SourceError> {
        let Place::Station { station, .. } = &target.place else {
            return Err(SourceError::Unsupported(
                "USGS normals need a station".to_string(),
            ));
        };
        let wanted = |s: &&Series| {
            s.measurement == reading.measurement
                && (reading.measurement != Measurement::LakeLevel
                    || reading.datum.is_none()
                    || s.datum == reading.datum)
        };
        let Some(spec) = SERIES.iter().find(wanted) else {
            return Err(SourceError::Unsupported(format!(
                "no USGS normals for {:?}",
                reading.measurement
            )));
        };
        let day = day_key(reading.at.to_local(site_offset).date);
        let url = QueryBuilder::new(STATISTICS_URL)
            .param("monitoring_location_id", site_id(&station.id))
            .param("parameter_code", spec.code)
            .param("normal_type", "DOY")
            .param("computation_type", "percentile")
            .param("start_date", &day)
            .param("end_date", &day)
            .finish();
        let carry = Carry {
            code: spec.code.to_string(),
            value: reading.value,
            at: reading.at.0,
            offset_s: site_offset.0,
            day,
            fallback: false,
            reason: None,
        };
        let mut wanted_target = target.clone();
        wanted_target.measurements = std::iter::once(spec.measurement).collect();
        Ok(with_key(
            Request::new(url, Resource::UsgsStatistics, wanted_target)
                .optional()
                .metered()
                .max_age_hint(86_400)
                .with_carry(carry.text()?),
            ctx,
        ))
    }
}

/// What a Statistics API response comes to.
enum Normals {
    Done(Box<Percentile>),
    /// Use the daily record instead, for this reason.
    Fallback(String),
    /// Neither path can help (rate limited).
    Stop(SourceError),
}

#[derive(Deserialize)]
struct Collection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: SiteProps,
}

#[derive(Deserialize)]
struct SiteProps {
    monitoring_location_id: Option<String>,
    data: Vec<Parameter>,
}

#[derive(Deserialize)]
struct Parameter {
    #[serde(rename = "parameter_code")]
    code: String,
    unit_of_measure: Option<String>,
    values: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    time_of_year: Option<String>,
    computation: Option<String>,
    #[serde(default)]
    values: Vec<Value>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    percentiles: Vec<Value>,
    sample_count: Option<u32>,
}

/// A number written as a string or a number.
fn number(v: &Value) -> Option<f32> {
    let n = match v {
        Value::String(s) => s.trim().parse::<f32>().ok()?,
        Value::Number(n) => n.as_f64()? as f32,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// `raw` in `unit` as the canonical unit of `spec`'s measurement.
fn canon(spec: &Series, raw: f32, unit: Option<&str>) -> Option<f32> {
    let unit = unit.filter(|u| !u.is_empty()).unwrap_or(spec.unit);
    match (spec.measurement, to_canonical(raw, unit).ok()?) {
        (Measurement::Discharge, Canonical::Discharge(v))
        | (Measurement::LakeLevel, Canonical::Length(v)) => Some(v),
        _ => None,
    }
}

fn provenance(target: &Target, at: i64, quality: Quality) -> Result<Provenance, SourceError> {
    let Place::Station {
        station,
        distance_m,
        relation,
    } = &target.place
    else {
        return Err(SourceError::Unsupported(
            "USGS normals need a station".to_string(),
        ));
    };
    Ok(Provenance {
        source: SourceId::USGS,
        station: Some(station.clone()),
        distance_m: *distance_m,
        relation: *relation,
        observed_at: UnixTime(at),
        kind: ValueKind::Computed,
        quality,
    })
}

/// Reads the normals of the carried day from a response body.
fn read_normals(request: &Request, body: &[u8], carry: &Carry) -> Result<Percentile, String> {
    let spec = carry.series().map_err(|e| e.to_string())?;
    let site = match &request.target.place {
        Place::Station { station, .. } => site_id(&station.id),
        _ => return Err("not a station".to_string()),
    };
    let collection: Collection =
        serde_json::from_slice(body).map_err(|e| format!("unexpected shape: {e}"))?;
    let parameter = collection
        .features
        .iter()
        .filter(|f| {
            f.properties
                .monitoring_location_id
                .as_deref()
                .is_none_or(|id| id == site)
        })
        .flat_map(|f| &f.properties.data)
        .find(|p| p.code == carry.code)
        .ok_or_else(|| format!("no normals for {} at {site}", carry.code))?;
    let unit = parameter.unit_of_measure.as_deref();
    let of_day = parameter
        .values
        .iter()
        .filter(|e| e.time_of_year.as_deref() == Some(carry.day.as_str()));
    let convert = |v: &Value| number(v).and_then(|raw| canon(spec, raw, unit));

    let mut thresholds = PercentileThresholds::default();
    let mut sample_count = None;
    let (mut min, mut max, mut mean) = (None, None, None);
    for entry in of_day {
        match entry.computation.as_deref() {
            Some("percentile") => {
                sample_count = entry.sample_count;
                for (label, value) in entry.percentiles.iter().zip(&entry.values) {
                    let slot = match number(label) {
                        Some(5.0) => &mut thresholds.p05,
                        Some(10.0) => &mut thresholds.p10,
                        Some(25.0) => &mut thresholds.p25,
                        Some(50.0) => &mut thresholds.p50,
                        Some(75.0) => &mut thresholds.p75,
                        Some(90.0) => &mut thresholds.p90,
                        Some(95.0) => &mut thresholds.p95,
                        _ => continue,
                    };
                    *slot = convert(value);
                }
            }
            Some("minimum") => min = entry.value.as_ref().and_then(convert),
            Some("maximum") => max = entry.value.as_ref().and_then(convert),
            Some("arithmetic_mean") => mean = entry.value.as_ref().and_then(convert),
            _ => {}
        }
    }
    if thresholds == PercentileThresholds::default() {
        return Err(format!("no normals for {} on {}", carry.code, carry.day));
    }
    let years = sample_count.unwrap_or(0);
    let ranked = rank_detail(carry.value, &thresholds, min, max);
    let rank = ranked.map(|(r, _)| r);
    Ok(Percentile {
        day_of_year: carry.day.clone(),
        thresholds,
        min,
        max,
        mean,
        value: carry.value,
        rank,
        rank_clamped: ranked.is_some_and(|(_, clamped)| clamped),
        class: rank.and_then(|r| class(r, carry.value, min, max)),
        basis: PercentileBasis::StatisticsApi,
        sample_count: years,
        approved_only: true,
        datum: spec.datum,
        provenance: Some(
            provenance(&request.target, carry.at, Quality::Approved).map_err(|e| e.to_string())?,
        ),
    })
}

fn evaluate(request: &Request, response: &Response<'_>, carry: &Carry) -> Normals {
    match check_response(request, response) {
        Ok(body) => match read_normals(request, body, carry) {
            Ok(p) => Normals::Done(Box::new(p)),
            Err(reason) => Normals::Fallback(reason),
        },
        Err(e @ SourceError::RateLimited { .. }) => Normals::Stop(e),
        Err(e) => Normals::Fallback(e.to_string()),
    }
}

/// The `daily` request that stands in for the Statistics API: daily means of the last
/// [`FALLBACK_YEARS`] years up to the day before the reading's day.
fn fallback_request(
    request: &Request,
    carry: &Carry,
    reason: String,
    ctx: &FetchContext,
) -> Result<Request, SourceError> {
    let Place::Station { station, .. } = &request.target.place else {
        return Err(SourceError::Unsupported(
            "USGS normals need a station".to_string(),
        ));
    };
    let today = today(carry);
    let yesterday = UnixTime(carry.at - 86_400).to_local(carry.offset()).date;
    let date = |d: LocalDate| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
    let start = LocalDate {
        year: today.year - FALLBACK_YEARS,
        month: 1,
        day: 1,
    };
    let url = QueryBuilder::new(&format!("{BASE}/collections/daily/items"))
        .param("f", "json")
        .param("monitoring_location_id", site_id(&station.id))
        .param("parameter_code", &carry.code)
        .param("statistic_id", DAILY_MEAN)
        .param("datetime", format!("{}/{}", date(start), date(yesterday)))
        .param("skipGeometry", "true")
        .param("sortby", "time")
        .param("limit", DAILY_LIMIT.to_string())
        .param("properties", DAILY_PROPERTIES)
        .finish();
    let next = Carry {
        fallback: true,
        reason: Some(reason),
        ..carry.clone()
    };
    Ok(with_key(
        Request::new(url, Resource::UsgsStatistics, request.target.clone())
            .optional()
            .metered()
            .max_age_hint(21_600)
            .with_carry(next.text()?),
        ctx,
    ))
}

/// The standard-time day of the reading.
fn today(carry: &Carry) -> LocalDate {
    UnixTime(carry.at).to_local(carry.offset()).date
}

#[derive(Deserialize)]
struct Daily {
    features: Vec<DailyFeature>,
}

#[derive(Deserialize)]
struct DailyFeature {
    properties: DailyProps,
}

#[derive(Deserialize)]
struct DailyProps {
    time: Option<String>,
    value: Option<Value>,
    unit_of_measure: Option<String>,
}

/// The percentile computed from a `daily` response, today's value left out.
fn compute_from_daily(
    request: &Request,
    body: &[u8],
    carry: &Carry,
) -> Result<Percentile, SourceError> {
    let spec = carry.series()?;
    let daily: Daily = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("daily values: {e}")))?;
    let today = today(carry);
    let mut values: Vec<(LocalDate, f32)> = Vec::new();
    for feature in daily.features {
        let p = feature.properties;
        let (Some(time), Some(raw)) = (p.time.as_deref(), p.value.as_ref().and_then(number)) else {
            continue;
        };
        let date = parse_date(time.get(..10).unwrap_or(time))
            .map_err(|e| SourceError::Parse(format!("daily time {time:?}: {e}")))?;
        if (date.year, date.month, date.day) >= (today.year, today.month, today.day) {
            continue;
        }
        let value = canon(spec, raw, p.unit_of_measure.as_deref())
            .ok_or_else(|| SourceError::Parse(format!("unit for parameter {}", carry.code)))?;
        values.push((date, value));
    }
    let key = (today.month, today.day);
    let computed = from_daily(&values, key, carry.value, DEFAULT_MIN_YEARS);
    let Some(mut percentile) = computed else {
        let years = values
            .iter()
            .filter(|(d, _)| (d.month, d.day) == key)
            .map(|(d, _)| d.year)
            .collect::<BTreeSet<_>>()
            .len();
        return Err(SourceError::NoData(Some(format!(
            "{}computed from daily means: {years} years of {}, {DEFAULT_MIN_YEARS} needed",
            reason_prefix(carry),
            carry.day
        ))));
    };
    percentile.datum = spec.datum;
    percentile.provenance = Some(provenance(&request.target, carry.at, Quality::Provisional)?);
    Ok(percentile)
}

fn reason_prefix(carry: &Carry) -> String {
    carry
        .reason
        .as_deref()
        .map(|r| format!("statistics API: {r}; "))
        .unwrap_or_default()
}

/// Parses a response to a Statistics API request, or to the `daily` request that replaced it.
/// A Statistics API response that fails, changed shape or has no normals gives no value (the
/// follow-up then asks for the daily record); a rate limit is an error.
pub(super) fn parse(
    request: &Request,
    response: &Response<'_>,
    out: &mut Fragments,
) -> Result<(), SourceError> {
    let carry = Carry::read(request)?;
    let spec = carry.series()?;
    let percentile = if carry.fallback {
        let body = check_response(request, response).map_err(|e| match e {
            SourceError::Status { status, message } => SourceError::Status {
                status,
                message: Some(format!(
                    "{}computed from daily means{}",
                    reason_prefix(&carry),
                    message.map(|m| format!(": {m}")).unwrap_or_default()
                )),
            },
            other => other,
        })?;
        compute_from_daily(request, body, &carry)?
    } else {
        match evaluate(request, response, &carry) {
            Normals::Done(p) => *p,
            Normals::Fallback(_) => return Ok(()),
            Normals::Stop(e) => return Err(e),
        }
    };
    out.push(spec.measurement, FragmentValue::Percentile(percentile))
}

/// The follow-up of a Statistics API response: the `daily` request when it gave no normals.
pub(super) fn follow_up(
    request: &Request,
    response: &Response<'_>,
    ctx: &FetchContext,
) -> Result<Vec<Request>, SourceError> {
    let carry = Carry::read(request)?;
    if carry.fallback {
        return Ok(Vec::new());
    }
    match evaluate(request, response, &carry) {
        Normals::Done(_) => Ok(Vec::new()),
        Normals::Fallback(reason) => Ok(vec![fallback_request(request, &carry, reason, ctx)?]),
        Normals::Stop(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use underbite_core::model::{PercentileClass, Requirements};
    use underbite_core::provenance::{Relation, StationKind, StationRef, UsgsSiteType};

    use super::*;
    use crate::request::Window;
    use crate::source::Source;
    use crate::testutil::fixture_bytes;

    const NOW: UnixTime = UnixTime(1_794_148_200); // 2026-11-08T14:30:00Z
    const EST: UtcOffset = UtcOffset(-5 * 3_600);
    const CROTON_NORMALS: &str =
        "usgs-statistics-normals-croton-01375000-00060-doy-percentile-11-05_11-10.json";
    const CFS: f32 = 0.028_316_85;

    fn target(id: &str, measurement: Measurement) -> Target {
        Target {
            place: Place::Station {
                station: StationRef {
                    source: SourceId::USGS,
                    id: id.to_string(),
                    name: "Test site".to_string(),
                    kind: StationKind::UsgsSite(UsgsSiteType::Stream),
                    latitude: 41.225,
                    longitude: -73.859,
                    bin: None,
                    reference_id: None,
                },
                distance_m: Some(1200.0),
                relation: Relation::SameWaterBody,
            },
            measurements: BTreeSet::from([measurement]),
            window: Window {
                from: UnixTime(NOW.0 - 7 * 86_400),
                to: NOW,
            },
            datum: None,
        }
    }

    fn ctx() -> FetchContext {
        FetchContext::new(NOW, "underbite/test")
    }

    fn croton_request(at: UnixTime, value: f32) -> Request {
        let reading = Reading {
            measurement: Measurement::Discharge,
            datum: None,
            value,
            at,
        };
        Usgs::new()
            .statistics_request(
                &target("01375000", Measurement::Discharge),
                &reading,
                EST,
                &ctx(),
            )
            .unwrap()
    }

    fn run(request: &Request, status: u16, body: &[u8]) -> Result<Fragments, SourceError> {
        let mut out = Fragments::new();
        Usgs::new().parse(request, &Response::new(status, body), &ctx(), &mut out)?;
        Ok(out)
    }

    fn percentile(out: &Fragments) -> &Percentile {
        match out.fragments() {
            [f] => match &f.value {
                FragmentValue::Percentile(p) => p,
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn request_asks_for_one_standard_time_day() {
        // 2026-11-09T04:30:00Z is 23:30 EST on 8 November.
        let request = croton_request(UnixTime(1_794_198_600), 0.35);
        let url = &request.url;
        assert!(url.starts_with(STATISTICS_URL), "{url}");
        for part in [
            "monitoring_location_id=USGS-01375000",
            "parameter_code=00060",
            "normal_type=DOY",
            "computation_type=percentile",
            "start_date=11-08",
            "end_date=11-08",
        ] {
            assert!(url.contains(part), "{part} in {url}");
        }
        assert!(request.metered && !request.required);
        assert_eq!(request.resource, Resource::UsgsStatistics);
    }

    #[test]
    fn croton_six_days_and_the_eighth() {
        let body = fixture_bytes("usgs", CROTON_NORMALS);
        let mut days = Vec::new();
        for day in 5..=10_i64 {
            let at = UnixTime(NOW.0 + (day - 8) * 86_400);
            let out = run(&croton_request(at, 12.4 * CFS), 200, &body).unwrap();
            days.push(percentile(&out).day_of_year.clone());
        }
        assert_eq!(days, ["11-05", "11-06", "11-07", "11-08", "11-09", "11-10"]);

        let out = run(&croton_request(NOW, 12.4 * CFS), 200, &body).unwrap();
        let p = percentile(&out);
        let t = &p.thresholds;
        assert!((t.p05.unwrap() - 0.634 * CFS).abs() < 1e-4);
        assert!((t.p50.unwrap() - 15.0 * CFS).abs() < 1e-4);
        assert!((t.p95.unwrap() - 1180.0 * CFS).abs() < 1e-2);
        assert_eq!(p.sample_count, 93);
        assert_eq!(p.basis, PercentileBasis::StatisticsApi);
        assert!(p.approved_only);
        assert_eq!(p.datum, None);
        assert_eq!(p.class, Some(PercentileClass::Normal));
        let rank = p.rank.unwrap();
        assert!((25.0..50.0).contains(&rank), "{rank}");
        let prov = p.provenance.as_ref().unwrap();
        assert_eq!(prov.source, SourceId::USGS);
        assert_eq!(prov.observed_at, NOW);
        insta::assert_yaml_snapshot!("croton_normals_1108", p);
    }

    #[test]
    fn lake_median_is_in_metres_ngvd29() {
        let reading = Reading {
            measurement: Measurement::LakeLevel,
            datum: Some(VerticalDatum::Ngvd29),
            value: 755.49 * 0.3048,
            at: NOW,
        };
        let request = Usgs::new()
            .statistics_request(
                &target("01323500", Measurement::LakeLevel),
                &reading,
                EST,
                &ctx(),
            )
            .unwrap();
        assert!(request.url.contains("parameter_code=62614"));
        let body = fixture_bytes(
            "usgs",
            "usgs-statistics-normals-great-sacandaga-01323500-62614-doy-percentile-11-08.json",
        );
        let out = run(&request, 200, &body).unwrap();
        let p = percentile(&out);
        assert!((p.thresholds.p50.unwrap() - 754.97 * 0.3048).abs() < 1e-2);
        assert_eq!(p.datum, Some(VerticalDatum::Ngvd29));
        assert_eq!(p.sample_count, 96);
        assert_eq!(out.fragments()[0].measurement, Measurement::LakeLevel);
    }

    #[test]
    fn datum_without_a_parameter_is_unsupported() {
        let reading = Reading {
            measurement: Measurement::LakeLevel,
            datum: Some(VerticalDatum::Mllw),
            value: 1.0,
            at: NOW,
        };
        let err = Usgs::new()
            .statistics_request(
                &target("01323500", Measurement::LakeLevel),
                &reading,
                EST,
                &ctx(),
            )
            .unwrap_err();
        assert!(matches!(err, SourceError::Unsupported(_)), "{err:?}");
    }

    const CHANGED_SHAPE: &str = r#"{"type":"FeatureCollection","features":[{"properties":{"monitoring_location_id":"USGS-01375000","data":[{"parameter_code":"00060","unit_of_measure":"ft^3/s","values":{"11-08":{"p50":15.0}}}]}}],"next":null}"#;

    #[test]
    fn changed_shape_falls_back_to_the_daily_record() {
        let request = croton_request(NOW, 0.35);
        let response = Response::new(200, CHANGED_SHAPE.as_bytes());
        let mut out = Fragments::new();
        Usgs::new()
            .parse(&request, &response, &ctx(), &mut out)
            .unwrap();
        assert!(out.is_empty());
        let next = Usgs::new().follow_up(&request, &response, &ctx()).unwrap();
        let [daily] = next.as_slice() else {
            panic!("{next:?}");
        };
        let url = &daily.url;
        assert!(url.contains("/collections/daily/items"), "{url}");
        for part in [
            "parameter_code=00060",
            "statistic_id=00003",
            "sortby=time",
            "limit=50000",
            "datetime=1956-01-01%2F2026-11-07",
        ] {
            assert!(url.contains(part), "{part} in {url}");
        }
        assert!(daily.metered && !daily.required);
        // The fallback does not chain.
        let again = Usgs::new()
            .follow_up(daily, &Response::new(200, b"{}"), &ctx())
            .unwrap();
        assert_eq!(again.len(), 0);
    }

    #[test]
    fn failures_and_missing_normals_fall_back_but_a_rate_limit_does_not() {
        let request = croton_request(NOW, 0.35);
        let falls_back = |status: u16, body: &[u8]| {
            let response = Response::new(status, body);
            let mut out = Fragments::new();
            Usgs::new()
                .parse(&request, &response, &ctx(), &mut out)
                .unwrap();
            assert!(out.is_empty());
            Usgs::new()
                .follow_up(&request, &response, &ctx())
                .unwrap()
                .len()
        };
        assert_eq!(falls_back(500, b"Internal Server Error"), 1);
        assert_eq!(falls_back(404, b"{\"message\":\"nothing\"}"), 1);
        assert_eq!(falls_back(200, br#"{"features":[],"next":null}"#), 1);
        let empty = br#"{"features":[{"properties":{"monitoring_location_id":"USGS-01375000","data":[{"parameter_code":"00060","unit_of_measure":"ft^3/s","values":[{"time_of_year":"11-08","computation":"percentile","values":[null,null],"percentiles":["5","10"],"sample_count":3}]}]}}],"next":null}"#;
        assert_eq!(falls_back(200, empty), 1);
        let limited = Response::new(429, b"slow down");
        let mut out = Fragments::new();
        let err = Usgs::new()
            .parse(&request, &limited, &ctx(), &mut out)
            .unwrap_err();
        assert!(matches!(err, SourceError::RateLimited { .. }), "{err:?}");
    }

    /// A `daily` body with the 8 November mean of each of `years`, today's, and a `null`.
    fn daily_body(years: std::ops::RangeInclusive<i32>) -> String {
        let mut features: Vec<String> = years
            .map(|y| {
                let value = 1.0 + f64::from(y - 2000) * 4.0;
                format!(
                    r#"{{"properties":{{"time":"{y}-11-08","value":"{value}","unit_of_measure":"ft^3/s"}}}}"#
                )
            })
            .collect();
        features.push(
            r#"{"properties":{"time":"2026-11-08","value":"500.0","unit_of_measure":"ft^3/s"}}"#
                .to_string(),
        );
        features.push(
            r#"{"properties":{"time":"2020-11-09","value":null,"unit_of_measure":"ft^3/s"}}"#
                .to_string(),
        );
        format!(r#"{{"features":[{}]}}"#, features.join(","))
    }

    fn daily_request() -> Request {
        let request = croton_request(NOW, 0.35);
        let response = Response::new(200, CHANGED_SHAPE.as_bytes());
        Usgs::new()
            .follow_up(&request, &response, &ctx())
            .unwrap()
            .remove(0)
    }

    #[test]
    fn daily_fallback_computes_thresholds_without_today() {
        let body = daily_body(2005..=2025);
        let out = run(&daily_request(), 200, body.as_bytes()).unwrap();
        let p = percentile(&out);
        assert_eq!(p.basis, PercentileBasis::ComputedFromDaily { years: 21 });
        assert!(!p.approved_only);
        assert_eq!(p.day_of_year, "11-08");
        // Today's 500 cfs is not in the sample: the maximum is the 2025 value, 101 cfs.
        assert!((p.max.unwrap() - 101.0 * CFS).abs() < 1e-3, "{:?}", p.max);
        assert_eq!(p.provenance.as_ref().unwrap().quality, Quality::Provisional);
        assert!(p.rank.is_some());
    }

    #[test]
    fn one_year_of_daily_values_is_not_enough() {
        let body = fixture_bytes(
            "usgs",
            "usgs-daily-croton-01375000-00060-mean-2025-11-08_2026-11-07.json",
        );
        let err = run(&daily_request(), 200, &body).unwrap_err();
        let SourceError::NoData(Some(message)) = &err else {
            panic!("{err:?}");
        };
        assert!(message.contains("computed from daily means"), "{message}");
        assert!(message.contains("1 years of 11-08"), "{message}");
        assert!(
            message.starts_with("statistics API: unexpected shape"),
            "{message}"
        );
    }

    #[test]
    fn plan_for_a_site_does_not_ask_for_normals() {
        let plan = Usgs::new()
            .plan(
                &target("01375000", Measurement::Discharge),
                &Requirements::default(),
                &ctx(),
            )
            .unwrap();
        assert!(plan.iter().all(|r| r.resource != Resource::UsgsStatistics));
    }
}
