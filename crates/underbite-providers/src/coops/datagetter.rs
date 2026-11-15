//! CO-OPS data API: water levels, tide and current predictions, observations.
//!
//! The URL builder and one parser per product. Every parser expects a body that already passed
//! [`check_response`](crate::source::check_response); heights, speeds and temperatures come out
//! in canonical units, times in UTC, each series with its provenance.

use std::time::Duration;

use serde::Deserialize;
use underbite_core::datum::{Heights, VerticalDatum};
use underbite_core::model::{
    CurrentEvent, CurrentKind, CurrentSeries, TideEvent, TideKind, TideSeries, TideSourceKind, Wind,
};
use underbite_core::provenance::{
    Provenance, Quality, Relation, SourceId, StationKind, StationRef, ValueKind,
};
use underbite_core::series::{Sample, Series, Vector};
use underbite_core::time::{UnixTime, parse_coops};
use underbite_core::units::{Canonical, to_canonical};
use whether_core::time::civil_from_days;

use crate::request::{Place, QueryBuilder, Request, Window};
use crate::source::SourceError;

/// The data API endpoint.
pub const BASE: &str = "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter";

/// Most days of `interval=hilo` predictions per request (the service allows a year).
pub const MAX_HILO_DAYS: i64 = 365;
/// Most days of `max_slack` current predictions per request (the service stops at 366).
pub const MAX_SLACK_DAYS: i64 = 365;
/// Most days per request for interval predictions and observations (water level allows 31).
pub const MAX_INTERVAL_DAYS: i64 = 31;

const DAY: i64 = 86_400;
const CURVE_STEP: Duration = Duration::from_mins(6);

/// One data API request.
#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    /// `predictions`, `water_level`, `currents_predictions`, …
    pub product: &'a str,
    /// The station id as CO-OPS lists it.
    pub station: &'a str,
    /// The days wanted; both end dates are inclusive and cover whole UTC days.
    pub window: Window,
    /// The datum name, for products that need one.
    pub datum: Option<&'a str>,
    /// `hilo`, `6`, `max_slack`.
    pub interval: Option<&'a str>,
    /// The current meter bin.
    pub bin: Option<u16>,
}

/// The URL of `query`, with the four parameters the service insists on.
pub fn url(query: &Query<'_>) -> String {
    let date = |t: UnixTime| {
        let (y, m, d) = civil_from_days(t.0.div_euclid(DAY));
        format!("{y:04}{m:02}{d:02}")
    };
    let mut url = QueryBuilder::new(BASE)
        .param("product", query.product)
        .param("application", "underbite")
        .param("begin_date", date(query.window.from))
        .param("end_date", date(query.window.to));
    if let Some(datum) = query.datum {
        url = url.param("datum", datum);
    }
    url = url
        .param("station", query.station)
        .param("time_zone", "gmt")
        .param("units", "metric");
    if let Some(interval) = query.interval {
        url = url.param("interval", interval);
    }
    url = url.param("format", "json");
    if let Some(bin) = query.bin {
        url = url.param("bin", bin.to_string());
    }
    url.finish()
}

/// `window` cut into pieces of at most `max_days` whole UTC days, each clipped to the window.
pub fn day_chunks(window: Window, max_days: i64) -> Vec<Window> {
    let last = window.to.0.div_euclid(DAY);
    let mut chunks = Vec::new();
    let mut day = window.from.0.div_euclid(DAY);
    while day <= last {
        let end = (day + max_days.max(1) - 1).min(last);
        chunks.push(Window {
            from: UnixTime(window.from.0.max(day * DAY)),
            to: UnixTime(window.to.0.min((end + 1) * DAY - 1)),
        });
        day = end + 1;
    }
    chunks
}

/// A number the service writes as a JSON number or as a string; empty text is a gap.
#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    Num(f64),
    Text(String),
}

impl Number {
    fn value(&self) -> Result<Option<f32>, SourceError> {
        match self {
            Self::Num(v) => Ok(Some(*v as f32)),
            Self::Text(s) if s.trim().is_empty() => Ok(None),
            Self::Text(s) => s
                .trim()
                .parse::<f32>()
                .map(Some)
                .map_err(|_| parse_error(format!("not a number: {s:?}"))),
        }
    }
}

fn value(field: Option<&Number>) -> Result<Option<f32>, SourceError> {
    field.map_or(Ok(None), Number::value)
}

