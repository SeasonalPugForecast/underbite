//! Fisheries and Oceans Canada, Canadian Hydrographic Service: the Integrated Water Level System (IWLS) for tide predictions, water levels and currents.
//!
//! Tides and water levels for Canadian stations: high and low water (`wlp-hilo`), predicted
//! levels at an interval (`wlp`) and observed levels (`wlo`). Heights are metres above the
//! station's chart datum and are tagged [`VerticalDatum::ChsChartDatum`]; they are never converted
//! to a US datum. A Great Lakes station also lists its chart datum's elevation on IGLD 1985 in
//! its metadata (`datums[IGLD85].offset`); given that offset, [`plan_with`] asks for heights in
//! [`VerticalDatum::Igld85`] and the parsers add it.
//!
//! The service has no bounding-box filter: [`stations_url`] fetches the whole list or one
//! region's and the caller filters locally. Data requests take the station's `id` (24 hex
//! characters), not its five-digit `code`, which is why a station's [`StationRef::id`] here is
//! the hex id. The limits are 3 requests per second and 30 per minute per client IP, so the host
//! spaces requests at least [`MIN_REQUEST_SPACING_S`] seconds apart; HTTP 429 is
//! [`SourceError::RateLimited`] and is not retried.
//!
//! Registering the source needs a [`Resource`] variant per [`DfoResource`]; until then the
//! planner and the parser are the free functions [`plan_with`] and [`parse_with`], which take the
//! mapping between [`DfoResource`] and [`Resource`].

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::Deserialize;
use underbite_core::datum::{Heights, VerticalDatum};
use underbite_core::model::{
    Measurement, Requirements, TideEvent, TideKind, TideSeries, TideSourceKind,
};
use underbite_core::provenance::{
    Attribution, Provenance, Quality, Relation, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::{Sample, Series};
use underbite_core::time::{UnixTime, parse_rfc3339};
use whether_core::time::civil_from_days;

use crate::request::{
    Place, QueryBuilder, Request, Resource, Response, Target, Window, encode_path_segment,
};
use crate::source::{
    Budget, FetchContext, FragmentValue, Fragments, Period, SourceError, check_response,
};

/// The API base.
pub const BASE: &str = "https://api-iwls.dfo-mpo.gc.ca/api/v1";

/// The source id.
pub const SOURCE_ID: SourceId = SourceId::DFO_IWLS;

/// Shortest wait between two requests, seconds: 2 s keeps both the 3 per second and the 30 per
/// minute limits.
pub const MIN_REQUEST_SPACING_S: u32 = 2;

/// The most metered requests per hour, the 30 per minute limit over an hour. Hosts also keep
/// [`MIN_REQUEST_SPACING_S`], which a per-hour count cannot express.
pub const REQUESTS_PER_HOUR: u32 = 1_800;

/// Most days of `wlp-hilo` predictions per request (the time-series definition allows 366).
pub const MAX_HILO_DAYS: i64 = 365;

/// Most days of any interval series per request, whatever the resolution.
pub const MAX_INTERVAL_DAYS: i64 = 31;

/// The measurements this source supplies.
pub const MEASUREMENTS: [Measurement; 1] = [Measurement::Tide];

/// How long a prediction response may be reused, seconds.
const PREDICTION_AGE_S: u32 = 6 * 3_600;
/// How long an observation response may be reused, seconds.
const OBSERVATION_AGE_S: u32 = 600;

const DAY: i64 = 86_400;
const CURVE_STEP: Duration = Duration::from_mins(6);

/// The notice the CHS licence asks derivative products to carry in a prominent place, less the
/// user's name.
const LICENCE_NOTICE: &str = "This product is not to be used for navigation. This product contains \
    intellectual property (Data) of the Canadian Hydrographic Service of the Department of \
    Fisheries and Oceans. The copyrights in the Data are and remain the property of His Majesty \
    the King in Right of Canada and shall not be sold, licensed, leased, assigned or given to a \
    third party. The incorporation of the Data in this product does not constitute an \
    endorsement or an approval of this product by the Canadian Hydrographic Service, the \
    Department of Fisheries and Oceans or His Majesty the King in Right of Canada.";

/// The credit every display of IWLS data shows, with the licence notice.
pub fn attribution() -> Attribution {
    Attribution {
        source: SOURCE_ID,
        credit: format!(
            "Canadian Hydrographic Service, Fisheries and Oceans Canada. {LICENCE_NOTICE}"
        ),
        url: Some("https://www.tides.gc.ca/en/licence-agreement".into()),
    }
}

/// The usage budget in the units a [`Budget`] has; see [`MIN_REQUEST_SPACING_S`].
pub fn budget() -> Budget {
    Budget {
        period: Period::Hour,
        limit: REQUESTS_PER_HOUR,
    }
}

/// The data resources of this source, for the registration to map onto [`Resource`] variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DfoResource {
    /// `data?time-series-code=wlp-hilo`: high and low water predictions.
    TideHighLow,
    /// `data?time-series-code=wlp`: predicted water level at an interval.
    TideCurve,
    /// `data?time-series-code=wlo`: observed water level.
    WaterLevel,
}

/// The resolutions of a `data` request (`resolution`); the service defaults to one minute, so a
/// request always names one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resolution {
    /// `ONE_MINUTE`.
    OneMinute,
    /// `THREE_MINUTES`.
    ThreeMinutes,
    /// `FIVE_MINUTES`.
    FiveMinutes,
    /// `FIFTEEN_MINUTES`.
    FifteenMinutes,
    /// `SIXTY_MINUTES`.
    SixtyMinutes,
}

