//! USGS Water Data OGC API: monitoring locations and time series values.
//!
//! `latest-continuous` gives the newest value of every series of a site, `continuous` the values
//! of one series over a window and `daily` daily statistics. Values are strings, `null` when
//! missing; `qualifier` is an array or `null`. Responses are `GeoJSON` `FeatureCollection` objects.

use std::cmp::Reverse;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use underbite_core::datum::{Heights, VerticalDatum};
use underbite_core::model::{Measurement, Turbidity, TurbidityUnit};
use underbite_core::provenance::{Attribution, Provenance, Quality, SourceId, ValueKind};
use underbite_core::series::{Sample, Series};
use underbite_core::time::{UnixTime, UtcOffset, parse_date, parse_rfc3339};
use underbite_core::units::{Canonical, to_canonical};

use super::Usgs;
use crate::request::{Place, QueryBuilder, Request, Resource, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, SourceError, check_response};

/// The API's base URL (version 1).
pub(super) const BASE: &str = "https://api.waterdata.usgs.gov/ogcapi/v1";

/// The statistic of an instantaneous series.
const INSTANTANEOUS: &str = "00011";

/// Properties asked of `latest-continuous`.
const LATEST_PROPERTIES: &str = "time_series_id,monitoring_location_id,parameter_code,statistic_id,time,value,unit_of_measure,approval_status,qualifier";

/// Most features one `latest-continuous` response may hold (one per series; a site has about 25).
const LATEST_LIMIT: u32 = 1000;

/// What a parameter's canonical value looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Discharge,
    Length,
    Temperature,
    Conductivity,
    Salinity,
    MgPerL,
    Ph,
    Turbidity,
}

/// A USGS parameter code the app reads.
#[derive(Clone, Copy, Debug)]
struct Param {
    code: &'static str,
    measurement: Measurement,
    kind: Kind,
    /// The unit assumed when a response names none.
    unit: &'static str,
    /// The datum of a height.
    datum: Option<VerticalDatum>,
}

const fn param(
    code: &'static str,
    measurement: Measurement,
    kind: Kind,
    unit: &'static str,
    datum: Option<VerticalDatum>,
) -> Param {
    Param {
        code,
        measurement,
        kind,
        unit,
        datum,
    }
}

/// Parameter codes by measurement. A parameter of the same measurement in another datum is a
/// separate row; [`candidates`] orders them.
const PARAMS: &[Param] = &[
    param(
        "00060",
        Measurement::Discharge,
        Kind::Discharge,
        "ft^3/s",
        None,
    ),
    param(
        "00065",
        Measurement::GageHeight,
        Kind::Length,
        "ft",
        Some(VerticalDatum::Gauge),
    ),
    param(
        "00010",
        Measurement::WaterTemperature,
        Kind::Temperature,
        "degC",
        None,
    ),
    param(
        "63680",
        Measurement::Turbidity,
        Kind::Turbidity,
        "_FNU",
        None,
    ),
    param(
        "00300",
        Measurement::DissolvedOxygen,
        Kind::MgPerL,
        "mg/l",
        None,
    ),
    param(
        "00095",
        Measurement::Conductivity,
        Kind::Conductivity,
        "uS/cm",
        None,
    ),
    param("00400", Measurement::Ph, Kind::Ph, "pH Units", None),
    param("90860", Measurement::Salinity, Kind::Salinity, "PSU", None),
    param(
        "62614",
        Measurement::LakeLevel,
        Kind::Length,
        "ft",
        Some(VerticalDatum::Ngvd29),
    ),
    param(
        "62615",
        Measurement::LakeLevel,
        Kind::Length,
        "ft",
        Some(VerticalDatum::Navd88),
    ),
    param(
        "62616",
        Measurement::LakeLevel,
        Kind::Length,
        "m",
        Some(VerticalDatum::Ngvd29),
    ),
    // Reservoir elevation: USGS does not state the datum.
    param(
        "00062",
        Measurement::LakeLevel,
        Kind::Length,
        "ft",
        Some(VerticalDatum::Gauge),
    ),
    param(
        "62620",
        Measurement::Tide,
        Kind::Length,
        "ft",
        Some(VerticalDatum::Navd88),
    ),
    param(
        "62619",
        Measurement::Tide,
        Kind::Length,
        "ft",
        Some(VerticalDatum::Ngvd29),
    ),
];

/// The measurements the source supplies.
pub(super) const MEASUREMENTS: &[Measurement] = &[
    Measurement::Discharge,
    Measurement::GageHeight,
    Measurement::LakeLevel,
    Measurement::WaterTemperature,
    Measurement::Turbidity,
    Measurement::DissolvedOxygen,
    Measurement::Conductivity,
    Measurement::Salinity,
    Measurement::Ph,
    Measurement::Tide,
];

fn spec(code: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.code == code)
}

/// The parameters that can stand for `measurement`, best first: those in the wanted datum, then
/// NAVD88 before NGVD29 before the unstated reservoir datum.
fn candidates(measurement: Measurement, wanted: Option<VerticalDatum>) -> Vec<&'static Param> {
    let mut found: Vec<&Param> = PARAMS
        .iter()
        .filter(|p| p.measurement == measurement)
        .collect();
    let rank = |p: &Param| match p.datum {
        d if d.is_some() && d == wanted => 0,
        Some(VerticalDatum::Navd88) => 1,
        Some(VerticalDatum::Ngvd29) => 2,
        _ => 3,
    };
    found.sort_by_key(|p| rank(p));
    found
}

/// The credit every display of USGS values shows.
pub(super) fn attribution() -> Attribution {
    Attribution {
        source: SourceId::USGS,
        credit: "U.S. Geological Survey".to_string(),
        url: Some("https://waterdata.usgs.gov".to_string()),
    }
}

/// The notice that goes with provisional values.
fn provisional_notice() -> Attribution {
    Attribution {
        source: SourceId::USGS,
        credit: "Provisional data from the U.S. Geological Survey are subject to revision"
            .to_string(),
        url: Some("https://waterdata.usgs.gov/provisional-data-statement/".to_string()),
    }
}

/// `USGS-<number>` for a station id written either way.
pub(super) fn site_id(station_id: &str) -> String {
    if station_id.starts_with("USGS-") {
        station_id.to_string()
    } else {
        format!("USGS-{station_id}")
    }
}

/// The station of a target.
fn station_site(target: &Target) -> Result<String, SourceError> {
    match &target.place {
        Place::Station { station, .. } => Ok(site_id(&station.id)),
        _ => Err(SourceError::Unsupported(
            "USGS values need a station".to_string(),
        )),
    }
}

/// Adds the key header when a key is configured. It never goes in the URL.
pub(super) fn with_key(request: Request, ctx: &FetchContext) -> Request {
    match &ctx.secrets().usgs_api_key {
        Some(key) => request.header("X-Api-Key", key.clone()),
        None => request,
    }
}

