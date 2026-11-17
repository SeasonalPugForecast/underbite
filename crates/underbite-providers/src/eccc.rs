//! Environment and Climate Change Canada hydrometric data: real-time river level and discharge, and the HYDAT daily means.
//!
//! The service is the MSC `GeoMet` OGC API (`api.weather.gc.ca`, pygeoapi, `GeoJSON`). Three
//! collections are used:
//!
//! - `hydrometric-realtime`: the last 30 days at 5-minute steps. [`Eccc::plan`] asks for the newest
//!   rows of a station (`sortby=-DATETIME`), [`Eccc::parse`] turns them into a discharge series
//!   (m³/s) and a gage height series ([`VerticalDatum::Gauge`], metres). `LEVEL` is metres above
//!   the station's own datum, mostly an arbitrary local one, so it is a gauge reading to compare
//!   with the same station's history only. Every real-time value is provisional.
//! - `hydrometric-daily-mean`: the HYDAT archive, which ends at the last 31 December. "Compared
//!   with normal" is computed from this station's own history ([`Eccc::percentile_request`]) with
//!   [`from_daily`], while the current year comes from the real-time collection. USGS daily means
//!   are a different product and are never mixed in.
//! - `hydrometric-stations`: discovery by box ([`stations_request`], [`parse_stations`]), active
//!   real-time stations only.
//!
//! Times are read from `DATETIME` (UTC). `DATETIME_LST` is local standard time with its offset in
//! the string; it is used only to learn the station's standard-time offset, which fixes the
//! standard-time day of a reading (the HYDAT daily means are per standard-time day).
//!
//! The requests and parsers are plain functions and methods of [`Eccc`] over [`EcccRequest`], which
//! mirrors [`crate::request::Request`] until the shared request type has resources for these
//! collections.

use std::collections::BTreeMap;

use serde::Deserialize;
use underbite_core::datum::{Heights, VerticalDatum};
use underbite_core::derive::percentile::{DEFAULT_MIN_YEARS, from_daily};
use underbite_core::map::Bbox;
use underbite_core::model::{Coverage, Measurement, StationCandidate};
use underbite_core::provenance::{
    Attribution, Provenance, Quality, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::{Sample, Series};
use underbite_core::time::{LocalDate, UnixTime, UtcOffset, parse_date, parse_rfc3339};

use crate::request::{Place, QueryBuilder, Response, Target};
use crate::source::{FetchContext, FragmentValue, Fragments, SourceError};

/// The API root.
pub const BASE: &str = "https://api.weather.gc.ca/collections";

/// The ECCC Data Servers End-use Licence.
pub const LICENCE_URL: &str = "https://eccc-msc.github.io/open-data/licence/readme_en/";

/// The Water Survey of Canada's notes on provisional real-time data.
const PROVISIONAL_URL: &str = "https://wateroffice.ec.gc.ca/contactus/faq_e.html";

/// The measurements the source supplies.
pub const MEASUREMENTS: &[Measurement] = &[Measurement::Discharge, Measurement::GageHeight];

/// The most real-time rows asked for in one request (the service returned 1,096 rows for
/// `limit=1100`; its own maximum is not documented).
pub const MAX_REALTIME_ROWS: i64 = 1_000;

/// Years of daily means asked for by default when computing normals.
pub const DEFAULT_HISTORY_YEARS: i32 = 20;

/// Stations asked for per page of a discovery box.
pub const STATIONS_PAGE: u32 = 100;

/// Longest message kept from an error body, in characters.
const MAX_MESSAGE: usize = 200;

/// The seconds between two real-time rows.
const STEP_S: i64 = 300;

/// The credit every display of ECCC values shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::ECCC_HYDROMETRIC,
        credit: "Data Source: Environment and Climate Change Canada".to_string(),
        url: Some(LICENCE_URL.to_string()),
    }
}

/// The notice that goes with real-time values.
fn provisional_notice() -> Attribution {
    Attribution {
        source: SourceId::ECCC_HYDROMETRIC,
        credit: "Real-time Water Survey of Canada data are provisional and subject to revision"
            .to_string(),
        url: Some(PROVISIONAL_URL.to_string()),
    }
}

/// Which collection a request is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EcccResource {
    /// `hydrometric-stations`.
    Stations,
    /// `hydrometric-realtime`.
    Realtime,
    /// `hydrometric-daily-mean`, for the day-of-year normals of one measurement.
    DailyMean,
}

/// The reading a normal is computed for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// The measurement.
    pub measurement: Measurement,
    /// The value, canonical units (metres on the station datum, m³/s).
    pub value: f32,
    /// When it was observed.
    pub observed_at: UnixTime,
    /// The station's standard-time day of the observation.
    pub day: LocalDate,
}

/// A GET the host sends, with what the parser needs to read the answer.
#[derive(Clone, Debug, PartialEq)]
pub struct EcccRequest {
    /// The URL, query included.
    pub url: String,
    /// The collection, which selects the parser.
    pub resource: EcccResource,
    /// What the request is for.
    pub target: Target,
    /// Whether the link fails without this response.
    pub required: bool,
    /// For [`EcccResource::DailyMean`]: the reading the normals are for.
    pub reading: Option<Reading>,
}

/// The ECCC hydrometric source: settings only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eccc {
    /// How old the newest real-time value may be before the station counts as dead, seconds.
    pub max_age_s: i64,
    /// Years of daily means a normal is computed from.
    pub history_years: i32,
}

impl Default for Eccc {
    fn default() -> Self {
        Self {
            max_age_s: 12 * 3_600,
            history_years: DEFAULT_HISTORY_YEARS,
        }
    }
}

/// `05BH004`: two digits, two letters, three digits.
fn valid_station_number(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 7
        && b[..2].iter().all(u8::is_ascii_digit)
        && b[2..4].iter().all(u8::is_ascii_uppercase)
        && b[4..].iter().all(u8::is_ascii_digit)
}

fn station_number(target: &Target) -> Result<&str, SourceError> {
    match &target.place {
        Place::Station { station, .. } if valid_station_number(&station.id) => Ok(&station.id),
        Place::Station { station, .. } => Err(SourceError::Unsupported(format!(
            "{:?} is not a Water Survey station number",
            station.id
        ))),
        _ => Err(SourceError::Unsupported(
            "ECCC values need a station".to_string(),
        )),
    }
}

fn items(collection: &str) -> String {
    format!("{BASE}/{collection}/items")
}