impl Resolution {
    /// The query parameter value.
    pub fn param(self) -> &'static str {
        match self {
            Self::OneMinute => "ONE_MINUTE",
            Self::ThreeMinutes => "THREE_MINUTES",
            Self::FiveMinutes => "FIVE_MINUTES",
            Self::FifteenMinutes => "FIFTEEN_MINUTES",
            Self::SixtyMinutes => "SIXTY_MINUTES",
        }
    }

    /// Minutes between samples.
    pub fn minutes(self) -> i64 {
        match self {
            Self::OneMinute => 1,
            Self::ThreeMinutes => 3,
            Self::FiveMinutes => 5,
            Self::FifteenMinutes => 15,
            Self::SixtyMinutes => 60,
        }
    }

    /// The longest span one request may cover, days: 7 days times the resolution in minutes,
    /// at most [`MAX_INTERVAL_DAYS`] (a week of 1-minute data, three weeks of 3-minute data, a
    /// month otherwise).
    pub fn max_days(self) -> i64 {
        (7 * self.minutes()).min(MAX_INTERVAL_DAYS)
    }
}

/// The time-series codes this source reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Code {
    HighLow,
    Predicted,
    Observed,
}

impl Code {
    fn param(self) -> &'static str {
        match self {
            Self::HighLow => "wlp-hilo",
            Self::Predicted => "wlp",
            Self::Observed => "wlo",
        }
    }
}

/// `t` as `YYYY-MM-DDTHH:MM:SSZ`, the only form the service accepts.
pub fn format_time(time: UnixTime) -> String {
    let (year, month, day) = civil_from_days(time.0.div_euclid(DAY));
    let secs = time.0.rem_euclid(DAY);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

/// `window` cut into consecutive pieces of at most `max_days` days each, without overlap (each
/// ends one second before the next starts, inside the service's "not bigger than" limit).
pub fn chunks(window: Window, max_days: i64) -> Vec<Window> {
    let span = max_days.max(1) * DAY;
    let mut out = Vec::new();
    let mut from = window.from.0;
    while from <= window.to.0 {
        let to = window.to.0.min(from.saturating_add(span - 1));
        out.push(Window {
            from: UnixTime(from),
            to: UnixTime(to),
        });
        from = to.saturating_add(1);
    }
    out
}

/// A chart-region filter of the station list (`chs-region-code`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// Atlantic.
    Atlantic,
    /// Quebec.
    Quebec,
    /// Pacific.
    Pacific,
    /// Central and Arctic.
    CentralArctic,
}

impl Region {
    /// The query parameter value.
    pub fn param(self) -> &'static str {
        match self {
            Self::Atlantic => "ATL",
            Self::Quebec => "QUE",
            Self::Pacific => "PAC",
            Self::CentralArctic => "CNA",
        }
    }
}

/// The station list: every station (about 900 kB) or one region's.
pub fn stations_url(region: Option<Region>) -> String {
    let mut url = QueryBuilder::new(&format!("{BASE}/stations"));
    if let Some(region) = region {
        url = url.param("chs-region-code", region.param());
    }
    url.finish()
}

/// One station by its five-digit code; the record carries the `id` data requests need.
pub fn station_by_code_url(code: &str) -> String {
    QueryBuilder::new(&format!("{BASE}/stations"))
        .param("code", code)
        .finish()
}

/// A station's metadata (datums, tidal or not, time zone).
pub fn metadata_url(station_id: &str) -> String {
    format!(
        "{BASE}/stations/{}/metadata",
        encode_path_segment(station_id)
    )
}

/// A data request for one series of one station. `resolution` is sent for interval series and
/// left out for `wlp-hilo`.
fn data_url(
    station_id: &str,
    code: Code,
    window: Window,
    resolution: Option<Resolution>,
) -> String {
    let mut url = QueryBuilder::new(&format!(
        "{BASE}/stations/{}/data",
        encode_path_segment(station_id)
    ))
    .param("time-series-code", code.param())
    .param("from", format_time(window.from))
    .param("to", format_time(window.to));
    if let Some(resolution) = resolution {
        url = url.param("resolution", resolution.param());
    }
    url.finish()
}

/// A time series a station offers.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct StationSeries {
    /// `wlo`, `wlp`, `wlp-hilo`, `wcsp1`, …
    pub code: String,
    /// English name.
    #[serde(rename = "nameEn", default)]
    pub name_en: Option<String>,
    /// French name.
    #[serde(rename = "nameFr", default)]
    pub name_fr: Option<String>,
}

/// A station of the IWLS list.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Station {
    /// The id data requests use: 24 hex characters.
    pub id: String,
    /// The five-digit code, `"07735"`.
    pub code: String,
    /// The official name.
    #[serde(rename = "officialName")]
    pub official_name: String,
    /// Other names, English and French, comma separated (`Leaf Basin, Lac Aux Feuilles`).
    #[serde(rename = "alternativeName", default)]
    pub alternative_name: Option<String>,
    /// Latitude, degrees north.
    pub latitude: f64,
    /// Longitude, degrees east.
    pub longitude: f64,
    /// `ATL`, `QUE`, `PAC` or `CNA`.
    #[serde(rename = "chsRegionCode", default)]
    pub region: Option<String>,
    /// Province or territory code.
    #[serde(rename = "provinceCode", default)]
    pub province: Option<String>,
    /// `PERMANENT`, `TEMPORARY`, `SEASONAL`, `DISCONTINUED`, `VIRTUAL` or `UNKNOWN`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// Whether the station is operating now.
    #[serde(default)]
    pub operating: bool,
    /// The series the station has.
    #[serde(rename = "timeSeries", default)]
    pub series: Vec<StationSeries>,
}