/// A `latest-continuous` request for `codes` at `sites`.
pub fn latest_request(
    sites: &[String],
    codes: &[&str],
    target: &Target,
    ctx: &FetchContext,
) -> Request {
    let url = QueryBuilder::new(&format!("{BASE}/collections/latest-continuous/items"))
        .param("f", "json")
        .param("monitoring_location_id", sites.join(","))
        .param("parameter_code", codes.join(","))
        .param("skipGeometry", "true")
        .param("properties", LATEST_PROPERTIES)
        .param("limit", LATEST_LIMIT.to_string())
        .finish();
    with_key(
        Request::new(url, Resource::UsgsLatestContinuous, target.clone())
            .metered()
            .max_age_hint(300),
        ctx,
    )
}

/// The requests for `target`: the newest value of every wanted parameter.
pub(super) fn plan(target: &Target, ctx: &FetchContext) -> Result<Vec<Request>, SourceError> {
    let site = station_site(target)?;
    let mut codes: Vec<&str> = Vec::new();
    for measurement in &target.measurements {
        for p in PARAMS.iter().filter(|p| p.measurement == *measurement) {
            codes.push(p.code);
        }
    }
    if codes.is_empty() {
        return Err(SourceError::Unsupported(
            "no USGS parameter for the measurements".to_string(),
        ));
    }
    Ok(vec![latest_request(&[site], &codes, target, ctx)])
}

#[derive(Deserialize)]
struct Collection {
    features: Vec<Feature>,
    #[serde(default)]
    links: Vec<Link>,
}

#[derive(Deserialize)]
struct Link {
    rel: Option<String>,
    href: Option<String>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Props,
}

#[derive(Deserialize, Default)]
struct Props {
    time_series_id: Option<String>,
    monitoring_location_id: Option<String>,
    parameter_code: Option<String>,
    statistic_id: Option<String>,
    time: Option<String>,
    value: Option<Value>,
    unit_of_measure: Option<String>,
    approval_status: Option<String>,
    qualifier: Option<Vec<String>>,
    primary: Option<String>,
}

fn parse_error(what: impl std::fmt::Display) -> SourceError {
    SourceError::Parse(what.to_string())
}

/// The quality of a value: a qualifier beats the approval status. `EQUIP`, `MAINT`, `SEASONAL`
/// and `DISCONTINUED` flag it, `ICE` marks ice, `ESTIMATED` an estimate; other codes are ignored.
pub(super) fn quality(approval: Option<&str>, qualifiers: &[String]) -> Quality {
    let has = |codes: &[&str]| qualifiers.iter().any(|q| codes.contains(&q.as_str()));
    if has(&["EQUIP", "MAINT", "SEASONAL", "DISCONTINUED"]) {
        Quality::Flagged
    } else if has(&["ICE"]) {
        Quality::IceAffected
    } else if has(&["ESTIMATED"]) {
        Quality::Estimated
    } else if approval == Some("Approved") {
        Quality::Approved
    } else {
        Quality::Provisional
    }
}

/// One value in canonical units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Point {
    pub(super) t: i64,
    pub(super) v: f32,
    pub(super) q: Quality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) unit: Option<TurbidityUnit>,
}

/// What a request for a series carries to its parse: the parameter (the paged responses name
/// none) and the points of the pages read so far.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Carry {
    parameter_code: String,
    points: Vec<Point>,
}

impl Carry {
    fn text(&self) -> Result<String, SourceError> {
        serde_json::to_string(self).map_err(parse_error)
    }
}

fn canonical(kind: Kind, value: Canonical) -> Option<(f32, Option<TurbidityUnit>)> {
    Some(match (kind, value) {
        (Kind::Discharge, Canonical::Discharge(v))
        | (Kind::Length, Canonical::Length(v))
        | (Kind::Temperature, Canonical::Temperature(v))
        | (Kind::Conductivity, Canonical::Conductivity(v))
        | (Kind::Salinity, Canonical::Salinity(v))
        | (Kind::MgPerL, Canonical::ConcentrationMgPerL(v))
        | (Kind::Ph, Canonical::Ph(v)) => (v, None),
        (Kind::Turbidity, Canonical::Turbidity(v, u)) => (v, Some(u)),
        _ => return None,
    })
}

/// The value of a feature at `t`, or `None` when it is `null`.
fn point(props: &Props, p: &Param, t: UnixTime) -> Result<Option<Point>, SourceError> {
    let text = match &props.value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => return Err(parse_error(format!("value {other}"))),
    };
    let raw: f32 = text
        .parse()
        .ok()
        .filter(|v: &f32| v.is_finite())
        .ok_or_else(|| parse_error(format!("value {text:?}")))?;
    let unit = props
        .unit_of_measure
        .as_deref()
        .filter(|u| !u.is_empty())
        .unwrap_or(p.unit);
    let canon = to_canonical(raw, unit).map_err(parse_error)?;
    let (v, turbidity) = canonical(p.kind, canon)
        .ok_or_else(|| parse_error(format!("unit {unit:?} for parameter {}", p.code)))?;
    Ok(Some(Point {
        t: t.0,
        v,
        q: quality(
            props.approval_status.as_deref(),
            props.qualifier.as_deref().unwrap_or(&[]),
        ),
        unit: turbidity,
    }))
}

fn provenance(
    target: &Target,
    observed_at: i64,
    kind: ValueKind,
    quality: Quality,
) -> Result<Provenance, SourceError> {
    let Place::Station {
        station,
        distance_m,
        relation,
    } = &target.place
    else {
        return Err(SourceError::Unsupported(
            "USGS values need a station".to_string(),
        ));
    };
    Ok(Provenance {
        source: SourceId::USGS,
        station: Some(station.clone()),
        distance_m: *distance_m,
        relation: *relation,
        observed_at: UnixTime(observed_at),
        kind,
        quality,
    })
}

/// The fragment for `points` of parameter `p`; adds the provisional notice when needed.
fn fragment(
    p: &Param,
    mut points: Vec<Point>,
    target: &Target,
    kind: ValueKind,
    out: &mut Fragments,
) -> Result<(), SourceError> {
    points.sort_by_key(|pt| pt.t);
    let Some(last) = points.last() else {
        return Ok(());
    };
    let prov = provenance(target, last.t, kind, last.q)?;
    if points.iter().any(|pt| pt.q == Quality::Provisional) {
        out.credit(provisional_notice());
    }
    let value = if p.kind == Kind::Turbidity {
        let items = samples(&points, |pt| {
            Some(Turbidity {
                value: pt.v,
                unit: pt.unit?,
            })
        });
        FragmentValue::Turbidity(Series::new(prov, items))
    } else if let Some(datum) = p.datum {
        FragmentValue::Heights(Heights {
            datum,
            series: Series::new(prov, samples(&points, |pt| Some(pt.v))),
        })
    } else {
        FragmentValue::Series(Series::new(prov, samples(&points, |pt| Some(pt.v))))
    };
    out.push(p.measurement, value)
}