/// The real-time request: the newest rows of a station, newest first.
///
/// `rows` is clamped to 12 to [`MAX_REALTIME_ROWS`].
///
/// # Errors
/// [`SourceError::Unsupported`] when the target is not a station with a Water Survey number.
pub fn realtime_request(target: &Target, rows: i64) -> Result<EcccRequest, SourceError> {
    let id = station_number(target)?;
    let url = QueryBuilder::new(&items("hydrometric-realtime"))
        .param("f", "json")
        .param("STATION_NUMBER", id)
        .param("sortby", "-DATETIME")
        .param("limit", rows.clamp(12, MAX_REALTIME_ROWS).to_string())
        .param("skipGeometry", "true")
        .param(
            "properties",
            "DATETIME,DATETIME_LST,LEVEL,DISCHARGE,LEVEL_SYMBOL_EN,DISCHARGE_SYMBOL_EN",
        )
        .finish();
    Ok(EcccRequest {
        url,
        resource: EcccResource::Realtime,
        target: target.clone(),
        required: true,
        reading: None,
    })
}

/// The stations of a box, active and reporting in real time, `STATION_NUMBER` order, a page of
/// [`STATIONS_PAGE`] from `offset`. The box is rounded to two decimals.
///
/// # Errors
/// [`SourceError::Unsupported`] for a box that is not finite or is inverted.
pub fn stations_request(area: Bbox, offset: u32) -> Result<String, SourceError> {
    let finite = [area.west, area.south, area.east, area.north]
        .iter()
        .all(|v| v.is_finite());
    if !finite || area.west > area.east || area.south > area.north {
        return Err(SourceError::Unsupported(
            "a box needs finite, ordered corners".to_string(),
        ));
    }
    let bbox = format!(
        "{:.2},{:.2},{:.2},{:.2}",
        area.west, area.south, area.east, area.north
    );
    Ok(QueryBuilder::new(&items("hydrometric-stations"))
        .param("f", "json")
        .param("bbox", bbox)
        .param("STATUS_EN", "Active")
        .param("REAL_TIME", "1")
        .param("sortby", "STATION_NUMBER")
        .param("limit", STATIONS_PAGE.to_string())
        .param("offset", offset.to_string())
        .finish())
}

impl Eccc {
    /// A source with the default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Real-time values older than `seconds` mean a dead station.
    #[must_use]
    pub fn with_max_age(mut self, seconds: i64) -> Self {
        self.max_age_s = seconds;
        self
    }

    /// Years of daily means a normal is computed from.
    #[must_use]
    pub fn with_history_years(mut self, years: i32) -> Self {
        self.history_years = years;
        self
    }

    /// The credit every display of its values shows.
    pub fn attribution(&self) -> Attribution {
        attribution()
    }

    /// The measurements it supplies.
    pub fn measurements(&self) -> &'static [Measurement] {
        MEASUREMENTS
    }

    /// The requests for `target`: the newest real-time rows, enough for the target's window.
    ///
    /// # Errors
    /// [`SourceError::Unsupported`] for a target that is not a station or wants only
    /// measurements the source does not supply.
    pub fn plan(
        &self,
        target: &Target,
        ctx: &FetchContext,
    ) -> Result<Vec<EcccRequest>, SourceError> {
        if !target.measurements.iter().any(|m| MEASUREMENTS.contains(m)) {
            return Err(SourceError::Unsupported(
                "ECCC supplies discharge and gage height only".to_string(),
            ));
        }
        let since = ctx.now.0.saturating_sub(target.window.from.0).max(0);
        Ok(vec![realtime_request(target, since / STEP_S + 1)?])
    }

    /// The request for the day-of-year normals of `reading`, from this station's own daily means
    /// of the last [`Eccc::history_years`] years up to the day before the reading's day. It is
    /// optional: a station with too short a record gives no normals, not a failed link.
    ///
    /// # Errors
    /// [`SourceError::Unsupported`] for a target that is not a station or a measurement the
    /// source does not supply.
    pub fn percentile_request(
        &self,
        target: &Target,
        reading: Reading,
    ) -> Result<EcccRequest, SourceError> {
        let id = station_number(target)?;
        let field = match reading.measurement {
            Measurement::Discharge => "DISCHARGE",
            Measurement::GageHeight => "LEVEL",
            other => {
                return Err(SourceError::Unsupported(format!(
                    "ECCC has no daily means for {other:?}"
                )));
            }
        };
        let years = self.history_years.max(1);
        let day = reading.day;
        let yesterday =
            UnixTime(underbite_core::time::day_number(day).saturating_sub(1) * 86_400 + 43_200)
                .to_local(UtcOffset(0))
                .date;
        let date = |d: LocalDate| format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
        let start = LocalDate {
            year: day.year - years,
            month: 1,
            day: 1,
        };
        let url = QueryBuilder::new(&items("hydrometric-daily-mean"))
            .param("f", "json")
            .param("STATION_NUMBER", id)
            .param("datetime", format!("{}/{}", date(start), date(yesterday)))
            .param("sortby", "DATE")
            .param("limit", (i64::from(years) * 366 + 366).to_string())
            .param("skipGeometry", "true")
            .param("properties", format!("DATE,{field}"))
            .finish();
        Ok(EcccRequest {
            url,
            resource: EcccResource::DailyMean,
            target: target.clone(),
            required: false,
            reading: Some(reading),
        })
    }

    /// Reads the newest real-time reading of `measurement`, for [`Eccc::percentile_request`].
    ///
    /// # Errors
    /// As [`Eccc::parse`].
    pub fn latest_reading(
        &self,
        response: &Response<'_>,
        measurement: Measurement,
    ) -> Result<Reading, SourceError> {
        let body = check(response)?;
        let rows = realtime_rows(body)?;
        rows.iter()
            .find_map(|row| {
                let value = match measurement {
                    Measurement::Discharge => row.discharge,
                    Measurement::GageHeight => row.level,
                    _ => None,
                }?;
                Some(Reading {
                    measurement,
                    value,
                    observed_at: row.time,
                    day: row.time.to_local(row.offset).date,
                })
            })
            .ok_or_else(|| SourceError::NoData(Some("no real-time value".to_string())))
    }

    /// Parses a response to a request of this module into `out`: real-time rows into discharge and
    /// gage height series, daily means into a percentile. A station list is read with
    /// [`parse_stations`].
    ///
    /// # Errors
    /// [`SourceError`] for an error status, a body of the wrong shape, no value, or a newest value
    /// older than [`Eccc::max_age_s`].
    pub fn parse(
        &self,
        request: &EcccRequest,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let body = check(response)?;
        match request.resource {
            EcccResource::Realtime => self.parse_realtime(request, body, ctx, out),
            EcccResource::DailyMean => parse_normals(request, body, out),
            EcccResource::Stations => Err(SourceError::Unsupported(
                "a station list is parsed with parse_stations".to_string(),
            )),
        }
    }

    fn parse_realtime(
        &self,
        request: &EcccRequest,
        body: &[u8],
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        let rows = realtime_rows(body)?;
        let wanted = |m: Measurement| request.target.measurements.contains(&m);
        let mut newest: Option<UnixTime> = None;
        let mut added = false;
        for measurement in MEASUREMENTS.iter().copied().filter(|m| wanted(*m)) {
            let samples: Vec<Sample<f32>> = rows
                .iter()
                .filter_map(|row| {
                    let (value, symbol) = match measurement {
                        Measurement::Discharge => {
                            (row.discharge.filter(|v| *v >= 0.0), &row.discharge_symbol)
                        }
                        _ => (row.level, &row.level_symbol),
                    };
                    value.map(|value| Sample {
                        time: row.time,
                        value,
                        quality: Some(quality(symbol.as_deref())),
                    })
                })
                .collect();
            let Some(last) = samples.iter().max_by_key(|s| s.time) else {
                continue;
            };
            newest = newest.max(Some(last.time));
            if ctx.now.0.saturating_sub(last.time.0) > self.max_age_s {
                continue;
            }
            let prov = provenance(
                &request.target,
                last.time,
                last.quality.unwrap_or(Quality::Provisional),
            )?;
            let series = Series::new(prov, samples);
            let value = match measurement {
                Measurement::GageHeight => FragmentValue::Heights(Heights {
                    datum: VerticalDatum::Gauge,
                    series,
                }),
                _ => FragmentValue::Series(series),
            };
            out.push(measurement, value)?;
            added = true;
        }
        if added {
            out.credit(provisional_notice());
            return Ok(());
        }
        Err(match newest {
            Some(latest) => SourceError::Stale { latest },
            None => SourceError::NoData(Some("no real-time value".to_string())),
        })
    }
}