impl Station {
    /// The series with `code`, if the station has it.
    pub fn series(&self, code: &str) -> Option<&StationSeries> {
        self.series.iter().find(|s| s.code == code)
    }

    /// Whether the station predicts tides (`wlp` or `wlp-hilo`).
    pub fn predicts_tides(&self) -> bool {
        self.series("wlp-hilo").is_some() || self.series("wlp").is_some()
    }

    /// Whether the station is worth offering for tides or water levels: operating and with a
    /// `wlo`, `wlp` or `wlp-hilo` series (the list holds many discontinued and temporary
    /// stations).
    pub fn offers_water_level(&self) -> bool {
        self.operating && (self.predicts_tides() || self.series("wlo").is_some())
    }

    /// The names a person might search: the official name and the alternative names.
    pub fn names(&self) -> Vec<&str> {
        let mut names = vec![self.official_name.as_str()];
        for name in self.alternative_name.iter().flat_map(|n| n.split(',')) {
            let name = name.trim();
            if !name.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                names.push(name);
            }
        }
        names
    }

    /// The station as a provenance record: `id` is the hex id data requests use, `name` the
    /// official name.
    pub fn station_ref(&self) -> StationRef {
        StationRef {
            source: SOURCE_ID,
            id: self.id.clone(),
            name: self.official_name.clone(),
            kind: if self.predicts_tides() {
                StationKind::TideHarmonic
            } else {
                StationKind::WaterLevel
            },
            latitude: self.latitude,
            longitude: self.longitude,
            bin: None,
            reference_id: None,
        }
    }
}

fn json<'a, T: Deserialize<'a>>(body: &'a [u8]) -> Result<T, SourceError> {
    serde_json::from_slice(body).map_err(|e| SourceError::Parse(e.to_string()))
}

/// The stations of a list response (`/stations`, or `/stations?code=…` with one record).
///
/// # Errors
/// The failure [`check_response`] finds, or [`SourceError::Parse`] for another shape.
pub fn parse_stations(
    request: &Request,
    response: &Response<'_>,
) -> Result<Vec<Station>, SourceError> {
    let body = check_response(request, response)?;
    json(body)
}

/// What a station's `/metadata` says that matters here.
#[derive(Clone, Debug, PartialEq)]
pub struct Metadata {
    /// Offsets of other vertical datums relative to chart datum, by code (`IGLD85`, `CGVD2013`).
    pub datums: BTreeMap<String, f32>,
    /// Whether the water is tidal.
    pub tidal: Option<bool>,
    /// `LAKE`, `RIVER`, `OCEAN`, …
    pub water_body: Option<String>,
    /// The IANA time zone of the station.
    pub time_zone: Option<String>,
}

impl Metadata {
    /// The elevation of the station's chart datum on IGLD 1985, metres: a height above chart
    /// datum plus this is the IGLD 85 elevation. Great Lakes stations only.
    pub fn igld85_offset_m(&self) -> Option<f32> {
        self.datums.get("IGLD85").copied()
    }
}

#[derive(Deserialize)]
struct RawDatum {
    code: String,
    offset: f64,
}

#[derive(Deserialize)]
struct RawMetadata {
    #[serde(default)]
    datums: Vec<RawDatum>,
    #[serde(rename = "isTidal", default)]
    is_tidal: Option<bool>,
    #[serde(rename = "waterBody", default)]
    water_body: Option<String>,
    #[serde(rename = "timeZoneCode", default)]
    time_zone: Option<String>,
}

/// A station's metadata.
///
/// # Errors
/// The failure [`check_response`] finds, or [`SourceError::Parse`] for another shape.
pub fn parse_metadata(request: &Request, response: &Response<'_>) -> Result<Metadata, SourceError> {
    let body = check_response(request, response)?;
    let raw: RawMetadata = json(body)?;
    Ok(Metadata {
        datums: raw
            .datums
            .into_iter()
            .map(|d| (d.code, d.offset as f32))
            .collect(),
        tidal: raw.is_tidal,
        water_body: raw.water_body,
        time_zone: raw.time_zone,
    })
}