/// `points` as samples of the values `value` makes of them.
fn samples<T>(points: &[Point], value: impl Fn(&Point) -> Option<T>) -> Vec<Sample<T>> {
    points
        .iter()
        .filter_map(|pt| {
            value(pt).map(|value| Sample {
                time: UnixTime(pt.t),
                value,
                quality: Some(pt.q),
            })
        })
        .collect()
}

/// The newest value of one series of the response.
#[derive(Clone, Debug)]
pub(super) struct Latest {
    param: &'static Param,
    series_id: Option<String>,
    statistic: Option<String>,
    primary: bool,
    pub(super) point: Point,
    /// Older than the freshness limit: a dead series.
    pub(super) stale: bool,
}

/// Every series of `site` in a `latest-continuous` body whose parameter the app reads and whose
/// value is not `null`, marked stale when older than `max_age_s` before `now`.
pub(super) fn read_latest(
    body: &[u8],
    site: &str,
    now: UnixTime,
    max_age_s: i64,
) -> Result<Vec<Latest>, SourceError> {
    let collection: Collection = serde_json::from_slice(body).map_err(parse_error)?;
    let mut found = Vec::new();
    for feature in collection.features {
        let props = feature.properties;
        if props
            .monitoring_location_id
            .as_deref()
            .is_some_and(|id| id != site)
        {
            continue;
        }
        let Some(p) = props.parameter_code.as_deref().and_then(spec) else {
            continue;
        };
        let time = props
            .time
            .as_deref()
            .and_then(parse_rfc3339)
            .ok_or_else(|| parse_error("time of a series"))?;
        let Some(point) = point(&props, p, time)? else {
            continue;
        };
        found.push(Latest {
            param: p,
            series_id: props.time_series_id,
            statistic: props.statistic_id,
            primary: props.primary.as_deref() == Some("Primary"),
            stale: now.0.saturating_sub(time.0) > max_age_s,
            point,
        });
    }
    Ok(found)
}