/// The quality a Water Survey symbol (English text) stands for; real-time data without one is
/// provisional.
fn quality(symbol: Option<&str>) -> Quality {
    let Some(text) = symbol.map(str::to_ascii_lowercase) else {
        return Quality::Provisional;
    };
    if text.contains("ice") {
        Quality::IceAffected
    } else if text.contains("estimate") || text.contains("partial") {
        Quality::Estimated
    } else if text.contains("dry") {
        Quality::Flagged
    } else {
        Quality::Provisional
    }
}

fn provenance(
    target: &Target,
    observed_at: UnixTime,
    quality: Quality,
) -> Result<Provenance, SourceError> {
    let Place::Station {
        station,
        distance_m,
        relation,
    } = &target.place
    else {
        return Err(SourceError::Unsupported(
            "ECCC values need a station".to_string(),
        ));
    };
    Ok(Provenance {
        source: SourceId::ECCC_HYDROMETRIC,
        station: Some(station.clone()),
        distance_m: *distance_m,
        relation: *relation,
        observed_at,
        kind: ValueKind::Observed,
        quality,
    })
}

/// Rounds the float32 widening of the service (`38.400001525878906`) to three decimals.
fn round3(v: f64) -> Option<f32> {
    v.is_finite()
        .then(|| ((v * 1_000.0).round() / 1_000.0) as f32)
}

fn tidy(text: &str) -> Option<String> {
    let text = text.trim();
    let cut = text
        .char_indices()
        .nth(MAX_MESSAGE)
        .map_or(text.len(), |(i, _)| i);
    (cut > 0).then(|| text[..cut].to_string())
}

/// Checks a response and returns its trimmed body: 2xx goes on, 404 is not found, 429 is rate
/// limited, anything else is a status error with the `description` of the OGC error body.
fn check<'a>(response: &Response<'a>) -> Result<&'a [u8], SourceError> {
    let body = response.body.trim_ascii();
    let message = || {
        #[derive(Deserialize)]
        struct Problem {
            description: Option<String>,
        }
        serde_json::from_slice::<Problem>(body)
            .ok()
            .and_then(|p| p.description)
            .and_then(|d| tidy(&d))
    };
    match response.status {
        200..=299 => {}
        404 | 410 => return Err(SourceError::NotFound(message())),
        429 => {
            return Err(SourceError::RateLimited {
                retry_after_s: response.retry_after,
            });
        }
        status => {
            return Err(SourceError::Status {
                status,
                message: message(),
            });
        }
    }
    match body.first() {
        None => Err(SourceError::NoData(Some("empty response".to_string()))),
        Some(b'{') => Ok(body),
        Some(b'<') => Err(SourceError::Parse("HTML instead of JSON".to_string())),
        Some(_) => Err(SourceError::Parse("not a JSON object".to_string())),
    }
}

#[derive(Deserialize)]
struct Collection<P> {
    features: Vec<Feature<P>>,
    #[serde(default, rename = "numberMatched")]
    matched: Option<u64>,
    #[serde(default)]
    links: Vec<Link>,
}

#[derive(Deserialize)]
struct Feature<P> {
    properties: P,
    #[serde(default)]
    geometry: Option<Geometry>,
}

#[derive(Deserialize)]
struct Geometry {
    coordinates: Option<Vec<f64>>,
}

#[derive(Deserialize)]
struct Link {
    rel: Option<String>,
    href: Option<String>,
}

fn collection<P: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<Collection<P>, SourceError> {
    serde_json::from_slice(body).map_err(|e| SourceError::Parse(format!("ECCC response: {e}")))
}

/// The `offset` of the `next` link of a page, when there is a next page on the ECCC API.
///
/// # Errors
/// [`SourceError::Parse`] for a body that is not a feature collection or a next link off the API.
pub fn next_offset(body: &[u8]) -> Result<Option<u32>, SourceError> {
    let page: Collection<serde_json::Value> = collection(body)?;
    let Some(href) = page
        .links
        .iter()
        .find(|l| l.rel.as_deref() == Some("next"))
        .and_then(|l| l.href.as_deref())
    else {
        return Ok(None);
    };
    if !href.starts_with(BASE) {
        return Err(SourceError::Parse(format!(
            "next link off the ECCC API: {href}"
        )));
    }
    let offset = href
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split('&')
        .find_map(|pair| pair.strip_prefix("offset="))
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| SourceError::Parse("next link without an offset".to_string()))?;
    Ok(Some(offset))
}

#[derive(Deserialize)]
struct RealtimeProps {
    #[serde(rename = "DATETIME")]
    datetime: String,
    #[serde(rename = "DATETIME_LST")]
    datetime_lst: Option<String>,
    #[serde(rename = "LEVEL")]
    level: Option<f64>,
    #[serde(rename = "DISCHARGE")]
    discharge: Option<f64>,
    #[serde(rename = "LEVEL_SYMBOL_EN")]
    level_symbol: Option<String>,
    #[serde(rename = "DISCHARGE_SYMBOL_EN")]
    discharge_symbol: Option<String>,
}