/// The requests for the tide of a CHS station: high and low water (required), predicted levels
/// and observed levels (optional), each cut into pieces the service allows. `to_resource` maps
/// the three [`DfoResource`]s to the registered [`Resource`] variants. Every request is metered.
///
/// Heights are in [`VerticalDatum::ChsChartDatum`]. When the target asks for
/// [`VerticalDatum::Igld85`] and `igld85_offset_m` (from [`Metadata::igld85_offset_m`]) is known,
/// the requests carry the offset and the parsed heights are IGLD 85 elevations; without the
/// offset that request is [`SourceError::Unsupported`]. Any other datum asked for (a US datum from
/// the user's settings) is ignored: IWLS heights are never converted to it.
///
/// # Errors
/// [`SourceError::Unsupported`] unless the target is an IWLS station and wants a tide.
pub fn plan_with(
    target: &Target,
    requirements: &Requirements,
    ctx: &FetchContext,
    igld85_offset_m: Option<f32>,
    to_resource: impl Fn(DfoResource) -> Resource,
) -> Result<Vec<Request>, SourceError> {
    let Place::Station { station, .. } = &target.place else {
        return Err(SourceError::Unsupported("IWLS needs a station".into()));
    };
    if station.source != SOURCE_ID {
        return Err(SourceError::Unsupported(format!(
            "{} is not an IWLS station",
            station.id
        )));
    }
    let wanted = target.measurements.contains(&Measurement::Tide)
        && (requirements.measurements.is_empty()
            || requirements.measurements.contains(&Measurement::Tide));
    if !wanted {
        return Err(SourceError::Unsupported(format!(
            "IWLS offers only tides, not {:?}",
            target.measurements
        )));
    }
    let carry = match target.datum {
        Some(VerticalDatum::Igld85) => Some(
            igld85_offset_m
                .filter(|o| o.is_finite())
                .ok_or_else(|| {
                    SourceError::Unsupported(
                        "IGLD 85 heights need the station's IGLD85 datum offset".into(),
                    )
                })?
                .to_string(),
        ),
        _ => None,
    };
    let datum = if carry.is_some() {
        VerticalDatum::Igld85
    } else {
        VerticalDatum::ChsChartDatum
    };
    let interval = Some(Resolution::FifteenMinutes);
    let interval_days = Resolution::FifteenMinutes.max_days();
    let specs = [
        (Code::HighLow, DfoResource::TideHighLow, None, MAX_HILO_DAYS),
        (
            Code::Predicted,
            DfoResource::TideCurve,
            interval,
            interval_days,
        ),
        (
            Code::Observed,
            DfoResource::WaterLevel,
            interval,
            interval_days,
        ),
    ];
    let mut requests = Vec::new();
    for (code, resource, resolution, max_days) in specs {
        let observed = code == Code::Observed;
        let mut window = target.window;
        if observed {
            window.to = UnixTime(window.to.0.min(ctx.now.0));
        }
        for chunk in chunks(window, max_days) {
            let chunk_target = Target {
                place: target.place.clone(),
                measurements: BTreeSet::from([Measurement::Tide]),
                window: chunk,
                datum: Some(datum),
            };
            let url = data_url(&station.id, code, chunk, resolution);
            let age = if observed {
                OBSERVATION_AGE_S
            } else {
                PREDICTION_AGE_S
            };
            let mut request = Request::new(url, to_resource(resource), chunk_target)
                .metered()
                .max_age_hint(age);
            if let Some(carry) = &carry {
                request = request.with_carry(carry.clone());
            }
            if code != Code::HighLow {
                request = request.optional();
            }
            requests.push(request);
        }
    }
    Ok(requests)
}

/// One row of a `data` response.
#[derive(Deserialize)]
struct Row {
    #[serde(rename = "eventDate")]
    event_date: String,
    value: f64,
    #[serde(rename = "qcFlagCode", default)]
    qc: Option<String>,
    #[serde(default)]
    reviewed: bool,
}

/// A row's quality: `"3"` is questionable, `"1"` and reviewed is approved, anything else is
/// provisional.
fn row_quality(row: &Row) -> Quality {
    match row.qc.as_deref() {
        Some("3") => Quality::Flagged,
        Some("1") if row.reviewed => Quality::Approved,
        _ => Quality::Provisional,
    }
}

/// The least trusted quality among `qualities`.
fn worst(qualities: impl IntoIterator<Item = Quality>) -> Quality {
    let mut worst = Quality::Approved;
    for q in qualities {
        worst = match (worst, q) {
            (_, Quality::Flagged) | (Quality::Flagged, _) => Quality::Flagged,
            (Quality::Approved, other) | (other, Quality::Approved) => other,
            (current, _) => current,
        };
    }
    worst
}

type Rows = Vec<(UnixTime, f32, Quality)>;

/// The rows of a body: time, height in metres (in the requested datum) and quality.
fn rows(body: &[u8], datum_offset_m: f32) -> Result<Rows, SourceError> {
    let raw: Vec<Row> = json(body)?;
    let mut out = Vec::with_capacity(raw.len());
    for row in &raw {
        let time = parse_rfc3339(&row.event_date)
            .ok_or_else(|| SourceError::Parse(format!("bad eventDate {:?}", row.event_date)))?;
        if !row.value.is_finite() {
            return Err(SourceError::Parse("non-finite value".into()));
        }
        out.push((time, row.value as f32 + datum_offset_m, row_quality(row)));
    }
    if out.is_empty() {
        return Err(SourceError::NoData(Some("no values".into())));
    }
    Ok(out)
}

fn station_of(request: &Request) -> Result<(&StationRef, Option<f32>, Relation), SourceError> {
    match &request.target.place {
        Place::Station {
            station,
            distance_m,
            relation,
        } => Ok((station, *distance_m, *relation)),
        _ => Err(SourceError::Unsupported("IWLS data needs a station".into())),
    }
}

/// The datum of the heights and the offset to add to the service's values.
fn datum_of(request: &Request) -> Result<(VerticalDatum, f32), SourceError> {
    match request.carry() {
        Some(text) => {
            let offset: f32 = text
                .parse()
                .ok()
                .filter(|o: &f32| o.is_finite())
                .ok_or_else(|| SourceError::Parse("bad IGLD 85 offset".into()))?;
            Ok((VerticalDatum::Igld85, offset))
        }
        None => Ok((VerticalDatum::ChsChartDatum, 0.0)),
    }
}

fn provenance(
    request: &Request,
    kind: ValueKind,
    quality: Quality,
    observed_at: UnixTime,
) -> Result<Provenance, SourceError> {
    let (station, distance_m, relation) = station_of(request)?;
    Ok(Provenance {
        source: SOURCE_ID,
        station: Some(station.clone()),
        distance_m,
        relation,
        observed_at,
        kind,
        quality,
    })
}