#[derive(Deserialize)]
struct Predictions {
    predictions: Vec<Prediction>,
}

#[derive(Deserialize)]
struct Prediction {
    t: String,
    v: Option<Number>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Observations {
    data: Vec<Observation>,
}

/// A row of any observed product: `v` for levels, temperature and pressure; `s` and `d` for wind
/// and currents; `g` and `q` where the product has them.
#[derive(Deserialize)]
struct Observation {
    t: String,
    v: Option<Number>,
    s: Option<Number>,
    d: Option<Number>,
    g: Option<Number>,
    q: Option<String>,
}

#[derive(Deserialize)]
struct CurrentPredictions {
    current_predictions: CurrentBody,
}

#[derive(Deserialize)]
struct CurrentBody {
    units: Option<String>,
    cp: Vec<CurrentRow>,
}

#[derive(Deserialize)]
struct CurrentRow {
    #[serde(rename = "Type")]
    kind: Option<String>,
    #[serde(rename = "Time")]
    time: String,
    #[serde(rename = "Velocity_Major")]
    velocity: Option<Number>,
    #[serde(rename = "Speed")]
    speed: Option<Number>,
    #[serde(rename = "Direction")]
    direction: Option<Number>,
    #[serde(rename = "meanFloodDir")]
    flood_dir: Option<Number>,
    #[serde(rename = "meanEbbDir")]
    ebb_dir: Option<Number>,
    #[serde(rename = "Bin")]
    bin: Option<Number>,
    #[serde(rename = "Depth")]
    depth: Option<Number>,
}

/// What a current curve request carries to [`current_curve`] when the service answers with speed
/// and direction: the flood direction that tells the sign.
#[derive(Deserialize)]
struct FloodCarry {
    flood_dir_deg: f32,
}

fn parse_error(message: impl Into<String>) -> SourceError {
    SourceError::Parse(message.into())
}

fn json<'a, T: Deserialize<'a>>(body: &'a [u8]) -> Result<T, SourceError> {
    serde_json::from_slice(body).map_err(|e| parse_error(e.to_string()))
}

fn time(text: &str) -> Result<UnixTime, SourceError> {
    parse_coops(text).map_err(|e| parse_error(e.to_string()))
}

/// `v` in `unit` as a canonical length, speed, temperature or pressure.
fn canonical(v: f32, unit: &str) -> Result<f32, SourceError> {
    match to_canonical(v, unit).map_err(|e| parse_error(e.to_string()))? {
        Canonical::Length(x)
        | Canonical::Speed(x)
        | Canonical::Temperature(x)
        | Canonical::Pressure(x) => Ok(x),
        other => Err(parse_error(format!("unexpected quantity {other:?}"))),
    }
}

fn station(request: &Request) -> Result<(&StationRef, Option<f32>, Relation), SourceError> {
    match &request.target.place {
        Place::Station {
            station,
            distance_m,
            relation,
        } => Ok((station, *distance_m, *relation)),
        _ => Err(SourceError::Unsupported(
            "CO-OPS data needs a station".into(),
        )),
    }
}

fn provenance(
    request: &Request,
    kind: ValueKind,
    quality: Quality,
    observed_at: UnixTime,
) -> Result<Provenance, SourceError> {
    let (station, distance_m, relation) = station(request)?;
    Ok(Provenance {
        source: SourceId::COOPS,
        station: Some(station.clone()),
        distance_m,
        relation,
        observed_at,
        kind,
        quality,
    })
}

/// The datum the request asked heights in; the response does not say.
fn datum(request: &Request) -> VerticalDatum {
    request.target.datum.unwrap_or(VerticalDatum::Mllw)
}

fn predictions(body: &[u8]) -> Result<Vec<(UnixTime, f32, Option<String>)>, SourceError> {
    let rows: Predictions = json(body)?;
    let mut out = Vec::with_capacity(rows.predictions.len());
    for row in rows.predictions {
        let height = value(row.v.as_ref())?.ok_or_else(|| parse_error("prediction without v"))?;
        out.push((time(&row.t)?, canonical(height, "m")?, row.kind));
    }
    if out.is_empty() {
        return Err(SourceError::NoData(Some("no predictions".into())));
    }
    Ok(out)
}

/// `predictions` with `interval=hilo`: highs and lows. Heights are in the datum the target asked
/// for. The curve between them is interpolated (`Computed`, `Estimated`); a 6-minute request
/// supplies the real one at harmonic stations.
pub fn tide_events(
    request: &Request,
    body: &[u8],
    now: UnixTime,
) -> Result<TideSeries, SourceError> {
    let mut events = Vec::new();
    for (time, height, kind) in predictions(body)? {
        let kind = match kind.as_deref() {
            Some("H" | "HH") => TideKind::High,
            Some("L" | "LL") => TideKind::Low,
            other => return Err(parse_error(format!("unknown tide type {other:?}"))),
        };
        events.push(TideEvent {
            time,
            kind,
            height_m: Some(height),
        });
    }
    events.sort_by_key(|e| e.time);
    let provenance = provenance(request, ValueKind::Predicted, Quality::Approved, now)?;
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
        source_kind: tide_source_kind(request)?,
        datum: datum(request),
        events,
        curve,
        observed: None,
        provenance,
    })
}