struct Row {
    time: UnixTime,
    offset: UtcOffset,
    level: Option<f32>,
    discharge: Option<f32>,
    level_symbol: Option<String>,
    discharge_symbol: Option<String>,
}

/// The standard-time offset of a row: its `DATETIME_LST` read as if it were UTC, minus
/// `DATETIME`. Never parses the local string as UTC time.
fn standard_offset(utc: UnixTime, lst: Option<&str>) -> Result<UtcOffset, SourceError> {
    let Some(lst) = lst else {
        return Ok(UtcOffset(0));
    };
    let naive = lst
        .get(..19)
        .and_then(|head| parse_rfc3339(&format!("{head}Z")))
        .ok_or_else(|| SourceError::Parse(format!("DATETIME_LST {lst:?}")))?;
    let seconds = naive.0 - utc.0;
    if seconds.abs() > 14 * 3_600 || seconds % 900 != 0 {
        return Err(SourceError::Parse(format!(
            "DATETIME_LST {lst:?} is not a standard-time offset from DATETIME"
        )));
    }
    i32::try_from(seconds)
        .map(UtcOffset)
        .map_err(|_| SourceError::Parse(format!("DATETIME_LST {lst:?}")))
}

fn realtime_rows(body: &[u8]) -> Result<Vec<Row>, SourceError> {
    let page: Collection<RealtimeProps> = collection(body)?;
    let mut rows = Vec::with_capacity(page.features.len());
    for feature in page.features {
        let p = feature.properties;
        let time = parse_rfc3339(&p.datetime)
            .ok_or_else(|| SourceError::Parse(format!("DATETIME {:?}", p.datetime)))?;
        rows.push(Row {
            time,
            offset: standard_offset(time, p.datetime_lst.as_deref())?,
            level: p.level.and_then(round3),
            discharge: p.discharge.and_then(round3),
            level_symbol: p.level_symbol,
            discharge_symbol: p.discharge_symbol,
        });
    }
    // Newest first, whatever the service's order.
    rows.sort_by_key(|r| std::cmp::Reverse(r.time));
    Ok(rows)
}

/// One station-local day of the HYDAT archive.
#[derive(Clone, Debug, PartialEq)]
pub struct DailyMean {
    /// The station's standard-time day.
    pub date: LocalDate,
    /// Daily mean level, metres on the station datum.
    pub level: Option<f32>,
    /// Daily mean discharge, m³/s.
    pub discharge: Option<f32>,
    /// The Water Survey symbol of the discharge (`Ice Conditions`), as the service writes it.
    pub discharge_symbol: Option<String>,
    /// The Water Survey symbol of the level.
    pub level_symbol: Option<String>,
}

#[derive(Deserialize)]
struct DailyProps {
    #[serde(rename = "DATE")]
    date: String,
    #[serde(rename = "LEVEL")]
    level: Option<f64>,
    #[serde(rename = "DISCHARGE")]
    discharge: Option<f64>,
    #[serde(rename = "DISCHARGE_SYMBOL_EN")]
    discharge_symbol: Option<String>,
    #[serde(rename = "LEVEL_SYMBOL_EN")]
    level_symbol: Option<String>,
}

/// The days of a `hydrometric-daily-mean` response, ascending, values rounded to three decimals.
/// Missing days are absent.
///
/// # Errors
/// [`SourceError`] as for [`Eccc::parse`], and [`SourceError::Parse`] for a malformed date.
pub fn parse_daily_means(response: &Response<'_>) -> Result<Vec<DailyMean>, SourceError> {
    let body = check(response)?;
    daily_means(body)
}

fn daily_means(body: &[u8]) -> Result<Vec<DailyMean>, SourceError> {
    let page: Collection<DailyProps> = collection(body)?;
    let mut days = Vec::with_capacity(page.features.len());
    for feature in page.features {
        let p = feature.properties;
        let date = parse_date(&p.date)
            .map_err(|e| SourceError::Parse(format!("daily mean date {:?}: {e}", p.date)))?;
        days.push(DailyMean {
            date,
            level: p.level.and_then(round3),
            discharge: p.discharge.and_then(round3),
            discharge_symbol: p.discharge_symbol,
            level_symbol: p.level_symbol,
        });
    }
    days.sort_by_key(|d| underbite_core::time::day_number(d.date));
    Ok(days)
}

fn parse_normals(
    request: &EcccRequest,
    body: &[u8],
    out: &mut Fragments,
) -> Result<(), SourceError> {
    let reading = request.reading.ok_or_else(|| {
        SourceError::Unsupported("daily means need the reading they are compared with".to_string())
    })?;
    let key = (reading.day.month, reading.day.day);
    let values: Vec<(LocalDate, f32)> = daily_means(body)?
        .into_iter()
        .filter(|d| {
            underbite_core::time::day_number(d.date) < underbite_core::time::day_number(reading.day)
        })
        .filter_map(|d| {
            let v = match reading.measurement {
                Measurement::Discharge => d.discharge,
                _ => d.level,
            };
            v.map(|v| (d.date, v))
        })
        .collect();
    let Some(mut percentile) = from_daily(&values, key, reading.value, DEFAULT_MIN_YEARS) else {
        let years = values
            .iter()
            .filter(|(d, _)| (d.month, d.day) == key)
            .map(|(d, _)| d.year)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        return Err(SourceError::NoData(Some(format!(
            "computed from daily means: {years} years of {:02}-{:02}, {DEFAULT_MIN_YEARS} needed",
            key.0, key.1
        ))));
    };
    if reading.measurement == Measurement::GageHeight {
        percentile.datum = Some(VerticalDatum::Gauge);
    }
    percentile.provenance = Some(provenance(
        &request.target,
        reading.observed_at,
        Quality::Provisional,
    )?);
    out.push(reading.measurement, FragmentValue::Percentile(percentile))
}

/// A station of the `hydrometric-stations` collection.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    /// The Water Survey station number (`05BH004`).
    pub id: String,
    /// The name, as the service writes it (upper case).
    pub name: String,
    /// Province, territory or (border stations) state code.
    pub region: String,
    /// `Active` or `Discontinued`.
    pub status: String,
    /// The operating agency; empty when unknown.
    pub contributor: String,
    /// The datum `LEVEL` is referred to, as text (`ASSUMED DATUM`); empty when unknown.
    pub vertical_datum: String,
    /// Whether the station reports in real time.
    pub real_time: bool,
    /// Latitude, degrees north, five decimals.
    pub latitude: f64,
    /// Longitude, degrees east, five decimals.
    pub longitude: f64,
    /// Gross drainage area, km².
    pub drainage_area_km2: Option<f64>,
}