/// `wlp-hilo`: highs and lows. A row does not say which it is, so each is classified against its
/// neighbours (higher than the next, or for the last row than the previous, is a high). The
/// curve between them is interpolated (`Computed`, `Estimated`); `wlp` supplies the real one.
///
/// # Errors
/// [`SourceError::NoData`] for no rows or one row, [`SourceError::Parse`] for a bad body.
pub fn tide_events(
    request: &Request,
    body: &[u8],
    now: UnixTime,
) -> Result<TideSeries, SourceError> {
    let (datum, offset) = datum_of(request)?;
    let mut rows = rows(body, offset)?;
    rows.sort_by_key(|r| r.0);
    if rows.len() < 2 {
        return Err(SourceError::NoData(Some(
            "one extremum cannot be classed as high or low".into(),
        )));
    }
    let mut events: Vec<TideEvent> = Vec::with_capacity(rows.len());
    for (i, &(time, height, _)) in rows.iter().enumerate() {
        let against = match rows.get(i + 1) {
            Some(next) => height - next.1,
            None => height - rows[i - 1].1,
        };
        let kind = if against > 0.0 {
            TideKind::High
        } else if against < 0.0 {
            TideKind::Low
        } else {
            // Equal neighbours: alternate, which is what extrema do.
            match events.last().map(|e| e.kind) {
                Some(TideKind::High) => TideKind::Low,
                _ => TideKind::High,
            }
        };
        events.push(TideEvent {
            time,
            kind,
            height_m: Some(height),
        });
    }
    let quality = worst(rows.iter().map(|r| r.2));
    let provenance = provenance(request, ValueKind::Predicted, quality, now)?;
    let samples = underbite_core::astro::tide::curve_from_events(&events, CURVE_STEP);
    let curve = (!samples.is_empty()).then(|| {
        let estimated = Provenance {
            kind: ValueKind::Computed,
            quality: Quality::Estimated,
            ..provenance.clone()
        };
        Series::new(estimated, samples)
    });
    Ok(TideSeries {
        source_kind: TideSourceKind::HarmonicStation,
        datum,
        events,
        curve,
        observed: None,
        provenance,
    })
}

fn samples(rows: Rows) -> Vec<Sample<f32>> {
    rows.into_iter()
        .map(|(time, value, quality)| Sample {
            time,
            value,
            quality: Some(quality),
        })
        .collect()
}

/// `wlp`: the predicted level at an interval, as a curve without events.
///
/// # Errors
/// [`SourceError::NoData`] for no rows, [`SourceError::Parse`] for a bad body.
pub fn tide_curve(
    request: &Request,
    body: &[u8],
    now: UnixTime,
) -> Result<TideSeries, SourceError> {
    let (datum, offset) = datum_of(request)?;
    let rows = rows(body, offset)?;
    let quality = worst(rows.iter().map(|r| r.2));
    let provenance = provenance(request, ValueKind::Predicted, quality, now)?;
    Ok(TideSeries {
        source_kind: TideSourceKind::HarmonicStation,
        datum,
        events: Vec::new(),
        curve: Some(Series::new(provenance.clone(), samples(rows))),
        observed: None,
        provenance,
    })
}

/// `wlo`: observed levels.
///
/// # Errors
/// [`SourceError::NoData`] for no rows, [`SourceError::Parse`] for a bad body.
pub fn water_level(request: &Request, body: &[u8]) -> Result<Heights, SourceError> {
    let (datum, offset) = datum_of(request)?;
    let rows = rows(body, offset)?;
    let quality = worst(rows.iter().map(|r| r.2));
    let latest = rows.iter().map(|r| r.0).max().unwrap_or(UnixTime(0));
    let provenance = provenance(request, ValueKind::Observed, quality, latest)?;
    Ok(Heights {
        datum,
        series: Series::new(provenance, samples(rows)),
    })
}