fn tide_source_kind(request: &Request) -> Result<TideSourceKind, SourceError> {
    Ok(
        if station(request)?.0.kind == StationKind::TideSubordinate {
            TideSourceKind::SubordinateStation
        } else {
            TideSourceKind::HarmonicStation
        },
    )
}

/// `predictions` at 6 minutes: the curve of a harmonic station, in a series without events.
pub fn tide_curve(
    request: &Request,
    body: &[u8],
    now: UnixTime,
) -> Result<TideSeries, SourceError> {
    let samples = predictions(body)?
        .into_iter()
        .map(|(time, height, _)| Sample::new(time, height))
        .collect();
    let provenance = provenance(request, ValueKind::Predicted, Quality::Approved, now)?;
    Ok(TideSeries {
        source_kind: tide_source_kind(request)?,
        datum: datum(request),
        events: Vec::new(),
        curve: Some(Series::new(provenance.clone(), samples)),
        observed: None,
        provenance,
    })
}

/// Samples of an observed product. `sample` turns a row and its time into a value, or `None` for
/// a gap. The series is `Provisional` unless every sample says it is verified.
fn observed<T>(
    request: &Request,
    body: &[u8],
    mut sample: impl FnMut(&Observation, UnixTime) -> Result<Option<Sample<T>>, SourceError>,
) -> Result<Series<T>, SourceError> {
    let rows: Observations = json(body)?;
    let mut samples = Vec::with_capacity(rows.data.len());
    for row in &rows.data {
        if let Some(s) = sample(row, time(&row.t)?)? {
            samples.push(s);
        }
    }
    let Some(latest) = samples.iter().map(|s| s.time).max() else {
        return Err(SourceError::NoData(Some("no values".into())));
    };
    let verified = samples.iter().all(|s| s.quality == Some(Quality::Approved));
    let quality = if verified {
        Quality::Approved
    } else {
        Quality::Provisional
    };
    let provenance = provenance(request, ValueKind::Observed, quality, latest)?;
    Ok(Series::new(provenance, samples))
}

/// `water_level`: observed heights in the requested datum. `q` `p` is preliminary and `v`
/// verified; the flags in `f` are not gaps.
pub fn water_level(request: &Request, body: &[u8]) -> Result<Heights, SourceError> {
    let series = observed(request, body, |row, time| {
        let Some(v) = value(row.v.as_ref())? else {
            return Ok(None);
        };
        let quality = match row.q.as_deref() {
            Some("v") => Quality::Approved,
            _ => Quality::Provisional,
        };
        Ok(Some(Sample {
            time,
            value: canonical(v, "m")?,
            quality: Some(quality),
        }))
    })?;
    Ok(Heights {
        datum: datum(request),
        series,
    })
}

/// A scalar product (`water_temperature` in °C, `air_temperature`, `air_pressure` in hPa).
pub fn scalar(request: &Request, body: &[u8], unit: &str) -> Result<Series<f32>, SourceError> {
    observed(request, body, |row, time| {
        value(row.v.as_ref())?
            .map(|v| Ok(Sample::new(time, canonical(v, unit)?)))
            .transpose()
    })
}