impl Station {
    /// The station as a [`StationRef`].
    pub fn station_ref(&self) -> StationRef {
        StationRef {
            source: SourceId::ECCC_HYDROMETRIC,
            id: self.id.clone(),
            name: self.name.clone(),
            kind: StationKind::WaterLevel,
            latitude: self.latitude,
            longitude: self.longitude,
            bin: None,
            reference_id: None,
        }
    }

    /// Whether `LEVEL` is a height above sea level: the station names a real datum rather than
    /// an assumed or unknown one. Levels are [`VerticalDatum::Gauge`] either way.
    pub fn level_is_elevation(&self) -> bool {
        let d = self.vertical_datum.to_ascii_uppercase();
        !d.is_empty() && !d.contains("ASSUMED")
    }

    /// The station as a discovery candidate. A real-time station reports level, discharge or
    /// both; the stations collection does not say which, so both are offered, discharge as the
    /// primary product, and a missing one shows as no value later.
    pub fn candidate(&self) -> StationCandidate {
        let offer = |primary| Coverage {
            last_value_at: None,
            primary,
        };
        StationCandidate {
            station: self.station_ref(),
            offers: BTreeMap::from([
                (Measurement::Discharge, offer(true)),
                (Measurement::GageHeight, offer(false)),
            ]),
        }
    }
}

/// A page of stations.
#[derive(Clone, Debug, PartialEq)]
pub struct StationPage {
    /// The stations of the page.
    pub stations: Vec<Station>,
    /// How many stations match in all.
    pub matched: Option<u64>,
    /// The `offset` of the next page, when there is one.
    pub next_offset: Option<u32>,
}

#[derive(Deserialize)]
struct StationProps {
    #[serde(rename = "STATION_NUMBER")]
    id: String,
    #[serde(rename = "STATION_NAME")]
    name: Option<String>,
    #[serde(rename = "PROV_TERR_STATE_LOC")]
    region: Option<String>,
    #[serde(rename = "STATUS_EN")]
    status: Option<String>,
    #[serde(rename = "CONTRIBUTOR_EN")]
    contributor: Option<String>,
    #[serde(rename = "VERTICAL_DATUM")]
    vertical_datum: Option<String>,
    #[serde(rename = "REAL_TIME")]
    real_time: Option<u8>,
    #[serde(rename = "DRAINAGE_AREA_GROSS")]
    drainage_area: Option<f64>,
}

fn round5(v: f64) -> f64 {
    (v * 100_000.0).round() / 100_000.0
}