/// The series that stand for the target's measurements: per measurement the best parameter that
/// has a live series, and of its series the instantaneous one, then the primary, then the
/// newest. Also the newest time among series dropped for age.
pub(super) fn select<'a>(
    readings: &'a [Latest],
    target: &Target,
) -> (Vec<(Measurement, &'a Latest)>, Option<i64>) {
    let mut chosen = Vec::new();
    let mut newest_stale: Option<i64> = None;
    for &measurement in &target.measurements {
        let mut pick = None;
        for p in candidates(measurement, target.datum) {
            let of_param = readings.iter().filter(|r| std::ptr::eq(r.param, p));
            let (live, dead): (Vec<_>, Vec<_>) = of_param.partition(|r| !r.stale);
            newest_stale = dead.iter().map(|r| r.point.t).chain(newest_stale).max();
            pick = live.into_iter().min_by_key(|r| {
                (
                    r.statistic.as_deref() != Some(INSTANTANEOUS),
                    !r.primary,
                    Reverse(r.point.t),
                )
            });
            if pick.is_some() {
                break;
            }
        }
        if let Some(r) = pick {
            chosen.push((measurement, r));
        }
    }
    (chosen, newest_stale)
}

fn parse_latest(
    request: &Request,
    body: &[u8],
    ctx: &FetchContext,
    usgs: &Usgs,
    out: &mut Fragments,
) -> Result<(), SourceError> {
    let site = station_site(&request.target)?;
    let readings = read_latest(body, &site, ctx.now, usgs.max_age_s)?;
    let (chosen, newest_stale) = select(&readings, &request.target);
    if chosen.is_empty() {
        return Err(match newest_stale {
            Some(t) => SourceError::Stale {
                latest: UnixTime(t),
            },
            None => SourceError::NoData(Some("no values for the wanted parameters".to_string())),
        });
    }
    for (_, r) in chosen {
        fragment(
            r.param,
            vec![r.point.clone()],
            &request.target,
            ValueKind::Observed,
            out,
        )?;
    }
    Ok(())
}

/// Parses a response to a USGS values request into `out`.
pub(super) fn parse(
    usgs: &Usgs,
    request: &Request,
    response: &Response<'_>,
    ctx: &FetchContext,
    out: &mut Fragments,
) -> Result<(), SourceError> {
    let body = check_response(request, response)?;
    match request.resource {
        Resource::UsgsLatestContinuous => parse_latest(request, body, ctx, usgs, out),
        Resource::UsgsContinuous | Resource::UsgsDaily => parse_values(request, body, out),
        other => Err(SourceError::Unsupported(format!("{other:?}"))),
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
fn format_time(t: UnixTime) -> String {
    let l = t.to_local(UtcOffset(0));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        l.date.year, l.date.month, l.date.day, l.hour, l.minute, l.second
    )
}

/// The longest span `continuous` answers for, a little under three years.
const MAX_CONTINUOUS_SPAN_S: i64 = 1_094 * 86_400;

/// Most values per `continuous` page; a week at 15 minutes is 673.
const CONTINUOUS_LIMIT: u32 = 2000;

/// Properties asked of `continuous`.
const CONTINUOUS_PROPERTIES: &str =
    "time_series_id,parameter_code,time,value,unit_of_measure,approval_status,qualifier";

/// A `continuous` request for one series over the target's window (at most three years back from
/// its end), for the measurement of parameter `p`. Carries the parameter to the parse.
fn continuous_request(
    series_id: &str,
    p: &Param,
    target: &Target,
    ctx: &FetchContext,
) -> Result<Request, SourceError> {
    let to = target.window.to;
    let from = target.window.from.0.max(to.0 - MAX_CONTINUOUS_SPAN_S);
    let url = QueryBuilder::new(&format!("{BASE}/collections/continuous/items"))
        .param("f", "json")
        .param("time_series_id", series_id)
        .param(
            "datetime",
            format!("{}/{}", format_time(UnixTime(from)), format_time(to)),
        )
        .param("skipGeometry", "true")
        .param("sortby", "time")
        .param("limit", CONTINUOUS_LIMIT.to_string())
        .param("properties", CONTINUOUS_PROPERTIES)
        .finish();
    let mut wanted = target.clone();
    wanted.measurements = std::iter::once(p.measurement).collect();
    let carry = Carry {
        parameter_code: p.code.to_string(),
        points: Vec::new(),
    };
    Ok(with_key(
        Request::new(url, Resource::UsgsContinuous, wanted)
            .metered()
            .max_age_hint(300)
            .with_carry(carry.text()?),
        ctx,
    ))
}

/// The values of a `continuous` page or a `daily` page, appended to what the pages before it
/// carried. Values are of the carried parameter; `null` values are skipped.
fn read_values(request: &Request, body: &[u8]) -> Result<(&'static Param, Carry), SourceError> {
    let mut carry: Carry = match request.carry() {
        Some(text) => serde_json::from_str(text).map_err(parse_error)?,
        None => Carry::default(),
    };
    let site = station_site(&request.target)?;
    let collection: Collection = serde_json::from_slice(body).map_err(parse_error)?;
    if carry.parameter_code.is_empty() {
        carry.parameter_code = collection
            .features
            .iter()
            .find_map(|f| f.properties.parameter_code.clone())
            .ok_or_else(|| parse_error("no parameter code"))?;
    }
    let p = spec(&carry.parameter_code)
        .ok_or_else(|| parse_error(format!("parameter {}", carry.parameter_code)))?;
    for feature in collection.features {
        let props = feature.properties;
        let other_site = props
            .monitoring_location_id
            .as_deref()
            .is_some_and(|id| id != site);
        let other_parameter = props.parameter_code.as_deref().is_some_and(|c| c != p.code);
        if other_site || other_parameter {
            continue;
        }
        let time = value_time(request.resource, props.time.as_deref())?;
        carry.points.extend(point(&props, p, time)?);
    }
    Ok((p, carry))
}

fn parse_values(request: &Request, body: &[u8], out: &mut Fragments) -> Result<(), SourceError> {
    let (p, carry) = read_values(request, body)?;
    if carry.points.is_empty() {
        return Err(SourceError::NoData(Some(
            "no values in the window".to_string(),
        )));
    }
    let kind = if request.resource == Resource::UsgsDaily {
        ValueKind::Computed
    } else {
        ValueKind::Observed
    };
    fragment(p, carry.points, &request.target, kind, out)
}

/// The time of a value of `resource`. A `continuous` time is an instant. A `daily` time is a
/// calendar date, and the value is the mean over that date in the site's standard time; it is
/// stamped nominally at 12:00 UTC of the date, which keeps the date in every US zone from UTC-4
/// to UTC-10. Consumers use the date (`underbite_core::time::local_date` with UTC), for
/// instance for day-of-year keys, never the instant.
fn value_time(resource: Resource, text: Option<&str>) -> Result<UnixTime, SourceError> {
    let text = text.ok_or_else(|| parse_error("value without a time"))?;
    if resource == Resource::UsgsDaily {
        let date = parse_date(text).map_err(parse_error)?;
        Ok(UnixTime(date.days_since_epoch() * 86_400 + 12 * 3_600))
    } else {
        parse_rfc3339(text).ok_or_else(|| parse_error(format!("time {text:?}")))
    }
}

/// Properties asked of `daily`.
const DAILY_PROPERTIES: &str = "time_series_id,monitoring_location_id,parameter_code,statistic_id,time,value,unit_of_measure,approval_status,qualifier";

/// Most values per `daily` page (the API's maximum): 50 years of days.
const DAILY_LIMIT: u32 = 50_000;

/// The statistic of a daily mean.
const DAILY_MEAN: &str = "00003";

/// `daily` requests for the target's measurements over its window, one per measurement, each for
/// the parameter that best matches `target.datum`. Values are daily means, stamped nominally at
/// 12:00 UTC of their date (see [`value_time`]).
pub(super) fn daily_requests(
    target: &Target,
    ctx: &FetchContext,
) -> Result<Vec<Request>, SourceError> {
    let site = station_site(target)?;
    let from = target.window.from.to_local(UtcOffset(0)).date;
    let to = target.window.to.to_local(UtcOffset(0)).date;
    let date =
        |d: underbite_core::time::LocalDate| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
    let mut requests = Vec::new();
    for &measurement in &target.measurements {
        let Some(p) = candidates(measurement, target.datum).into_iter().next() else {
            continue;
        };
        let url = QueryBuilder::new(&format!("{BASE}/collections/daily/items"))
            .param("f", "json")
            .param("monitoring_location_id", &site)
            .param("parameter_code", p.code)
            .param("statistic_id", DAILY_MEAN)
            .param("datetime", format!("{}/{}", date(from), date(to)))
            .param("skipGeometry", "true")
            .param("sortby", "time")
            .param("limit", DAILY_LIMIT.to_string())
            .param("properties", DAILY_PROPERTIES)
            .finish();
        let mut wanted = target.clone();
        wanted.measurements = std::iter::once(measurement).collect();
        let carry = Carry {
            parameter_code: p.code.to_string(),
            points: Vec::new(),
        };
        requests.push(with_key(
            Request::new(url, Resource::UsgsDaily, wanted)
                .metered()
                .max_age_hint(3_600)
                .with_carry(carry.text()?),
            ctx,
        ));
    }
    if requests.is_empty() {
        return Err(SourceError::Unsupported(
            "no USGS parameter for the measurements".to_string(),
        ));
    }
    Ok(requests)
}

/// The `next` link of a page, if it has one and it stays on the API.
fn next_link(body: &[u8]) -> Result<Option<String>, SourceError> {
    let collection: Collection = serde_json::from_slice(body).map_err(parse_error)?;
    let href = collection
        .links
        .into_iter()
        .find(|l| l.rel.as_deref() == Some("next"))
        .and_then(|l| l.href);
    match href {
        Some(h) if h.starts_with(&format!("{BASE}/collections/")) => Ok(Some(h)),
        Some(h) => Err(parse_error(format!("next link off the API: {h}"))),
        None => Ok(None),
    }
}

/// What a response calls for next: from `latest-continuous`, the recent values of each series
/// it chose; from a page of values, the next page (carrying the points read so far).
pub(super) fn follow_up(
    usgs: &Usgs,
    request: &Request,
    response: &Response<'_>,
    ctx: &FetchContext,
) -> Result<Vec<Request>, SourceError> {
    let body = check_response(request, response)?;
    let target = &request.target;
    match request.resource {
        Resource::UsgsLatestContinuous => {
            let window = target.window;
            if window.to <= window.from || window.from.0 >= ctx.now.0 - 3_600 {
                return Ok(Vec::new());
            }
            let readings = read_latest(body, &station_site(target)?, ctx.now, usgs.max_age_s)?;
            let (chosen, _) = select(&readings, target);
            chosen
                .into_iter()
                .filter_map(|(_, r)| Some((r.series_id.as_deref()?, r.param)))
                .map(|(id, p)| continuous_request(id, p, target, ctx))
                .collect()
        }
        Resource::UsgsContinuous | Resource::UsgsDaily => {
            let Some(href) = next_link(body)? else {
                return Ok(Vec::new());
            };
            let (_, carry) = read_values(request, body)?;
            let mut next =
                Request::new(href, request.resource, target.clone()).with_carry(carry.text()?);
            next.headers.clone_from(&request.headers);
            next.cache = request.cache;
            next.required = request.required;
            next.metered = request.metered;
            Ok(vec![next])
        }
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use underbite_core::model::Requirements;
    use underbite_core::provenance::{Relation, StationKind, StationRef};

    use super::*;
    use crate::request::Window;
    use crate::source::{FragmentValue as V, Secrets, Source};
    use crate::testutil::fixture_bytes;

    pub(super) const NOW: UnixTime = UnixTime(1_794_148_200); // 2026-11-08T14:30:00Z

    pub(super) fn station(id: &str) -> Place {
        Place::Station {
            station: StationRef {
                source: SourceId::USGS,
                id: id.to_string(),
                name: "Test site".to_string(),
                kind: StationKind::UsgsSite(underbite_core::provenance::UsgsSiteType::Stream),
                latitude: 41.225,
                longitude: -73.859,
                bin: None,
                reference_id: None,
            },
            distance_m: Some(1200.0),
            relation: Relation::SameWaterBody,
        }
    }

    pub(super) fn target(id: &str, measurements: &[Measurement]) -> Target {
        Target {
            place: station(id),
            measurements: measurements.iter().copied().collect::<BTreeSet<_>>(),
            window: Window {
                from: UnixTime(NOW.0 - 7 * 86_400),
                to: NOW,
            },
            datum: None,
        }
    }

    pub(super) fn ctx() -> FetchContext {
        FetchContext::new(NOW, "underbite/test")
    }

    fn run(resource_fixture: &str, target: &Target) -> Result<Fragments, SourceError> {
        let usgs = Usgs::default();
        let ctx = ctx();
        let req = usgs.plan(target, &Requirements::default(), &ctx)?.remove(0);
        let body = fixture_bytes("usgs", resource_fixture);
        let mut out = Fragments::new();
        usgs.parse(&req, &Response::new(200, &body), &ctx, &mut out)?;
        Ok(out)
    }

    fn only_series(out: &Fragments, m: Measurement) -> &Series<f32> {
        match out
            .fragments()
            .iter()
            .find(|f| f.measurement == m)
            .map(|f| &f.value)
        {
            Some(V::Series(s)) => s,
            other => panic!("no series for {m:?}: {other:?}"),
        }
    }

    #[test]
    fn croton_latest_gives_discharge_and_gage_height() {
        use Measurement::{Discharge, GageHeight};
        let out = run(
            "usgs-latest-continuous-croton-01375000.json",
            &target("01375000", &[Discharge, GageHeight]),
        )
        .unwrap();
        let q = only_series(&out, Discharge);
        let s = q.latest().unwrap();
        assert!((s.value - 0.351).abs() < 0.001, "{}", s.value);
        assert_eq!(s.quality, Some(Quality::Provisional));
        assert_eq!(s.time, UnixTime(1_794_145_500));
        let prov = q.provenance();
        assert_eq!(prov.source, SourceId::USGS);
        assert_eq!(prov.kind, ValueKind::Observed);
        assert_eq!(prov.distance_m, Some(1200.0));
        let Some(V::Heights(h)) = out
            .fragments()
            .iter()
            .find(|f| f.measurement == GageHeight)
            .map(|f| &f.value)
        else {
            panic!("no heights");
        };
        assert_eq!(h.datum, VerticalDatum::Gauge);
        assert!((h.series.latest().unwrap().value - 0.335).abs() < 0.001);
        assert!(
            out.attributions()
                .iter()
                .any(|a| a.credit.contains("Provisional"))
        );
    }

    /// Fragments as plain values: shape, datum, count, newest sample and provenance.
    pub(super) fn summary(out: &Fragments) -> Value {
        let round = |v: f32| format!("{v:.4}");
        let items: Vec<Value> = out
            .fragments()
            .iter()
            .map(|f| {
                let (shape, datum, prov, n, last) = match &f.value {
                    V::Series(s) => (
                        "series",
                        None,
                        s.provenance(),
                        s.samples().len(),
                        s.latest().map(|x| (x.time.0, round(x.value), x.quality)),
                    ),
                    V::Heights(h) => (
                        "heights",
                        Some(h.datum),
                        h.series.provenance(),
                        h.series.samples().len(),
                        h.series
                            .latest()
                            .map(|x| (x.time.0, round(x.value), x.quality)),
                    ),
                    V::Turbidity(s) => (
                        "turbidity",
                        None,
                        s.provenance(),
                        s.samples().len(),
                        s.latest()
                            .map(|x| (x.time.0, format!("{:.4} {}", x.value.value, x.value.unit.label()), x.quality)),
                    ),
                    other => panic!("unexpected {other:?}"),
                };
                serde_json::json!({
                    "measurement": format!("{:?}", f.measurement),
                    "shape": shape,
                    "datum": datum.map(|d| format!("{d:?}")),
                    "samples": n,
                    "last": last.map(|(t, v, q)| serde_json::json!({"time": t, "value": v, "quality": q})),
                    "observed_at": prov.observed_at.0,
                    "kind": prov.kind,
                    "quality": prov.quality,
                    "station": prov.station.as_ref().map(|s| s.id.clone()),
                })
            })
            .collect();
        serde_json::json!({
            "fragments": items,
            "credits": out.attributions().iter().map(|a| a.credit.clone()).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn bronx_river_has_seven_parameters_with_turbidity_in_fnu() {
        use Measurement::*;
        let all = [
            Discharge,
            GageHeight,
            WaterTemperature,
            Turbidity,
            DissolvedOxygen,
            Conductivity,
            Ph,
        ];
        let out = run(
            "usgs-latest-continuous-bronx-river-01302020.json",
            &target("01302020", &all),
        )
        .unwrap();
        assert_eq!(out.fragments().len(), 7);
        insta::assert_yaml_snapshot!("latest_bronx_river", summary(&out));
    }

    #[test]
    fn poughkeepsie_drops_the_dead_series() {
        use Measurement::*;
        let wanted = [Tide, WaterTemperature, Turbidity, Conductivity, Discharge];
        let out = run(
            "usgs-latest-continuous-hudson-poughkeepsie-01372058.json",
            &target("01372058", &wanted),
        )
        .unwrap();
        assert_eq!(out.fragments().len(), 1, "only the live tidal elevation");
        insta::assert_yaml_snapshot!("latest_poughkeepsie", summary(&out));

        // Only dead series for the wanted parameter: stale, with the newest of them.
        let err = run(
            "usgs-latest-continuous-hudson-poughkeepsie-01372058.json",
            &target("01372058", &[WaterTemperature]),
        )
        .unwrap_err();
        assert_eq!(
            err,
            SourceError::Stale {
                latest: parse_rfc3339("2019-11-06T17:00:00+00:00").unwrap()
            }
        );
    }

    #[test]
    fn qualifiers_of_the_poughkeepsie_series_map_to_quality() {
        let body = fixture_bytes(
            "usgs",
            "usgs-latest-continuous-hudson-poughkeepsie-01372058.json",
        );
        let json: Value = serde_json::from_slice(&body).unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for f in json["features"].as_array().unwrap() {
            let p = &f["properties"];
            let qualifiers: Vec<String> = p["qualifier"]
                .as_array()
                .map(|a| a.iter().map(|q| q.as_str().unwrap().to_string()).collect())
                .unwrap_or_default();
            seen.insert(
                p["parameter_code"].as_str().unwrap().to_string(),
                quality(p["approval_status"].as_str(), &qualifiers),
            );
        }
        assert_eq!(seen["72254"], Quality::Flagged, "EQUIP");
        assert_eq!(seen["72137"], Quality::Flagged, "MAINT");
        assert_eq!(seen["00095"], Quality::Flagged, "DISCONTINUED");
        assert_eq!(seen["62620"], Quality::Provisional);
        assert_eq!(seen["62619"], Quality::Approved);
    }

    #[test]
    fn qualifier_codes_rank_flagged_then_ice_then_estimated() {
        let q = |codes: &[&str]| {
            let owned: Vec<String> = codes.iter().map(ToString::to_string).collect();
            quality(Some("Approved"), &owned)
        };
        assert_eq!(q(&[]), Quality::Approved);
        assert_eq!(q(&["ESTIMATED"]), Quality::Estimated);
        assert_eq!(q(&["ICE"]), Quality::IceAffected);
        assert_eq!(q(&["ESTIMATED", "ICE"]), Quality::IceAffected);
        assert_eq!(q(&["EQUIP", "ESTIMATED"]), Quality::Flagged);
        assert_eq!(q(&["MAINT"]), Quality::Flagged);
        assert_eq!(q(&["SOMETHING"]), Quality::Approved);
        assert_eq!(quality(None, &[]), Quality::Provisional);
    }

    #[test]
    fn one_multi_site_response_serves_each_station() {
        use Measurement::*;
        let file = "usgs-latest-continuous-multi-site.json";
        let croton = run(
            file,
            &target("USGS-01375000", &[Discharge, GageHeight, LakeLevel]),
        )
        .unwrap();
        assert_eq!(croton.fragments().len(), 2);
        let lake = run(file, &target("01323500", &[Discharge, LakeLevel])).unwrap();
        assert_eq!(lake.fragments().len(), 1);
        let tide = run(file, &target("01372058", &[Tide])).unwrap();
        insta::assert_yaml_snapshot!(
            "latest_multi_site",
            serde_json::json!([summary(&croton), summary(&lake), summary(&tide)])
        );
    }

    #[test]
    fn a_lake_with_two_datums_follows_the_wanted_one() {
        let file = "usgs-latest-continuous-great-sacandaga-01323500.json";
        let heights = |datum| {
            let mut t = target("01323500", &[Measurement::LakeLevel]);
            t.datum = datum;
            let out = run(file, &t).unwrap();
            match &out.fragments()[0].value {
                V::Heights(h) => (h.datum, h.series.latest().unwrap().value),
                other => panic!("{other:?}"),
            }
        };
        let (first_datum, first) = heights(None);
        assert_eq!(first_datum, VerticalDatum::Navd88);
        assert!((first - 754.61 * 0.3048).abs() < 0.01, "{first}");
        let (second_datum, second) = heights(Some(VerticalDatum::Ngvd29));
        assert_eq!(second_datum, VerticalDatum::Ngvd29);
        assert!((second - 755.49 * 0.3048).abs() < 0.01, "{second}");
    }

    #[test]
    fn the_plan_asks_for_every_datum_of_a_lake_and_skips_geometry() {
        let usgs = Usgs::default();
        let t = target(
            "01323500",
            &[Measurement::LakeLevel, Measurement::Discharge],
        );
        let reqs = usgs.plan(&t, &Requirements::default(), &ctx()).unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].resource, Resource::UsgsLatestContinuous);
        assert!(reqs[0].metered && reqs[0].required);
        assert_eq!(
            reqs[0].url,
            "https://api.waterdata.usgs.gov/ogcapi/v1/collections/latest-continuous/items?f=json&monitoring_location_id=USGS-01323500&parameter_code=00060,62614,62615,62616,00062&skipGeometry=true&properties=time_series_id,monitoring_location_id,parameter_code,statistic_id,time,value,unit_of_measure,approval_status,qualifier&limit=1000"
        );
        assert!(!reqs[0].cache.no_store);
        assert_eq!(reqs[0].headers, Vec::new());
    }

    #[test]
    fn a_key_goes_in_a_header_never_the_url() {
        let ctx = ctx().with_secrets(Secrets {
            usgs_api_key: Some("sekrit-key-123".to_string()),
        });
        let t = target("01375000", &[Measurement::Discharge]);
        let req = Usgs::default()
            .plan(&t, &Requirements::default(), &ctx)
            .unwrap()
            .remove(0);
        assert!(!req.url.contains("sekrit"));
        assert_eq!(
            req.headers,
            vec![("X-Api-Key", "sekrit-key-123".to_string())]
        );
        assert!(req.cache.no_store && !req.may_store_on_disk());
        assert!(!format!("{req:?}").contains("sekrit"));
    }

    #[test]
    fn errors_are_classified() {
        let usgs = Usgs::default();
        let t = target("01375000", &[Measurement::Discharge]);
        let req = usgs
            .plan(&t, &Requirements::default(), &ctx())
            .unwrap()
            .remove(0);
        let body = fixture_bytes("usgs", "usgs-error-limit-exceeded.json");
        let mut out = Fragments::new();
        let err = usgs
            .parse(&req, &Response::new(400, &body), &ctx(), &mut out)
            .unwrap_err();
        assert_eq!(
            err,
            SourceError::Status {
                status: 400,
                message: Some("Limit of 50000 exceeded".to_string())
            }
        );
        let mut limited = Response::new(429, b"");
        limited.retry_after = Some(30);
        let err = usgs.parse(&req, &limited, &ctx(), &mut out).unwrap_err();
        assert_eq!(
            err,
            SourceError::RateLimited {
                retry_after_s: Some(30)
            }
        );
        let err = usgs
            .parse(
                &req,
                &Response::new(200, br#"{"features":[]}"#),
                &ctx(),
                &mut out,
            )
            .unwrap_err();
        assert!(matches!(err, SourceError::NoData(_)));
        assert!(out.is_empty());
    }

    #[test]
    fn only_stations_and_known_measurements_are_planned() {
        let usgs = Usgs::default();
        let mut t = target("01375000", &[Measurement::Discharge]);
        t.place = Place::Point(ctx().round(crate::request::Coordinates {
            latitude: 40.7,
            longitude: -74.0,
        }));
        assert!(matches!(
            usgs.plan(&t, &Requirements::default(), &ctx()),
            Err(SourceError::Unsupported(_))
        ));
        let t = target("01375000", &[Measurement::Waves]);
        assert!(matches!(
            usgs.plan(&t, &Requirements::default(), &ctx()),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn the_source_describes_itself() {
        let usgs = Usgs::default();
        assert_eq!(usgs.id(), SourceId::USGS);
        assert_eq!(usgs.attribution().credit, "U.S. Geological Survey");
        assert!(usgs.measurements().contains(&Measurement::LakeLevel));
        assert_eq!(usgs.budget().unwrap().limit, 1000);
        assert_eq!(usgs.max_follow_ups(), 24);
        assert_eq!(format_time(NOW), "2026-11-08T14:30:00Z");
    }

    /// Parses `file` as the answer to the `continuous` request for `code` at `site`.
    fn history(site: &str, code: &str, datum: Option<VerticalDatum>, file: &str) -> Fragments {
        let p = spec(code).unwrap();
        let mut t = target(site, &[p.measurement]);
        t.datum = datum;
        let req = continuous_request("0123abcd", p, &t, &ctx()).unwrap();
        let body = fixture_bytes("usgs", file);
        let mut out = Fragments::new();
        Usgs::default()
            .parse(&req, &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        out
    }

    #[test]
    fn croton_week_has_673_ascending_samples() {
        let out = history(
            "01375000",
            "00060",
            None,
            "usgs-continuous-croton-01375000-00060-2026-11-01_2026-11-08.json",
        );
        let s = only_series(&out, Measurement::Discharge);
        assert_eq!(s.samples().len(), 673);
        assert!(s.samples().windows(2).all(|w| w[0].time < w[1].time));
        assert!((s.samples()[0].value - 163.0 * 0.028_316_85).abs() < 0.001);
        insta::assert_yaml_snapshot!("continuous_croton_discharge", summary(&out));
    }

    #[test]
    fn lake_history_is_ngvd29_metres_and_tide_navd88() {
        let lake = history(
            "01323500",
            "62614",
            None,
            "usgs-continuous-great-sacandaga-01323500-62614-2026-11-01_2026-11-08.json",
        );
        let V::Heights(h) = &lake.fragments()[0].value else {
            panic!("heights");
        };
        assert_eq!(h.datum, VerticalDatum::Ngvd29);
        assert!((h.series.samples()[0].value - 756.04 * 0.3048).abs() < 0.01);
        let tide = history(
            "01372058",
            "62620",
            None,
            "usgs-continuous-hudson-poughkeepsie-01372058-62620-2026-11-01_2026-11-08.json",
        );
        assert_eq!(tide.fragments()[0].measurement, Measurement::Tide);
        insta::assert_yaml_snapshot!(
            "continuous_lake_and_tide",
            serde_json::json!([summary(&lake), summary(&tide)])
        );
    }

    #[test]
    fn turbidity_history_keeps_its_unit() {
        let out = history(
            "01302020",
            "63680",
            None,
            "usgs-continuous-bronx-river-01302020-63680-2026-11-01_2026-11-08.json",
        );
        let V::Turbidity(s) = &out.fragments()[0].value else {
            panic!("turbidity");
        };
        assert_eq!(s.samples()[0].value.unit, TurbidityUnit::Fnu);
        assert!((s.samples()[0].value.value - 1.3).abs() < 1e-6);
    }

    #[test]
    fn the_latest_response_calls_for_each_chosen_series_history() {
        let usgs = Usgs::default();
        let t = target(
            "01375000",
            &[Measurement::Discharge, Measurement::GageHeight],
        );
        let req = usgs
            .plan(&t, &Requirements::default(), &ctx())
            .unwrap()
            .remove(0);
        let body = fixture_bytes("usgs", "usgs-latest-continuous-croton-01375000.json");
        let follow = usgs
            .follow_up(&req, &Response::new(200, &body), &ctx())
            .unwrap();
        assert_eq!(follow.len(), 2);
        assert!(
            follow
                .iter()
                .all(|r| r.resource == Resource::UsgsContinuous && r.metered)
        );
        // Discharge first (measurement order), by series id, window in the encoding USGS
        // itself uses in its `next` links.
        assert_eq!(
            follow[0].url,
            "https://api.waterdata.usgs.gov/ogcapi/v1/collections/continuous/items?f=json&time_series_id=e32aab5ccc774c11b172102b97abc0d8&datetime=2026-11-01T14%3A30%3A00Z%2F2026-11-08T14%3A30%3A00Z&skipGeometry=true&sortby=time&limit=2000&properties=time_series_id,parameter_code,time,value,unit_of_measure,approval_status,qualifier"
        );
        let paged = fixture_bytes(
            "usgs",
            "usgs-continuous-croton-01375000-00060-paging-limit3.json",
        );
        let next = String::from_utf8(paged).unwrap();
        assert!(next.contains("datetime=2026-11-01T00%3A00%3A00Z%2F2026-11-08T00%3A00%3A00Z"));
        assert_eq!(follow[0].target.measurements.len(), 1);

        // A window that ends the instant it starts needs no history.
        let mut now_only = t.clone();
        now_only.window.from = NOW;
        let req = usgs
            .plan(&now_only, &Requirements::default(), &ctx())
            .unwrap()
            .remove(0);
        assert_eq!(
            usgs.follow_up(&req, &Response::new(200, &body), &ctx())
                .unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn a_window_over_three_years_is_cut_to_three_years() {
        let mut t = target("01375000", &[Measurement::Discharge]);
        t.window.from = UnixTime(NOW.0 - 5 * 365 * 86_400);
        let req = continuous_request("abc", spec("00060").unwrap(), &t, &ctx()).unwrap();
        assert!(req.url.contains("datetime=2023-11-"), "{}", req.url);
    }

    #[test]
    fn a_page_with_a_next_link_follows_up_and_merges_the_pages() {
        let usgs = Usgs::default();
        let ctx = ctx().with_secrets(Secrets {
            usgs_api_key: Some("sekrit-key-123".to_string()),
        });
        let t = target("01375000", &[Measurement::Discharge]);
        let first = continuous_request("e32a", spec("00060").unwrap(), &t, &ctx).unwrap();
        let page = fixture_bytes(
            "usgs",
            "usgs-continuous-croton-01375000-00060-paging-limit3.json",
        );
        let follow = usgs
            .follow_up(&first, &Response::new(200, &page), &ctx)
            .unwrap();
        assert_eq!(follow.len(), 1);
        let next = &follow[0];
        assert!(next.url.starts_with(
            "https://api.waterdata.usgs.gov/ogcapi/v1/collections/continuous/items?cursor=ZTMyYWFi"
        ));
        assert_eq!(next.headers, first.headers);
        assert!(next.cache.no_store);
        assert!(
            next.carry()
                .unwrap()
                .contains("\"parameter_code\":\"00060\"")
        );

        // The first page alone, then the follow-up's page on top of it.
        let mut one = Fragments::new();
        usgs.parse(&first, &Response::new(200, &page), &ctx, &mut one)
            .unwrap();
        assert_eq!(only_series(&one, Measurement::Discharge).samples().len(), 3);
        let rest = fixture_bytes(
            "usgs",
            "usgs-continuous-croton-01375000-00060-2026-11-01_2026-11-08.json",
        );
        let mut all = Fragments::new();
        usgs.parse(next, &Response::new(200, &rest), &ctx, &mut all)
            .unwrap();
        assert_eq!(
            only_series(&all, Measurement::Discharge).samples().len(),
            673
        );

        // The last page has no next link.
        assert_eq!(
            usgs.follow_up(next, &Response::new(200, &rest), &ctx)
                .unwrap(),
            Vec::new()
        );
    }

    /// Parses `file` as the answer to the daily request of `target`'s only measurement.
    fn daily(site: &str, m: Measurement, datum: Option<VerticalDatum>, file: &str) -> Fragments {
        let mut t = target(site, &[m]);
        t.datum = datum;
        let req = Usgs::default()
            .daily_requests(&t, &ctx())
            .unwrap()
            .remove(0);
        let body = fixture_bytes("usgs", file);
        let mut out = Fragments::new();
        Usgs::default()
            .parse(&req, &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        out
    }

    #[test]
    fn the_daily_request_asks_for_means_up_to_fifty_thousand() {
        let t = target("01375000", &[Measurement::Discharge]);
        let reqs = Usgs::default().daily_requests(&t, &ctx()).unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].resource, Resource::UsgsDaily);
        assert!(reqs[0].metered);
        assert_eq!(
            reqs[0].url,
            "https://api.waterdata.usgs.gov/ogcapi/v1/collections/daily/items?f=json&monitoring_location_id=USGS-01375000&parameter_code=00060&statistic_id=00003&datetime=2026-11-01%2F2026-11-08&skipGeometry=true&sortby=time&limit=50000&properties=time_series_id,monitoring_location_id,parameter_code,statistic_id,time,value,unit_of_measure,approval_status,qualifier"
        );
    }

    #[test]
    fn daily_means_are_stamped_at_noon_utc_of_their_date() {
        let out = daily(
            "01375000",
            Measurement::Discharge,
            None,
            "usgs-daily-croton-01375000-00060-mean-2026-07-06_2026-07-08.json",
        );
        let s = only_series(&out, Measurement::Discharge);
        assert_eq!(s.samples().len(), 3);
        let utc = underbite_core::time::zone("UTC").unwrap();
        let dates: Vec<String> = s
            .samples()
            .iter()
            .map(|x| {
                underbite_core::time::day_key(
                    underbite_core::time::local_date(x.time, &utc).unwrap(),
                )
            })
            .collect();
        assert_eq!(dates, ["07-06", "07-07", "07-08"]);
        assert_eq!(s.samples()[0].time.0 % 86_400, 12 * 3_600);
        assert!((s.samples()[0].value - 1110.0 * 0.028_316_85).abs() < 0.01);
        assert_eq!(s.provenance().kind, ValueKind::Computed);
        insta::assert_yaml_snapshot!("daily_croton_july", summary(&out));
    }

    #[test]
    fn a_year_of_daily_means_and_a_lake_in_ngvd29() {
        let year = daily(
            "01375000",
            Measurement::Discharge,
            None,
            "usgs-daily-croton-01375000-00060-mean-2025-11-08_2026-11-07.json",
        );
        assert_eq!(
            only_series(&year, Measurement::Discharge).samples().len(),
            365
        );
        let lake = daily(
            "01323500",
            Measurement::LakeLevel,
            Some(VerticalDatum::Ngvd29),
            "usgs-daily-great-sacandaga-01323500-62614-mean-2025-11-08_2026-11-07.json",
        );
        let V::Heights(h) = &lake.fragments()[0].value else {
            panic!("heights");
        };
        assert_eq!(h.datum, VerticalDatum::Ngvd29);
        assert_eq!(h.series.samples().len(), 365);
    }

    #[test]
    fn every_qualifier_combination_of_a_winter_day_maps_to_quality() {
        let body = fixture_bytes(
            "usgs",
            "usgs-daily-qualifiers-ny-bbox-00060-2026-01-20.json",
        );
        let json: Value = serde_json::from_slice(&body).unwrap();
        let p = spec("00060").unwrap();
        let mut counts = std::collections::BTreeMap::new();
        let mut missing = 0;
        for f in json["features"].as_array().unwrap() {
            let props: Props = serde_json::from_value(f["properties"].clone()).unwrap();
            match point(&props, p, UnixTime(0)).unwrap() {
                Some(pt) => *counts.entry(pt.q).or_insert(0) += 1,
                None => missing += 1,
            }
        }
        assert_eq!(missing, 9, "null values (ICE with no flow) are skipped");
        assert_eq!(
            counts,
            [
                (Quality::Provisional, 18),
                (Quality::Approved, 81),
                (Quality::Estimated, 182),
                (Quality::IceAffected, 2),
                (Quality::Flagged, 1),
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn daily_pages_follow_their_next_link() {
        let t = target("01375000", &[Measurement::Discharge]);
        let usgs = Usgs::default();
        let req = usgs.daily_requests(&t, &ctx()).unwrap().remove(0);
        let page = fixture_bytes(
            "usgs",
            "usgs-daily-croton-01375000-00060-paging-limit3.json",
        );
        let follow = usgs
            .follow_up(&req, &Response::new(200, &page), &ctx())
            .unwrap();
        assert_eq!(follow.len(), 1);
        assert_eq!(follow[0].resource, Resource::UsgsDaily);
        assert!(follow[0].url.contains("/collections/daily/items?cursor="));
        let mut out = Fragments::new();
        usgs.parse(&req, &Response::new(200, &page), &ctx(), &mut out)
            .unwrap();
        assert_eq!(only_series(&out, Measurement::Discharge).samples().len(), 3);
    }

    #[test]
    fn a_next_link_off_the_api_is_refused() {
        let body = br#"{"features":[],"links":[{"rel":"next","href":"https://example.org/x"}]}"#;
        let t = target("01375000", &[Measurement::Discharge]);
        let req = continuous_request("e32a", spec("00060").unwrap(), &t, &ctx()).unwrap();
        let err = Usgs::default()
            .follow_up(&req, &Response::new(200, body), &ctx())
            .unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)));
    }
}