/// `wind`: speed and gust in m/s, direction in degrees true (the "from" convention is assumed,
/// the documentation does not say).
pub fn wind(request: &Request, body: &[u8]) -> Result<Series<Wind>, SourceError> {
    observed(request, body, |row, time| {
        let Some(speed) = value(row.s.as_ref())? else {
            return Ok(None);
        };
        let gust = value(row.g.as_ref())?
            .map(|g| canonical(g, "m/s"))
            .transpose()?;
        Ok(Some(Sample::new(
            time,
            Wind {
                speed_ms: canonical(speed, "m/s")?,
                gust_ms: gust,
                from_deg: value(row.d.as_ref())?,
            },
        )))
    })
}

/// `currents` (PORTS meters): speed in m/s and direction in degrees true. Whether the direction
/// is the one the water flows toward is assumed, the documentation does not say.
pub fn observed_currents(request: &Request, body: &[u8]) -> Result<Series<Vector>, SourceError> {
    observed(request, body, |row, time| {
        let (Some(s), Some(d)) = (value(row.s.as_ref())?, value(row.d.as_ref())?) else {
            return Ok(None);
        };
        Ok(Some(Sample::new(
            time,
            Vector {
                speed_ms: canonical(s, "cm/s")?,
                toward_deg: d,
            },
        )))
    })
}

fn current_rows(body: &[u8]) -> Result<Vec<CurrentRow>, SourceError> {
    let body: CurrentPredictions = json(body)?;
    let body = body.current_predictions;
    if body.units.as_deref().is_some_and(|u| !u.contains("cm/s")) {
        return Err(parse_error(format!("unexpected units {:?}", body.units)));
    }
    if body.cp.is_empty() {
        return Err(SourceError::NoData(Some("no current predictions".into())));
    }
    Ok(body.cp)
}