/// Parses the response to a request [`plan_with`] made into a tide fragment. Observed levels
/// stand alone (no events, no curve) for the host to merge with the predictions.
///
/// # Errors
/// The failure [`check_response`] finds (429 is [`SourceError::RateLimited`]; do not retry in a
/// loop), or [`SourceError::Parse`] for a body that is not the data array.
pub fn parse_with(
    request: &Request,
    resource: DfoResource,
    response: &Response<'_>,
    ctx: &FetchContext,
    out: &mut Fragments,
) -> Result<(), SourceError> {
    let body = check_response(request, response)?;
    let tide = match resource {
        DfoResource::TideHighLow => tide_events(request, body, ctx.now)?,
        DfoResource::TideCurve => tide_curve(request, body, ctx.now)?,
        DfoResource::WaterLevel => {
            let level = water_level(request, body)?;
            TideSeries {
                source_kind: TideSourceKind::HarmonicStation,
                datum: level.datum,
                events: Vec::new(),
                curve: None,
                provenance: level.series.provenance().clone(),
                observed: Some(level.series),
            }
        }
    };
    out.push(Measurement::Tide, FragmentValue::TideSeries(tide))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;

    const ID: &str = "5cebf1de3d0f4a073c4bb943";
    const DAY0: i64 = 1_794_096_000; // 2026-11-08 00:00 UTC
    const NOW: UnixTime = UnixTime(DAY0 + 10 * DAY);

    fn ctx() -> FetchContext {
        FetchContext::new(NOW, "underbite/test")
    }

    fn station() -> StationRef {
        StationRef {
            source: SOURCE_ID,
            id: ID.into(),
            name: "Vancouver".into(),
            kind: StationKind::TideHarmonic,
            latitude: 49.2863,
            longitude: -123.0997,
            bin: None,
            reference_id: None,
        }
    }

    fn target(from: i64, to: i64, datum: Option<VerticalDatum>) -> Target {
        Target {
            place: Place::Station {
                station: station(),
                distance_m: Some(1_200.0),
                relation: Relation::SameWaterBody,
            },
            measurements: BTreeSet::from([Measurement::Tide]),
            window: Window {
                from: UnixTime(from),
                to: UnixTime(to),
            },
            datum,
        }
    }

    fn map(_: DfoResource) -> Resource {
        Resource::Fake
    }

    fn request_for(datum: Option<VerticalDatum>) -> Request {
        Request::new(
            "https://example.invalid/".to_string(),
            Resource::Fake,
            target(DAY0, DAY0 + DAY, datum),
        )
    }

    fn parse(
        resource: DfoResource,
        dir: &str,
        name: &str,
        carry: Option<&str>,
    ) -> Result<Fragments, SourceError> {
        let body = fixture_bytes(dir, name);
        let mut request = request_for(Some(VerticalDatum::Mllw));
        if let Some(carry) = carry {
            request = request.with_carry(carry);
        }
        let mut out = Fragments::new();
        parse_with(
            &request,
            resource,
            &Response::new(200, &body),
            &ctx(),
            &mut out,
        )?;
        Ok(out)
    }

    fn tide(out: &Fragments) -> &TideSeries {
        match &out.fragments()[0].value {
            FragmentValue::TideSeries(t) => t,
            other => panic!("not a tide series: {other:?}"),
        }
    }

    fn decoded(url: &str) -> String {
        url.replace("%3A", ":")
    }

    #[test]
    fn urls_name_the_resolution_and_whole_second_times() {
        let w = Window {
            from: UnixTime(DAY0),
            to: UnixTime(DAY0 + DAY),
        };
        assert_eq!(
            decoded(&data_url(
                ID,
                Code::Predicted,
                w,
                Some(Resolution::FifteenMinutes)
            )),
            format!(
                "{BASE}/stations/{ID}/data?time-series-code=wlp&from=2026-11-08T00:00:00Z&to=2026-11-09T00:00:00Z&resolution=FIFTEEN_MINUTES"
            )
        );
        assert!(!data_url(ID, Code::HighLow, w, None).contains("resolution"));
        assert_eq!(stations_url(None), format!("{BASE}/stations"));
        assert_eq!(
            stations_url(Some(Region::Pacific)),
            format!("{BASE}/stations?chs-region-code=PAC")
        );
        assert_eq!(
            station_by_code_url("07735"),
            format!("{BASE}/stations?code=07735")
        );
        assert_eq!(metadata_url(ID), format!("{BASE}/stations/{ID}/metadata"));
        assert_eq!(format_time(UnixTime(DAY0 + 3_661)), "2026-11-08T01:01:01Z");
    }

    #[test]
    fn range_limits_follow_the_resolution() {
        assert_eq!(Resolution::OneMinute.max_days(), 7);
        assert_eq!(Resolution::ThreeMinutes.max_days(), 21);
        assert_eq!(Resolution::FiveMinutes.max_days(), 31);
        assert_eq!(Resolution::FifteenMinutes.max_days(), 31);
        assert_eq!(Resolution::SixtyMinutes.max_days(), 31);
    }

    #[test]
    fn a_long_window_splits_without_overlap_or_excess() {
        let w = Window {
            from: UnixTime(DAY0 + 5),
            to: UnixTime(DAY0 + 70 * DAY),
        };
        let parts = chunks(w, 31);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].from, w.from);
        assert_eq!(parts[2].to, w.to);
        for p in &parts {
            assert!(p.to.0 - p.from.0 < 31 * DAY);
        }
        for pair in parts.windows(2) {
            assert_eq!(pair[1].from.0, pair[0].to.0 + 1);
        }
        let one = chunks(
            Window {
                from: UnixTime(DAY0),
                to: UnixTime(DAY0),
            },
            7,
        );
        assert_eq!(one.len(), 1);
        let backwards = Window {
            from: UnixTime(DAY0 + 1),
            to: UnixTime(DAY0),
        };
        assert_eq!(chunks(backwards, 7).len(), 0);
    }

    #[test]
    fn plan_splits_predictions_and_clips_observations_to_now() {
        let t = target(DAY0, DAY0 + 40 * DAY, Some(VerticalDatum::Mllw));
        let requests = plan_with(&t, &Requirements::default(), &ctx(), None, map).expect("plan");
        let with = |needle: &str| -> Vec<&Request> {
            requests.iter().filter(|r| r.url.contains(needle)).collect()
        };
        let hilo = with("wlp-hilo");
        let curve = with("time-series-code=wlp&");
        let observed = with("time-series-code=wlo");
        assert_eq!(hilo.len(), 1);
        assert_eq!(curve.len(), 2);
        assert_eq!(observed.len(), 1);
        assert!(requests.iter().all(|r| r.metered));
        assert!(hilo[0].required && !curve[0].required && !observed[0].required);
        assert!(curve.iter().all(|r| r.url.contains("FIFTEEN_MINUTES")));
        assert!(decoded(&observed[0].url).contains("to=2026-11-18T00:00:00Z"));
        // A US datum from the settings is not passed on and not converted to.
        assert!(
            requests
                .iter()
                .all(|r| r.target.datum == Some(VerticalDatum::ChsChartDatum))
        );
        assert!(requests.iter().all(|r| r.carry().is_none()));
    }

    #[test]
    fn plan_has_no_observations_for_a_future_window() {
        let t = target(DAY0 + 20 * DAY, DAY0 + 21 * DAY, None);
        let requests = plan_with(&t, &Requirements::default(), &ctx(), None, map).expect("plan");
        assert_eq!(requests.len(), 2);
    }

    #[test]
    fn plan_igld85_needs_the_offset_and_carries_it() {
        let t = target(DAY0, DAY0 + DAY, Some(VerticalDatum::Igld85));
        let err = plan_with(&t, &Requirements::default(), &ctx(), None, map).unwrap_err();
        assert!(matches!(err, SourceError::Unsupported(_)));
        let requests =
            plan_with(&t, &Requirements::default(), &ctx(), Some(74.2), map).expect("plan");
        assert!(requests.iter().all(|r| r.carry() == Some("74.2")));
        assert!(
            requests
                .iter()
                .all(|r| r.target.datum == Some(VerticalDatum::Igld85))
        );
    }

    #[test]
    fn plan_refuses_other_places_sources_and_measurements() {
        let mut t = target(DAY0, DAY0 + DAY, None);
        t.measurements = BTreeSet::from([Measurement::Waves]);
        assert!(plan_with(&t, &Requirements::default(), &ctx(), None, map).is_err());
        let mut t = target(DAY0, DAY0 + DAY, None);
        if let Place::Station { station, .. } = &mut t.place {
            station.source = SourceId::COOPS;
        }
        assert!(plan_with(&t, &Requirements::default(), &ctx(), None, map).is_err());
    }

    #[test]
    fn hilo_fixture_is_classified_and_provisional() {
        let out = parse(
            DfoResource::TideHighLow,
            "tier2",
            "dfo-iwls-07735-wlp-hilo-2026-11-08.json",
            None,
        )
        .expect("parse");
        let t = tide(&out);
        assert_eq!(t.datum, VerticalDatum::ChsChartDatum);
        assert_eq!(t.provenance.kind, ValueKind::Predicted);
        assert_eq!(t.provenance.quality, Quality::Provisional);
        assert_eq!(t.provenance.source, SourceId::DFO_IWLS);
        assert_eq!(t.provenance.observed_at, NOW);
        assert!(t.events.len() > 4);
        assert_eq!(t.events[0].time, UnixTime(DAY0 + 6 * 3_600 + 41 * 60));
        assert_eq!(t.events[0].kind, TideKind::Low);
        assert_eq!(t.events[0].height_m, Some(0.915));
        assert_eq!(t.events[1].kind, TideKind::High);
        for pair in t.events.windows(2) {
            assert_ne!(pair[0].kind, pair[1].kind);
        }
        assert!(t.curve.is_some());
        insta::assert_json_snapshot!("hilo_events_head", &t.events[..4]);
    }

    #[test]
    fn the_scouts_hilo_recording_matches_its_values() {
        let out = parse(
            DfoResource::TideHighLow,
            "dfo-iwls",
            "wlp-hilo-07735-2026-11-18-to-20.json",
            None,
        )
        .expect("parse");
        let t = tide(&out);
        let kinds: Vec<_> = t.events.iter().take(4).map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [TideKind::Low, TideKind::High, TideKind::Low, TideKind::High]
        );
        assert_eq!(t.events[0].height_m, Some(2.785));
    }

    #[test]
    fn predicted_levels_are_a_curve_with_per_sample_quality() {
        let out = parse(
            DfoResource::TideCurve,
            "dfo-iwls",
            "wlp-07735-15min-2026-11-18.json",
            None,
        )
        .expect("parse");
        let t = tide(&out);
        let curve = t.curve.as_ref().expect("curve");
        assert_eq!(curve.samples().len(), 97);
        assert_eq!(curve.samples()[0].value, 3.439);
        assert_eq!(curve.samples()[0].quality, Some(Quality::Provisional));
        assert_eq!(t.events.len(), 0);
        assert_eq!(t.datum, VerticalDatum::ChsChartDatum);
    }

    #[test]
    fn observed_levels_stand_alone() {
        let out = parse(
            DfoResource::WaterLevel,
            "dfo-iwls",
            "wlo-07735-15min-2026-11-17-0800-0900.json",
            None,
        )
        .expect("parse");
        let t = tide(&out);
        let observed = t.observed.as_ref().expect("observed");
        assert_eq!(observed.samples().len(), 5);
        assert_eq!(observed.provenance().kind, ValueKind::Observed);
        let nine = parse_rfc3339("2026-11-17T09:00:00Z").expect("time");
        assert_eq!(observed.provenance().observed_at, nine);
        assert!(t.curve.is_none());
        assert_eq!(t.events.len(), 0);
    }

    #[test]
    fn a_great_lakes_station_adds_its_igld85_offset() {
        let name = "wlo-13320-toronto-60min-2026-11-17-0800-0900.json";
        let out = parse(DfoResource::WaterLevel, "dfo-iwls", name, Some("74.2")).expect("parse");
        let t = tide(&out);
        assert_eq!(t.datum, VerticalDatum::Igld85);
        let first = t.observed.as_ref().expect("observed").samples()[0].value;
        assert!((first - (0.445 + 74.2)).abs() < 1e-3, "{first}");
        let plain = parse(DfoResource::WaterLevel, "dfo-iwls", name, None).expect("parse");
        assert_eq!(tide(&plain).datum, VerticalDatum::ChsChartDatum);
    }

    #[test]
    fn quality_flags_map_to_the_snapshot_scale() {
        let row = |qc: &str, reviewed: bool| Row {
            event_date: String::new(),
            value: 0.0,
            qc: Some(qc.into()),
            reviewed,
        };
        assert_eq!(row_quality(&row("1", true)), Quality::Approved);
        assert_eq!(row_quality(&row("1", false)), Quality::Provisional);
        assert_eq!(row_quality(&row("2", true)), Quality::Provisional);
        assert_eq!(row_quality(&row("3", true)), Quality::Flagged);
        assert_eq!(
            worst([Quality::Approved, Quality::Provisional]),
            Quality::Provisional
        );
        assert_eq!(
            worst([Quality::Provisional, Quality::Flagged, Quality::Approved]),
            Quality::Flagged
        );
        assert_eq!(worst([Quality::Approved]), Quality::Approved);
    }

    #[test]
    fn a_questionable_row_flags_the_series() {
        let body = br#"[
            {"eventDate":"2026-11-08T00:00:00Z","qcFlagCode":"1","reviewed":true,"value":1.0},
            {"eventDate":"2026-11-08T00:15:00Z","qcFlagCode":"3","reviewed":true,"value":1.1}]"#;
        let t = tide_curve(&request_for(None), body, NOW).expect("parse");
        assert_eq!(t.provenance.quality, Quality::Flagged);
        let curve = t.curve.expect("curve");
        assert_eq!(curve.samples()[0].quality, Some(Quality::Approved));
        assert_eq!(curve.samples()[1].quality, Some(Quality::Flagged));
    }

    #[test]
    fn errors_are_the_services_own() {
        let request = request_for(None);
        let mut out = Fragments::new();
        let mut run = |response: &Response<'_>| {
            parse_with(&request, DfoResource::TideCurve, response, &ctx(), &mut out).unwrap_err()
        };
        let body = fixture_bytes("dfo-iwls", "error-400-range-7-days.json");
        let err = run(&Response::new(400, &body));
        assert!(
            matches!(err, SourceError::Status { status: 400, .. }),
            "{err:?}"
        );
        let mut limited = Response::new(429, b"");
        limited.retry_after = Some(30);
        assert_eq!(
            run(&limited),
            SourceError::RateLimited {
                retry_after_s: Some(30)
            }
        );
        assert!(matches!(
            run(&Response::new(200, b"[]")),
            SourceError::NoData(_)
        ));
        let bad_date = br#"[{"eventDate":"soon","value":1}]"#;
        assert!(matches!(
            run(&Response::new(200, bad_date)),
            SourceError::Parse(_)
        ));
        assert!(out.is_empty());
    }

    #[test]
    fn a_lone_extremum_is_no_data() {
        let body = br#"[{"eventDate":"2026-11-08T00:00:00Z","qcFlagCode":"1","value":1.0}]"#;
        assert!(matches!(
            tide_events(&request_for(None), body, NOW),
            Err(SourceError::NoData(_))
        ));
    }

    #[test]
    fn station_record_keeps_both_languages_and_the_hex_id() {
        let body = fixture_bytes("tier2", "dfo-iwls-stations-07735.json");
        let stations =
            parse_stations(&request_for(None), &Response::new(200, &body)).expect("stations");
        assert_eq!(stations.len(), 1);
        let s = &stations[0];
        assert_eq!(s.id, ID);
        assert_eq!(s.code, "07735");
        assert_eq!(s.region.as_deref(), Some("PAC"));
        assert!(s.offers_water_level() && s.predicts_tides());
        let wlp = s.series("wlp").expect("wlp");
        assert_eq!(wlp.name_en.as_deref(), Some("Water level predictions"));
        assert_eq!(wlp.name_fr.as_deref(), Some("Prédictions de niveaux d'eau"));
        let r = s.station_ref();
        assert_eq!((r.id.as_str(), r.kind), (ID, StationKind::TideHarmonic));
    }

    #[test]
    fn the_whole_list_parses_and_the_offer_filter_drops_discontinued_stations() {
        let body = fixture_bytes("dfo-iwls", "stations-all-2026-11-17.json");
        let stations =
            parse_stations(&request_for(None), &Response::new(200, &body)).expect("stations");
        assert_eq!(stations.len(), 1_576);
        let offered = stations.iter().filter(|s| s.offers_water_level()).count();
        assert!((50..=140).contains(&offered), "{offered}");
        let leaf = stations.iter().find(|s| s.code == "04315").expect("04315");
        assert_eq!(leaf.names(), ["Tasiujaq", "Leaf Basin", "Lac Aux Feuilles"]);
        assert!(!leaf.offers_water_level());
    }

    #[test]
    fn metadata_gives_the_igld85_offset_only_on_the_great_lakes() {
        let toronto = fixture_bytes("dfo-iwls", "metadata-13320-toronto.json");
        let m =
            parse_metadata(&request_for(None), &Response::new(200, &toronto)).expect("metadata");
        assert_eq!(m.igld85_offset_m(), Some(74.2));
        assert_eq!(m.tidal, Some(false));
        assert_eq!(m.water_body.as_deref(), Some("LAKE"));
        assert_eq!(m.time_zone.as_deref(), Some("Canada/Eastern"));
        let vancouver = fixture_bytes("dfo-iwls", "metadata-07735.json");
        let m =
            parse_metadata(&request_for(None), &Response::new(200, &vancouver)).expect("metadata");
        assert_eq!(m.igld85_offset_m(), None);
        assert_eq!(m.datums.get("CGVD28"), Some(&-3.0));
    }

    #[test]
    fn attribution_carries_the_licence_notice() {
        let a = attribution();
        assert_eq!(a.source, SourceId::DFO_IWLS);
        assert!(
            a.credit
                .starts_with("Canadian Hydrographic Service, Fisheries and Oceans Canada.")
        );
        assert!(a.credit.contains("not to be used for navigation"));
        assert!(a.credit.contains("His Majesty the King in Right of Canada"));
        assert_eq!(budget().period, Period::Hour);
    }
}