/// Parses a response to [`stations_request`]. A station without a position is skipped.
///
/// # Errors
/// [`SourceError`] for an error status or a body of the wrong shape.
pub fn parse_stations(response: &Response<'_>) -> Result<StationPage, SourceError> {
    let body = check(response)?;
    let next_offset = next_offset(body)?;
    let page: Collection<StationProps> = collection(body)?;
    let matched = page.matched;
    let stations = page
        .features
        .into_iter()
        .filter_map(|f| {
            let coordinates = f.geometry?.coordinates?;
            let (lon, lat) = (*coordinates.first()?, *coordinates.get(1)?);
            let p = f.properties;
            Some(Station {
                name: p.name.unwrap_or_else(|| p.id.clone()),
                id: p.id,
                region: p.region.unwrap_or_default(),
                status: p.status.unwrap_or_default(),
                contributor: p.contributor.unwrap_or_default(),
                vertical_datum: p.vertical_datum.unwrap_or_default(),
                real_time: p.real_time == Some(1),
                latitude: round5(lat),
                longitude: round5(lon),
                drainage_area_km2: p.drainage_area,
            })
        })
        .collect();
    Ok(StationPage {
        stations,
        matched,
        next_offset,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use underbite_core::provenance::Relation;

    use super::*;
    use crate::request::Window;
    use crate::testutil::fixture_bytes;

    const NOW: UnixTime = UnixTime(1_794_909_600); // 2026-11-17T10:00:00Z

    fn t(s: &str) -> UnixTime {
        parse_rfc3339(s).unwrap()
    }

    fn target(measurements: &[Measurement]) -> Target {
        Target {
            place: Place::Station {
                station: StationRef {
                    source: SourceId::ECCC_HYDROMETRIC,
                    id: "05BH004".to_string(),
                    name: "BOW RIVER AT CALGARY".to_string(),
                    kind: StationKind::WaterLevel,
                    latitude: 51.05027,
                    longitude: -114.05146,
                    bin: None,
                    reference_id: None,
                },
                distance_m: Some(900.0),
                relation: Relation::SameWaterBody,
            },
            measurements: measurements.iter().copied().collect::<BTreeSet<_>>(),
            window: Window {
                from: UnixTime(NOW.0 - 3_600),
                to: NOW,
            },
            datum: None,
        }
    }

    fn ctx() -> FetchContext {
        FetchContext::new(NOW, "underbite/test")
    }

    fn request(url: &str) -> EcccRequest {
        EcccRequest {
            url: url.to_string(),
            resource: EcccResource::Realtime,
            target: target(MEASUREMENTS),
            required: true,
            reading: None,
        }
    }

    fn realtime(file: &str, dir: &str, now: UnixTime) -> Result<Fragments, SourceError> {
        let body = fixture_bytes(dir, file);
        let mut out = Fragments::new();
        let ctx = FetchContext::new(now, "underbite/test");
        Eccc::default().parse(&request("u"), &Response::new(200, &body), &ctx, &mut out)?;
        Ok(out)
    }

    fn series(out: &Fragments, m: Measurement) -> &Series<f32> {
        out.fragments()
            .iter()
            .find(|f| f.measurement == m)
            .and_then(|f| match &f.value {
                FragmentValue::Series(s) => Some(s),
                FragmentValue::Heights(h) => Some(&h.series),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn the_realtime_request_asks_for_the_newest_rows_of_one_station() {
        let t = target(MEASUREMENTS);
        let plan = Eccc::default().plan(&t, &ctx()).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].url,
            "https://api.weather.gc.ca/collections/hydrometric-realtime/items?f=json\
             &STATION_NUMBER=05BH004&sortby=-DATETIME&limit=13&skipGeometry=true\
             &properties=DATETIME,DATETIME_LST,LEVEL,DISCHARGE,LEVEL_SYMBOL_EN,DISCHARGE_SYMBOL_EN"
        );
        assert!(plan[0].required);
    }

    #[test]
    fn the_row_count_follows_the_window_within_bounds() {
        let mut t = target(MEASUREMENTS);
        t.window.from = UnixTime(NOW.0 - 30 * 86_400);
        let url = Eccc::default().plan(&t, &ctx()).unwrap().remove(0).url;
        assert!(url.contains("limit=1000&"), "{url}");
        t.window.from = NOW;
        let url = Eccc::default().plan(&t, &ctx()).unwrap().remove(0).url;
        assert!(url.contains("limit=12&"), "{url}");
    }

    #[test]
    fn only_stations_and_known_measurements_are_planned() {
        let e = Eccc::default();
        assert!(matches!(
            e.plan(&target(&[Measurement::Tide]), &ctx()),
            Err(SourceError::Unsupported(_))
        ));
        let mut t = target(MEASUREMENTS);
        t.place = Place::Point(ctx().round(crate::request::Coordinates {
            latitude: 51.05,
            longitude: -114.05,
        }));
        assert!(matches!(
            e.plan(&t, &ctx()),
            Err(SourceError::Unsupported(_))
        ));
        let mut t = target(MEASUREMENTS);
        if let Place::Station { station, .. } = &mut t.place {
            station.id = "05BH004&limit=5".to_string();
        }
        assert!(matches!(
            e.plan(&t, &ctx()),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn level_is_metres_on_the_gauge_datum_and_discharge_is_cubic_metres_per_second() {
        // The Datamart documentation gives `LEVEL` in metres and `DISCHARGE` in m3/s, so the
        // parser passes both through unconverted; the Bow River at Calgary reads 0.865 m and
        // 52.6 m3/s in the recording.
        let out = realtime("realtime-05BH004-limit12.json", "eccc", NOW).unwrap();
        let discharge = series(&out, Measurement::Discharge);
        let latest = discharge.latest().unwrap();
        assert_eq!(latest.value, 52.6);
        assert_eq!(latest.time, t("2026-11-17T09:05:00Z"));
        assert_eq!(discharge.samples().len(), 12);
        let level = out
            .fragments()
            .iter()
            .find_map(|f| match &f.value {
                FragmentValue::Heights(h) => Some(h),
                _ => None,
            })
            .unwrap();
        assert_eq!(level.datum, VerticalDatum::Gauge);
        assert_eq!(level.series.latest().unwrap().value, 0.865);
    }

    #[test]
    fn real_time_values_are_provisional_and_credited_as_such() {
        let out = realtime("realtime-05BH004-limit12.json", "eccc", NOW).unwrap();
        let s = series(&out, Measurement::Discharge);
        let p = s.provenance();
        assert_eq!(p.source, SourceId::ECCC_HYDROMETRIC);
        assert_eq!(p.quality, Quality::Provisional);
        assert_eq!(p.kind, ValueKind::Observed);
        assert_eq!(p.relation, Relation::SameWaterBody);
        assert_eq!(p.observed_at, t("2026-11-17T09:05:00Z"));
        assert!(
            s.samples()
                .iter()
                .all(|x| x.quality == Some(Quality::Provisional))
        );
        assert!(
            out.attributions()
                .iter()
                .any(|a| a.credit.contains("provisional"))
        );
    }

    #[test]
    fn the_scouts_earlier_recording_parses_too() {
        let out = realtime(
            "eccc-hydrometric-realtime-05BH004.json",
            "tier2",
            UnixTime(t("2026-11-08T15:00:00Z").0),
        )
        .unwrap();
        assert_eq!(
            series(&out, Measurement::Discharge).latest().unwrap().value,
            55.5
        );
        assert_eq!(
            series(&out, Measurement::GageHeight)
                .latest()
                .unwrap()
                .value,
            0.891
        );
    }

    #[test]
    fn only_the_wanted_measurements_are_returned() {
        let body = fixture_bytes("eccc", "realtime-05BH004-limit12.json");
        let mut out = Fragments::new();
        let mut r = request("u");
        r.target = target(&[Measurement::Discharge]);
        Eccc::default()
            .parse(&r, &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        assert_eq!(out.fragments().len(), 1);
        assert_eq!(out.fragments()[0].measurement, Measurement::Discharge);
    }

    #[test]
    fn the_standard_time_offset_comes_from_the_local_string_never_read_as_utc() {
        let body = fixture_bytes("eccc", "realtime-05BH004-limit12.json");
        let rows = realtime_rows(&body).unwrap();
        assert_eq!(rows[0].offset, UtcOffset(-7 * 3_600));
        // 2026-11-17T09:05Z is 02:05 on the 17th in Calgary standard time.
        assert_eq!(rows[0].time.to_local(rows[0].offset).date.day, 17);
        // 03:00Z is still the previous standard-time day.
        let early = standard_offset(t("2026-11-17T03:00:00Z"), Some("2026-11-16T20:00:00-07:00"));
        assert_eq!(early.unwrap(), UtcOffset(-7 * 3_600));
        assert!(matches!(
            standard_offset(t("2026-11-17T03:00:00Z"), Some("2026-11-16T20:07:00-07:00")),
            Err(SourceError::Parse(_))
        ));
        assert!(matches!(
            standard_offset(t("2026-11-17T03:00:00Z"), Some("2026-11-18T03:00:00-07:00")),
            Err(SourceError::Parse(_))
        ));
        assert!(matches!(
            standard_offset(t("2026-11-17T03:00:00Z"), Some("nonsense")),
            Err(SourceError::Parse(_))
        ));
    }

    type RowSpec<'a> = (&'a str, &'a str, Option<f64>, Option<f64>, Option<&'a str>);

    fn realtime_json(rows: &[RowSpec<'_>]) -> Vec<u8> {
        let features: Vec<_> = rows
            .iter()
            .map(|(utc, lst, level, discharge, symbol)| {
                serde_json::json!({"properties": {
                    "DATETIME": utc, "DATETIME_LST": lst, "LEVEL": level,
                    "DISCHARGE": discharge, "DISCHARGE_SYMBOL_EN": symbol,
                    "LEVEL_SYMBOL_EN": null}})
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({"type":"FeatureCollection","features":features}))
            .unwrap()
    }

    #[test]
    fn a_station_without_a_level_gives_discharge_only_and_ice_is_flagged() {
        let body = realtime_json(&[
            (
                "2026-11-17T09:05:00Z",
                "2026-11-17T02:05:00-07:00",
                None,
                Some(12.3),
                Some("Ice Conditions"),
            ),
            (
                "2026-11-17T09:00:00Z",
                "2026-11-17T02:00:00-07:00",
                None,
                Some(12.2),
                None,
            ),
        ]);
        let mut out = Fragments::new();
        Eccc::default()
            .parse(&request("u"), &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        assert_eq!(out.fragments().len(), 1);
        let s = series(&out, Measurement::Discharge);
        assert_eq!(s.provenance().quality, Quality::IceAffected);
        assert_eq!(s.samples()[0].quality, Some(Quality::Provisional));
    }

    #[test]
    fn symbols_map_to_quality() {
        assert_eq!(quality(None), Quality::Provisional);
        assert_eq!(quality(Some("Ice Conditions")), Quality::IceAffected);
        assert_eq!(quality(Some("Estimate")), Quality::Estimated);
        assert_eq!(quality(Some("Partial Day")), Quality::Estimated);
        assert_eq!(quality(Some("Dry")), Quality::Flagged);
        assert_eq!(quality(Some("Revised")), Quality::Provisional);
    }

    #[test]
    fn a_dead_station_is_stale_and_an_empty_page_is_no_data() {
        let later = UnixTime(NOW.0 + 13 * 3_600);
        match realtime("realtime-05BH004-limit12.json", "eccc", later) {
            Err(SourceError::Stale { latest }) => assert_eq!(latest, t("2026-11-17T09:05:00Z")),
            other => panic!("{other:?}"),
        }
        let empty = br#"{"type":"FeatureCollection","features":[]}"#;
        let mut out = Fragments::new();
        let r = Eccc::default().parse(&request("u"), &Response::new(200, empty), &ctx(), &mut out);
        assert!(matches!(r, Err(SourceError::NoData(_))), "{r:?}");
    }

    #[test]
    fn errors_are_classified() {
        let body = fixture_bytes("eccc", "error-400-limit.json");
        let check_status = |status, body: &[u8]| check(&Response::new(status, body)).map(|_| ());
        assert_eq!(
            check_status(400, &body),
            Err(SourceError::Status {
                status: 400,
                message: Some("limit value should be an integer".to_string())
            })
        );
        assert!(matches!(
            check_status(404, b"{}"),
            Err(SourceError::NotFound(_))
        ));
        let mut limited = Response::new(429, b"");
        limited.retry_after = Some(30);
        assert_eq!(
            check(&limited).map(|_| ()),
            Err(SourceError::RateLimited {
                retry_after_s: Some(30)
            })
        );
        assert!(matches!(
            check_status(200, b""),
            Err(SourceError::NoData(_))
        ));
        assert!(matches!(
            check_status(200, b"<html>"),
            Err(SourceError::Parse(_))
        ));
        assert!(matches!(
            check_status(503, b"down"),
            Err(SourceError::Status { status: 503, .. })
        ));
        let mut out = Fragments::new();
        let r = Eccc::default().parse(
            &request("u"),
            &Response::new(200, b"{\"features\": 3}"),
            &ctx(),
            &mut out,
        );
        assert!(matches!(r, Err(SourceError::Parse(_))));
    }

    // Daily means.

    fn daily_response() -> Vec<u8> {
        fixture_bytes("eccc", "daily-mean-05BH004-2023-2025-compact.json")
    }

    #[test]
    fn daily_means_are_rounded_ascending_and_keep_their_symbols() {
        let days = parse_daily_means(&Response::new(200, &daily_response())).unwrap();
        assert_eq!(days.len(), 1_096);
        let first = &days[0];
        assert_eq!(
            first.date,
            LocalDate {
                year: 2023,
                month: 1,
                day: 1
            }
        );
        assert_eq!(first.level, Some(1.557));
        assert_eq!(first.discharge, Some(43.8));
        assert_eq!(first.discharge_symbol.as_deref(), Some("Ice Conditions"));
        assert_eq!(first.level_symbol, None);
        assert!(days.windows(2).all(|w| w[0].date != w[1].date));
        assert_eq!(
            days[1_095].date,
            LocalDate {
                year: 2025,
                month: 12,
                day: 31
            }
        );
    }

    fn reading(m: Measurement, value: f32) -> Reading {
        Reading {
            measurement: m,
            value,
            observed_at: t("2026-11-17T09:05:00Z"),
            day: LocalDate {
                year: 2026,
                month: 11,
                day: 17,
            },
        }
    }

    #[test]
    fn the_daily_request_asks_for_the_stations_own_history_up_to_yesterday() {
        let e = Eccc::default().with_history_years(15);
        let r = e
            .percentile_request(
                &target(MEASUREMENTS),
                reading(Measurement::GageHeight, 0.865),
            )
            .unwrap();
        assert_eq!(
            r.url,
            "https://api.weather.gc.ca/collections/hydrometric-daily-mean/items?f=json\
             &STATION_NUMBER=05BH004&datetime=2011-01-01%2F2026-11-16&sortby=DATE&limit=5856\
             &skipGeometry=true&properties=DATE,LEVEL"
        );
        assert!(!r.required);
        assert_eq!(r.resource, EcccResource::DailyMean);
        let r = e
            .percentile_request(&target(MEASUREMENTS), reading(Measurement::Discharge, 52.6))
            .unwrap();
        assert!(r.url.ends_with("properties=DATE,DISCHARGE"), "{}", r.url);
        assert!(matches!(
            e.percentile_request(&target(MEASUREMENTS), reading(Measurement::Tide, 1.0)),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn three_years_of_history_are_too_few_for_a_normal() {
        let r = Eccc::default()
            .percentile_request(&target(MEASUREMENTS), reading(Measurement::Discharge, 52.6))
            .unwrap();
        let mut out = Fragments::new();
        let err = Eccc::default()
            .parse(&r, &Response::new(200, &daily_response()), &ctx(), &mut out)
            .unwrap_err();
        match err {
            SourceError::NoData(Some(m)) => {
                assert!(
                    m.contains("3 years of 11-17") && m.contains("10 needed"),
                    "{m}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    fn history_json(first: i32, last: i32) -> Vec<u8> {
        // One value per day, rising with the year, so the 17 November normals are known.
        let mut features = Vec::new();
        for year in first..=last {
            for (month, day) in [(11u8, 16u8), (11, 17), (11, 18)] {
                let v = 40.0 + f64::from(year - first) + f64::from(day) / 100.0;
                features.push(serde_json::json!({"properties": {
                    "DATE": format!("{year}-{month:02}-{day:02}"), "DISCHARGE": v, "LEVEL": null}}));
            }
        }
        serde_json::to_vec(&serde_json::json!({"features": features})).unwrap()
    }

    #[test]
    fn a_long_record_gives_a_computed_normal_with_the_reading_ranked_in_it() {
        let r = Eccc::default()
            .percentile_request(&target(MEASUREMENTS), reading(Measurement::Discharge, 50.0))
            .unwrap();
        let mut out = Fragments::new();
        Eccc::default()
            .parse(
                &r,
                &Response::new(200, &history_json(2006, 2025)),
                &ctx(),
                &mut out,
            )
            .unwrap();
        let FragmentValue::Percentile(p) = &out.fragments()[0].value else {
            panic!("not a percentile");
        };
        assert_eq!(out.fragments()[0].measurement, Measurement::Discharge);
        assert_eq!(p.day_of_year, "11-17");
        assert_eq!(
            p.basis,
            underbite_core::model::PercentileBasis::ComputedFromDaily { years: 20 }
        );
        assert_eq!(p.sample_count, 20);
        assert_eq!(p.min, Some(40.17));
        assert_eq!(p.max, Some(59.17));
        assert_eq!(p.datum, None);
        assert!(p.rank.is_some() && p.class.is_some());
        let prov = p.provenance.as_ref().unwrap();
        assert_eq!(prov.source, SourceId::ECCC_HYDROMETRIC);
        assert_eq!(prov.quality, Quality::Provisional);
    }

    #[test]
    fn a_level_normal_is_on_the_gauge_datum_and_leaves_the_current_day_out() {
        let mut body =
            serde_json::from_slice::<serde_json::Value>(&history_json(2006, 2025)).unwrap();
        // A row for the reading's own day must not count in its normals.
        body["features"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"properties": {
            "DATE": "2026-11-17", "DISCHARGE": 1000.0, "LEVEL": 99.0}}));
        let body = serde_json::to_vec(&body).unwrap();
        let r = Eccc::default()
            .percentile_request(&target(MEASUREMENTS), reading(Measurement::Discharge, 50.0))
            .unwrap();
        let mut out = Fragments::new();
        Eccc::default()
            .parse(&r, &Response::new(200, &body), &ctx(), &mut out)
            .unwrap();
        let FragmentValue::Percentile(p) = &out.fragments()[0].value else {
            panic!();
        };
        assert_eq!(p.max, Some(59.17));
        // Levels are absent in this history: no years, no normal.
        let r = Eccc::default()
            .percentile_request(&target(MEASUREMENTS), reading(Measurement::GageHeight, 0.9))
            .unwrap();
        let mut out = Fragments::new();
        assert!(matches!(
            Eccc::default().parse(&r, &Response::new(200, &body), &ctx(), &mut out),
            Err(SourceError::NoData(_))
        ));
    }

    #[test]
    fn the_latest_reading_carries_the_standard_time_day() {
        let body = fixture_bytes("eccc", "realtime-05BH004-limit12.json");
        let got = Eccc::default()
            .latest_reading(&Response::new(200, &body), Measurement::GageHeight)
            .unwrap();
        assert_eq!(got.value, 0.865);
        assert_eq!(
            got.day,
            LocalDate {
                year: 2026,
                month: 11,
                day: 17
            }
        );
        assert_eq!(got.observed_at, t("2026-11-17T09:05:00Z"));
    }

    // Stations.

    #[test]
    fn the_station_request_filters_to_active_real_time_stations_in_a_rounded_box() {
        let url = stations_request(
            Bbox {
                west: -114.2,
                south: 50.904,
                east: -113.896,
                north: 51.2,
            },
            10,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://api.weather.gc.ca/collections/hydrometric-stations/items?f=json\
             &bbox=-114.20,50.90,-113.90,51.20&STATUS_EN=Active&REAL_TIME=1\
             &sortby=STATION_NUMBER&limit=100&offset=10"
        );
        assert!(
            stations_request(
                Bbox {
                    west: 1.0,
                    south: 0.0,
                    east: 0.0,
                    north: 1.0
                },
                0
            )
            .is_err()
        );
        assert!(
            stations_request(
                Bbox {
                    west: f64::NAN,
                    south: 0.0,
                    east: 0.0,
                    north: 1.0
                },
                0
            )
            .is_err()
        );
    }

    #[test]
    fn a_station_page_lists_stations_with_their_datum_and_the_next_offset() {
        let body = fixture_bytes("eccc", "stations-bbox-calgary-limit10.json");
        let page = parse_stations(&Response::new(200, &body)).unwrap();
        assert_eq!(page.stations.len(), 10);
        assert_eq!(page.matched, Some(12));
        assert_eq!(page.next_offset, Some(10));
        let bow = page.stations.iter().find(|s| s.id == "05BH004").unwrap();
        assert_eq!(bow.name, "BOW RIVER AT CALGARY");
        assert_eq!(bow.region, "AB");
        assert_eq!(bow.status, "Active");
        assert_eq!(bow.vertical_datum, "ASSUMED DATUM");
        assert!(bow.real_time && !bow.level_is_elevation());
        assert_eq!(bow.drainage_area_km2, Some(7874.0));
        assert_eq!((bow.latitude, bow.longitude), (51.05027, -114.05146));
        let glenmore = page.stations.iter().find(|s| s.id == "05BJ008").unwrap();
        assert!(glenmore.level_is_elevation());
        let unknown = page.stations.iter().find(|s| s.id == "05BH002").unwrap();
        assert!(!unknown.level_is_elevation() && !unknown.real_time);
        let last = fixture_bytes("eccc", "stations-bbox-calgary-limit3-offset3.json");
        assert_eq!(
            parse_stations(&Response::new(200, &last))
                .unwrap()
                .stations
                .len(),
            3
        );
        let one = fixture_bytes("eccc", "stations-05BH004.json");
        assert_eq!(
            parse_stations(&Response::new(200, &one))
                .unwrap()
                .next_offset,
            None
        );
    }

    #[test]
    fn a_station_is_a_discovery_candidate_for_discharge_and_level() {
        let body = fixture_bytes("eccc", "stations-05BH004.json");
        let page = parse_stations(&Response::new(200, &body)).unwrap();
        let c = page.stations[0].candidate();
        assert_eq!(c.station.id, "05BH004");
        assert_eq!(c.station.source, SourceId::ECCC_HYDROMETRIC);
        assert_eq!(c.offers.len(), 2);
        assert!(c.offers[&Measurement::Discharge].primary);
        assert!(!c.offers[&Measurement::GageHeight].primary);
    }

    #[test]
    fn a_next_link_off_the_api_is_refused() {
        let body = br#"{"features":[],"links":[{"rel":"next","href":"https://example.com/items?offset=5"}]}"#;
        assert!(matches!(next_offset(body), Err(SourceError::Parse(_))));
        let body = br#"{"features":[],"links":[{"rel":"next","href":"https://api.weather.gc.ca/collections/x/items?offset=5&limit=1"}]}"#;
        assert_eq!(next_offset(body).unwrap(), Some(5));
    }

    #[test]
    fn the_source_describes_itself() {
        let e = Eccc::default();
        assert_eq!(
            e.measurements(),
            [Measurement::Discharge, Measurement::GageHeight]
        );
        let a = e.attribution();
        assert_eq!(a.source, SourceId::ECCC_HYDROMETRIC);
        assert_eq!(
            a.credit,
            "Data Source: Environment and Climate Change Canada"
        );
        assert_eq!(
            a.url.as_deref(),
            Some("https://eccc-msc.github.io/open-data/licence/readme_en/")
        );
    }
}