fn first_of(
    rows: &[CurrentRow],
    field: impl Fn(&CurrentRow) -> Option<&Number>,
) -> Result<Option<f32>, SourceError> {
    for row in rows {
        if let Some(v) = value(field(row))? {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

fn current_series(
    request: &Request,
    rows: &[CurrentRow],
    now: UnixTime,
) -> Result<CurrentSeries, SourceError> {
    let bin = first_of(rows, |r| r.bin.as_ref())?
        .map(|b| u16::try_from(b.round() as i64))
        .transpose()
        .map_err(|_| parse_error("bin out of range"))?
        .or(station(request)?.0.bin);
    let depth_m = first_of(rows, |r| r.depth.as_ref())?
        .map(|d| canonical(d, "m"))
        .transpose()?;
    Ok(CurrentSeries {
        events: Vec::new(),
        curve: None,
        flood_dir_deg: first_of(rows, |r| r.flood_dir.as_ref())?,
        ebb_dir_deg: first_of(rows, |r| r.ebb_dir.as_ref())?,
        bin,
        depth_m,
        observed: None,
        provenance: provenance(request, ValueKind::Predicted, Quality::Approved, now)?,
    })
}

/// `currents_predictions` with `interval=max_slack`: slack and maximum flood and ebb, signed
/// along the major axis (positive toward flood), with the flood and ebb directions and the bin.
pub fn current_events(
    request: &Request,
    body: &[u8],
    now: UnixTime,
) -> Result<CurrentSeries, SourceError> {
    let rows = current_rows(body)?;
    let mut events = Vec::with_capacity(rows.len());
    for row in &rows {
        let kind = match row.kind.as_deref() {
            Some("flood") => CurrentKind::MaxFlood,
            Some("ebb") => CurrentKind::MaxEbb,
            Some("slack") => CurrentKind::Slack,
            other => return Err(parse_error(format!("unknown current type {other:?}"))),
        };
        let v = value(row.velocity.as_ref())?
            .ok_or_else(|| parse_error("current event without Velocity_Major"))?;
        events.push(CurrentEvent {
            time: time(&row.time)?,
            kind,
            speed_ms: canonical(v, "cm/s")?,
        });
    }
    events.sort_by_key(|e| e.time);
    Ok(CurrentSeries {
        events,
        ..current_series(request, &rows, now)?
    })
}

/// `currents_predictions` at 6 minutes: the signed major-axis curve. The shape is detected from
/// the field names: `Velocity_Major` is used as it is; `Speed` and `Direction` are signed by
/// whether the direction is within 90 degrees of the flood direction the request carries
/// (`{"flood_dir_deg":…}`), and fail without it. An answer with event types is the
/// `max_slack` response a subordinate station gives to any interval, and is refused.
pub fn current_curve(
    request: &Request,
    body: &[u8],
    now: UnixTime,
) -> Result<CurrentSeries, SourceError> {
    let rows = current_rows(body)?;
    let carried = request
        .carry()
        .map(|c| serde_json::from_str::<FloodCarry>(c).map(|c| c.flood_dir_deg))
        .transpose()
        .map_err(|e| parse_error(e.to_string()))?;
    let mut samples = Vec::with_capacity(rows.len());
    for row in &rows {
        if row.kind.is_some() {
            return Err(parse_error("events in a current curve response"));
        }
        let v = if let Some(v) = value(row.velocity.as_ref())? {
            v
        } else if let (Some(speed), Some(dir)) =
            (value(row.speed.as_ref())?, value(row.direction.as_ref())?)
        {
            let flood = carried
                .ok_or_else(|| parse_error("speed and direction without a flood direction"))?;
            let apart = (dir - flood + 540.0).rem_euclid(360.0) - 180.0;
            if apart.abs() <= 90.0 { speed } else { -speed }
        } else {
            return Err(parse_error(
                "current row with neither Velocity_Major nor Speed",
            ));
        };
        samples.push(Sample::new(time(&row.time)?, canonical(v, "cm/s")?));
    }
    let mut series = current_series(request, &rows, now)?;
    series.flood_dir_deg = series.flood_dir_deg.or(carried);
    series.curve = Some(Series::new(series.provenance.clone(), samples));
    Ok(series)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::BTreeSet;

    use underbite_core::model::Measurement;

    use super::*;
    use crate::request::{Resource, Target};

    pub(crate) const NOW: UnixTime = UnixTime(1_794_000_000);

    pub(crate) fn station_ref(id: &str, kind: StationKind, bin: Option<u16>) -> StationRef {
        StationRef {
            source: SourceId::COOPS,
            id: id.into(),
            name: format!("Station {id}"),
            kind,
            latitude: 40.7,
            longitude: -74.0,
            bin,
            reference_id: None,
        }
    }

    pub(crate) fn target(
        station: StationRef,
        measurement: Measurement,
        from: i64,
        to: i64,
    ) -> Target {
        Target {
            place: Place::Station {
                station,
                distance_m: Some(640.0),
                relation: Relation::SameWaterBody,
            },
            measurements: BTreeSet::from([measurement]),
            window: Window {
                from: UnixTime(from),
                to: UnixTime(to),
            },
            datum: Some(VerticalDatum::Mllw),
        }
    }

    /// A request for the week of 2026-11-08 at `station`.
    pub(crate) fn request(
        resource: Resource,
        station: StationRef,
        measurement: Measurement,
    ) -> Request {
        let from = 1_794_096_000; // 2026-11-08 00:00 UTC
        Request::new(
            "https://example.test/".into(),
            resource,
            target(station, measurement, from, from + 7 * DAY - 1),
        )
    }
}

#[cfg(test)]
mod tests {
    use underbite_core::model::Measurement;

    use super::testing::*;
    use super::*;
    use crate::request::Resource;
    use crate::request::Response;
    use crate::source::check_response;
    use crate::testutil::fixture_bytes;

    fn checked(name: &str) -> Vec<u8> {
        let bytes = fixture_bytes("coops", name);
        let request = tide_request(Resource::CoopsTideHighLow, "x", StationKind::TideHarmonic);
        check_response(&request, &Response::new(200, &bytes))
            .unwrap()
            .to_vec()
    }

    fn tide_request(resource: Resource, id: &str, kind: StationKind) -> Request {
        request(resource, station_ref(id, kind, None), Measurement::Tide)
    }

    fn current_request(resource: Resource, id: &str, kind: StationKind, bin: u16) -> Request {
        request(
            resource,
            station_ref(id, kind, Some(bin)),
            Measurement::TidalCurrent,
        )
    }

    #[test]
    fn url_has_the_required_parameters_and_whole_days() {
        let window = Window {
            from: UnixTime(1_794_096_000 + 3_600),
            to: UnixTime(1_794_096_000 + 7 * DAY - 1),
        };
        let query = Query {
            product: "predictions",
            station: "8518750",
            window,
            datum: Some("MLLW"),
            interval: Some("hilo"),
            bin: None,
        };
        assert_eq!(
            url(&query),
            "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions&application=underbite&begin_date=20261108&end_date=20261114&datum=MLLW&station=8518750&time_zone=gmt&units=metric&interval=hilo&format=json"
        );
        let currents = Query {
            product: "currents_predictions",
            station: "NYH1927",
            datum: None,
            interval: Some("max_slack"),
            bin: Some(13),
            ..query
        };
        assert!(url(&currents).ends_with("interval=max_slack&format=json&bin=13"));
        assert!(!url(&currents).contains("datum"));
    }

    #[test]
    fn day_chunks_split_and_clip() {
        let day0 = 1_794_096_000;
        let window = Window {
            from: UnixTime(day0 + 100),
            to: UnixTime(day0 + 70 * DAY + 5),
        };
        let chunks = day_chunks(window, 31);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].from, window.from);
        assert_eq!(chunks[0].to.0, day0 + 31 * DAY - 1);
        assert_eq!(chunks[1].from.0, day0 + 31 * DAY);
        assert_eq!(chunks[2].to, window.to);
        let backwards = Window {
            from: window.to,
            to: window.from,
        };
        assert_eq!(day_chunks(backwards, 31).len(), 0);
    }

    #[test]
    fn harmonic_hilo_parses_27_events() {
        let body = checked("coops-8518750-hilo-2026-11-08.json");
        let request = tide_request(
            Resource::CoopsTideHighLow,
            "8518750",
            StationKind::TideHarmonic,
        );
        let series = tide_events(&request, &body, NOW).unwrap();
        assert_eq!(series.events.len(), 27);
        assert_eq!(series.source_kind, TideSourceKind::HarmonicStation);
        assert_eq!(series.datum, VerticalDatum::Mllw);
        let first = series.events[0];
        assert_eq!(first.time, parse_coops("2026-11-08 05:59").unwrap());
        assert_eq!(first.kind, TideKind::Low);
        assert!((first.height_m.unwrap() - 0.017).abs() < 1e-6);
        let curve = series.curve.as_ref().unwrap();
        assert_eq!(curve.provenance().kind, ValueKind::Computed);
        assert_eq!(curve.provenance().quality, Quality::Estimated);
        assert_eq!(series.provenance.kind, ValueKind::Predicted);
        insta::assert_json_snapshot!("harmonic_hilo_events", series.events);
    }

    #[test]
    fn subordinate_hilo_is_marked_subordinate() {
        let body = checked("coops-8518919-hilo-2026-11-08.json");
        let request = tide_request(
            Resource::CoopsTideHighLow,
            "8518919",
            StationKind::TideSubordinate,
        );
        let series = tide_events(&request, &body, NOW).unwrap();
        assert_eq!(series.source_kind, TideSourceKind::SubordinateStation);
        assert_eq!(series.events[0].kind, TideKind::High);
        assert!(series.curve.is_some());
        let diurnal = checked("coops-8729840-hilo-2026-11-08.json");
        let pensacola = tide_events(
            &tide_request(
                Resource::CoopsTideHighLow,
                "8729840",
                StationKind::TideHarmonic,
            ),
            &diurnal,
            NOW,
        )
        .unwrap();
        assert_eq!(pensacola.events.len(), 14);
    }

    #[test]
    fn six_minute_curve_has_1680_samples() {
        let body = checked("coops-8518750-predictions-6min-2026-11-08.json");
        let request = tide_request(
            Resource::CoopsTideCurve,
            "8518750",
            StationKind::TideHarmonic,
        );
        let series = tide_curve(&request, &body, NOW).unwrap();
        let curve = series.curve.unwrap();
        assert_eq!(curve.samples().len(), 1680);
        assert_eq!(series.events.len(), 0);
        assert_eq!(curve.provenance().kind, ValueKind::Predicted);
        let head: Vec<_> = curve.samples().iter().take(3).collect();
        insta::assert_json_snapshot!("curve_head", head);
    }

    #[test]
    fn max_slack_events_keep_signs_and_directions() {
        let body = checked("coops-NYH1927-bin13-max-slack-2026-11-08.json");
        let request = current_request(
            Resource::CoopsCurrentEvents,
            "NYH1927",
            StationKind::CurrentHarmonic,
            13,
        );
        let series = current_events(&request, &body, NOW).unwrap();
        let day = parse_coops("2026-11-09 00:00").unwrap();
        let on_day: Vec<_> = series.events.iter().filter(|e| e.time < day).collect();
        assert_eq!(on_day.len(), 8);
        assert_eq!(series.flood_dir_deg, Some(11.0));
        assert_eq!(series.ebb_dir_deg, Some(183.0));
        assert_eq!(series.bin, Some(13));
        assert_eq!(series.depth_m, Some(2.1));
        let ebb = series
            .events
            .iter()
            .find(|e| e.time == parse_coops("2026-11-08 06:30").unwrap())
            .unwrap();
        assert_eq!(ebb.kind, CurrentKind::MaxEbb);
        assert!((ebb.speed_ms + 1.175).abs() < 1e-5);
        insta::assert_json_snapshot!("current_events_head", on_day);
    }

    #[test]
    fn subordinate_current_events_have_no_depth() {
        let body = checked("coops-ACT3646-bin1-max-slack-2026-11-08.json");
        let request = current_request(
            Resource::CoopsCurrentEvents,
            "ACT3646",
            StationKind::CurrentSubordinate,
            1,
        );
        let series = current_events(&request, &body, NOW).unwrap();
        assert_eq!(series.events.len(), 55);
        assert_eq!(series.depth_m, None);
        assert_eq!(series.bin, Some(1));
        assert_eq!(series.flood_dir_deg, Some(31.0));
        let reference = checked("coops-n03020-bin7-max-slack-2026-11-08.json");
        let request = current_request(
            Resource::CoopsCurrentEvents,
            "n03020",
            StationKind::CurrentHarmonic,
            7,
        );
        assert_eq!(
            current_events(&request, &reference, NOW)
                .unwrap()
                .events
                .len(),
            54
        );
    }

    #[test]
    fn current_curve_reads_velocity_major() {
        let body = checked("coops-NYH1927-bin13-currents-predictions-6min-2026-11-08.json");
        let request = current_request(
            Resource::CoopsCurrentCurve,
            "NYH1927",
            StationKind::CurrentHarmonic,
            13,
        );
        let series = current_curve(&request, &body, NOW).unwrap();
        let curve = series.curve.unwrap();
        assert_eq!(curve.samples().len(), 1680);
        assert!((curve.samples()[0].value - 0.599).abs() < 1e-5);
        assert_eq!(series.flood_dir_deg, Some(11.0));
        assert_eq!(series.events.len(), 0);
    }

    #[test]
    fn current_curve_detects_speed_and_direction() {
        let body = checked("coops-NYH1927-bin13-speed-dir-6min-2026-11-08.json");
        let plain = current_request(
            Resource::CoopsCurrentCurve,
            "NYH1927",
            StationKind::CurrentHarmonic,
            13,
        );
        assert!(matches!(
            current_curve(&plain, &body, NOW),
            Err(SourceError::Parse(m)) if m.contains("flood direction")
        ));
        let carried = plain.with_carry(r#"{"flood_dir_deg":11.0}"#);
        let series = current_curve(&carried, &body, NOW).unwrap();
        let curve = series.curve.unwrap();
        assert_eq!(curve.samples().len(), 240);
        assert!((curve.samples()[0].value - 0.60056).abs() < 1e-5);
        assert!(curve.samples().iter().any(|s| s.value < 0.0));
        assert_eq!(series.flood_dir_deg, Some(11.0));
    }

    #[test]
    fn a_curve_answered_with_events_is_refused() {
        let body = checked("coops-ACT3646-bin1-max-slack-2026-11-08.json");
        let request = current_request(
            Resource::CoopsCurrentCurve,
            "ACT3646",
            StationKind::CurrentSubordinate,
            1,
        );
        assert!(matches!(
            current_curve(&request, &body, NOW),
            Err(SourceError::Parse(m)) if m.contains("events")
        ));
    }

    #[test]
    fn water_level_is_provisional_with_the_datum_echoed() {
        let body = checked("coops-8518750-water-level-2026-11-07.json");
        let request = tide_request(
            Resource::CoopsWaterLevel,
            "8518750",
            StationKind::TideHarmonic,
        );
        let heights = water_level(&request, &body).unwrap();
        assert_eq!(heights.datum, VerticalDatum::Mllw);
        let series = heights.series;
        assert_eq!(series.samples().len(), 240);
        assert_eq!(series.provenance().quality, Quality::Provisional);
        assert_eq!(series.provenance().kind, ValueKind::Observed);
        assert_eq!(series.samples()[0].quality, Some(Quality::Provisional));
        assert!((series.samples()[0].value - 1.564).abs() < 1e-6);
        assert_eq!(
            series.provenance().observed_at,
            series.latest().unwrap().time
        );
        let latest = checked("coops-8518750-water-level-latest.json");
        assert_eq!(
            water_level(&request, &latest)
                .unwrap()
                .series
                .samples()
                .len(),
            1
        );
    }

    #[test]
    fn temperature_and_pressure_are_canonical() {
        let request = tide_request(
            Resource::CoopsWaterTemperature,
            "8518750",
            StationKind::TideHarmonic,
        );
        let temp = scalar(
            &request,
            &checked("coops-8518750-water-temperature-2026-11-07.json"),
            "degC",
        )
        .unwrap();
        assert_eq!(temp.samples()[0].value, 16.0);
        let air = scalar(
            &request,
            &checked("coops-8518750-air-temperature-2026-11-07.json"),
            "degC",
        )
        .unwrap();
        assert!(!air.is_empty());
        let pressure = scalar(
            &request,
            &checked("coops-8518750-air-pressure-2026-11-07.json"),
            "hPa",
        )
        .unwrap();
        assert_eq!(pressure.samples()[0].value, 1009.9);
        insta::assert_json_snapshot!(
            "pressure_head",
            pressure.samples().iter().take(2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn wind_is_in_metres_per_second() {
        let request = request(
            Resource::CoopsWind,
            station_ref("8531680", StationKind::WaterLevel, None),
            Measurement::ObservedWind,
        );
        let series = wind(&request, &checked("coops-8531680-wind-2026-11-07.json")).unwrap();
        let first = series.samples()[0].value;
        assert_eq!(first.speed_ms, 3.9);
        assert_eq!(first.gust_ms, Some(4.3));
        assert_eq!(first.from_deg, Some(308.0));
        insta::assert_json_snapshot!(
            "wind_head",
            series.samples().iter().take(2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn observed_currents_convert_to_metres_per_second() {
        let request = current_request(
            Resource::CoopsCurrents,
            "n06010",
            StationKind::PortsCurrentMeter,
            4,
        );
        let series =
            observed_currents(&request, &checked("coops-n06010-currents-2026-11-07.json")).unwrap();
        let first = series.samples()[0].value;
        assert!((first.speed_ms - 0.148).abs() < 1e-5);
        assert_eq!(first.toward_deg, 58.0);
    }

    #[test]
    fn error_fixtures_map_to_source_errors() {
        let request = tide_request(
            Resource::CoopsTideHighLow,
            "8518750",
            StationKind::TideHarmonic,
        );
        let run = |name: &str, status: u16| {
            let bytes = fixture_bytes("coops", name);
            check_response(&request, &Response::new(status, &bytes)).unwrap_err()
        };
        for name in [
            "coops-8518750-wind-2026-11-07-error-nodata.json",
            "coops-error-nodata-currents-n03020.json",
            "coops-error-subordinate-interval-6.json",
        ] {
            assert!(matches!(run(name, 200), SourceError::NoData(_)), "{name}");
        }
        for name in [
            "coops-error-no-datum.json",
            "coops-error-end-before-begin.json",
            "coops-error-range-limit-hilo.json",
            "coops-error-bad-bin.json",
            "coops-error-no-product.txt",
        ] {
            match run(name, 400) {
                SourceError::Status {
                    status: 400,
                    message: Some(m),
                } => {
                    assert!(!m.starts_with(' ') && !m.starts_with('\n'), "{name}: {m:?}");
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn gaps_are_skipped_and_garbage_is_an_error() {
        let request = tide_request(
            Resource::CoopsWaterTemperature,
            "8518750",
            StationKind::TideHarmonic,
        );
        let body =
            br#"{"data":[{"t":"2026-11-07 00:00","v":""},{"t":"2026-11-07 00:06","v":"16.5"}]}"#;
        let series = scalar(&request, body, "degC").unwrap();
        assert_eq!(series.samples().len(), 1);
        let all_gaps = br#"{"data":[{"t":"2026-11-07 00:00","v":""}]}"#;
        assert!(matches!(
            scalar(&request, all_gaps, "degC"),
            Err(SourceError::NoData(_))
        ));
        let garbage = br#"{"data":[{"t":"2026-11-07 00:00","v":"warm"}]}"#;
        assert!(matches!(
            scalar(&request, garbage, "degC"),
            Err(SourceError::Parse(_))
        ));
    }
}
