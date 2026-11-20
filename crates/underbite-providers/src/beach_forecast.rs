//! The National Weather Service marine beach forecast: rip current risk, surf, water
//! temperature, UV, weather and winds for the shore zones of the surf zone forecast.
//!
//! The data is an Esri `MapServer` with a group of two polygon layers per forecast office (Day 1
//! and Day 2), so a host asks in this order (sans-IO, for the map only):
//!
//! 1. [`layer_list_request`], answered by [`parse_layer_list`]: which layer holds each office's
//!    Day 1 and Day 2 zones. The ids change when offices are added, so the list is read at run
//!    time and kept for [`LAYER_LIST_MAX_AGE_S`].
//! 2. [`box_request`] for the zones that meet the visible area (with their outlines, to draw) or
//!    [`point_request`] for the zone at a point (without them), on an office's layer, answered by
//!    [`parse_zones`].
//!
//! Every field is text the forecaster wrote, or `null` when the office has issued no forecast
//! (off season, or no product yet): `null` is "no forecast issued", never "low", so the risk is an
//! `Option` and a zone with no forecast says so ([`BeachZone::has_forecast`]). The risk keeps the
//! text as issued ([`BeachZone::rip_text`]) for a word that is not one of the three categories.
//! A body that is HTTP 200 with an `error` object is an error.
//!
//! Coordinates leave the device only rounded to two decimals (a point), or as a box rounded outward
//! to two decimals and capped at [`MAX_BOX_SPAN_DEG`].

use std::collections::BTreeMap;

use serde_json::Value;
use underbite_core::map::{Bbox, GeoPoint};
use underbite_core::time::{LocalDate, UnixTime, day_number};
use whether_core::time::days_in_month;

use crate::access::padus::polygons;
use crate::request::{MAX_COORDINATE_DECIMALS, QueryBuilder, Response, RoundedPoint};

/// The service.
pub const BASE_URL: &str = "https://mapservices.weather.noaa.gov/vector/rest/services/outlooks/marine_beachforecast/MapServer";

/// How long the layer list may be reused, seconds: it changes when an office is added.
pub const LAYER_LIST_MAX_AGE_S: u32 = 7 * 86_400;

/// How long a forecast answer may be reused, seconds: offices issue it a few times a day.
pub const MAX_AGE_S: u32 = 3_600;

/// The widest box, degrees of latitude or longitude, a request may cover.
pub const MAX_BOX_SPAN_DEG: f64 = 1.0;

/// The most zones one answer holds (the service allows 2000; an office has a dozen at most).
pub const MAX_ZONES: u32 = 200;

/// The search distance of a point query, metres: the zones follow the shoreline, so a point a
/// little offshore or inland would miss.
pub const POINT_DISTANCE_M: u32 = 3_000;

/// How far the server may simplify an outline, degrees (about 110 m).
const SIMPLIFY_DEG: f64 = 0.001;

const FIELDS: &str = "id,beachname,siteid,rip,uv,surf,wtemp,weather,winds,period,productdat,producttim,idp_ingestdate";

/// The `NWS` rip current risk categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RipRisk {
    Low,
    Moderate,
    High,
}

impl RipRisk {
    /// The category a forecast word names, without regard to case; none for any other word.
    pub fn from_word(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "moderate" => Some(Self::Moderate),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// One GET for the host to send. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeachRequest {
    pub url: String,
}

/// A beach forecast request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BeachError {
    /// The request asks for something that cannot be sent.
    #[error("invalid beach forecast request: {0}")]
    InvalidRequest(&'static str),
    /// The service answered with an HTTP failure.
    #[error("the beach forecast service answered HTTP {status}")]
    Status { status: u16 },
    /// HTTP 200 with an error object.
    #[error("the beach forecast service reported an error ({code}): {message}")]
    Service { code: i64, message: String },
    /// The body is not the expected JSON.
    #[error("the beach forecast response is not the expected JSON: {0}")]
    Parse(String),
}

/// The layers of one forecast office.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfficeLayers {
    /// Today's forecast, 1200 UTC to 1200 UTC the next day.
    pub day1: u32,
    /// The following 24 hours, when the office has the layer.
    pub day2: Option<u32>,
}

/// The offices of the service and their layers, by the office's three-letter id (`OKX`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerList {
    pub offices: BTreeMap<String, OfficeLayers>,
}

impl LayerList {
    /// The layers of an office, by its id in any case.
    pub fn office(&self, id: &str) -> Option<OfficeLayers> {
        self.offices.get(&id.trim().to_ascii_uppercase()).copied()
    }
}

/// The request for the service's layer list.
pub fn layer_list_request() -> BeachRequest {
    BeachRequest {
        url: QueryBuilder::new(BASE_URL).param("f", "json").finish(),
    }
}

/// The office and day of a layer name: `OKX Day 1`, `LIX Day1`, `BOX Day 2`.
fn office_day(name: &str) -> Option<(String, u8)> {
    let (office, day) = name.trim().split_once(char::is_whitespace)?;
    let office = office.trim();
    if !(3..=4).contains(&office.len()) || !office.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let rest = day.trim();
    let digits = rest
        .get(..3)
        .filter(|d| d.eq_ignore_ascii_case("day"))
        .map(|_| rest[3..].trim())?;
    let day = match digits {
        "1" => 1,
        "2" => 2,
        _ => return None,
    };
    Some((office.to_ascii_uppercase(), day))
}

/// Reads the layer list: every polygon layer named for an office and a day. The last group, of
/// offices with no beach forecast, names its layers by office alone, so it contributes none.
///
/// # Errors
/// [`BeachError::Status`], [`BeachError::Service`] or [`BeachError::Parse`].
pub fn parse_layer_list(response: &Response<'_>) -> Result<LayerList, BeachError> {
    let v = body(response)?;
    let layers = v
        .get("layers")
        .and_then(Value::as_array)
        .ok_or_else(|| BeachError::Parse("no layers".to_owned()))?;
    let mut offices: BTreeMap<String, OfficeLayers> = BTreeMap::new();
    for l in layers {
        let (Some(id), Some(name)) = (
            l.get("id")
                .and_then(Value::as_u64)
                .and_then(|i| u32::try_from(i).ok()),
            l.get("name").and_then(Value::as_str),
        ) else {
            continue;
        };
        if l.get("type").and_then(Value::as_str) != Some("Feature Layer") {
            continue;
        }
        let Some((office, day)) = office_day(name) else {
            continue;
        };
        let entry = offices.entry(office).or_insert(OfficeLayers {
            day1: u32::MAX,
            day2: None,
        });
        if day == 1 {
            entry.day1 = id;
        } else {
            entry.day2 = Some(id);
        }
    }
    offices.retain(|_, o| o.day1 != u32::MAX);
    if offices.is_empty() {
        return Err(BeachError::Parse("no office layers".to_owned()));
    }
    Ok(LayerList { offices })
}

/// The zone at a point: a search of [`POINT_DISTANCE_M`] around the point rounded to two decimals
/// on `layer`, with or without the zone's outline.
pub fn point_request(point: RoundedPoint, layer: u32, geometry: bool) -> BeachRequest {
    let (lat, lon) = point.fmt_param();
    let q = QueryBuilder::new(&format!("{BASE_URL}/{layer}/query"))
        .param("f", "json")
        .param("geometry", format!("{lon},{lat}"))
        .param("geometryType", "esriGeometryPoint")
        .param("inSR", "4326")
        .param("spatialRel", "esriSpatialRelIntersects")
        .param("distance", POINT_DISTANCE_M.to_string())
        .param("units", "esriSRUnit_Meter")
        .param("outFields", FIELDS)
        .param("returnGeometry", if geometry { "true" } else { "false" });
    BeachRequest {
        url: with_outline(q, geometry).finish(),
    }
}

fn with_outline(q: QueryBuilder, geometry: bool) -> QueryBuilder {
    if geometry {
        q.param("outSR", "4326")
            .param("maxAllowableOffset", format!("{SIMPLIFY_DEG}"))
            .param("geometryPrecision", "4")
    } else {
        q
    }
}

/// The zones that meet a box on `layer`, with their outlines. The box edges are rounded outward to
/// two decimals.
///
/// # Errors
/// [`BeachError::InvalidRequest`] for a box that is not finite, outside the world, inverted or
/// wider than [`MAX_BOX_SPAN_DEG`].
pub fn box_request(bbox: Bbox, layer: u32) -> Result<BeachRequest, BeachError> {
    let edges = [bbox.west, bbox.south, bbox.east, bbox.north];
    if !edges.iter().all(|v| v.is_finite())
        || bbox.west > bbox.east
        || bbox.south > bbox.north
        || bbox.west < -180.0
        || bbox.east > 180.0
        || bbox.south < -90.0
        || bbox.north > 90.0
    {
        return Err(BeachError::InvalidRequest("box"));
    }
    if bbox.east - bbox.west > MAX_BOX_SPAN_DEG || bbox.north - bbox.south > MAX_BOX_SPAN_DEG {
        return Err(BeachError::InvalidRequest("box too wide"));
    }
    let scale = 10f64.powi(i32::from(MAX_COORDINATE_DECIMALS));
    // A value already on the grid stays put (`-74.4 * 100` is `-7440.000000000001`).
    let down = |v: f64| (v * scale + 1e-6).floor() / scale;
    let up = |v: f64| (v * scale - 1e-6).ceil() / scale;
    let q = QueryBuilder::new(&format!("{BASE_URL}/{layer}/query"))
        .param("f", "json")
        .param(
            "geometry",
            format!(
                "{:.2},{:.2},{:.2},{:.2}",
                down(bbox.west),
                down(bbox.south),
                up(bbox.east),
                up(bbox.north)
            ),
        )
        .param("geometryType", "esriGeometryEnvelope")
        .param("inSR", "4326")
        .param("spatialRel", "esriSpatialRelIntersects")
        .param("outFields", FIELDS)
        .param("returnGeometry", "true")
        .param("resultRecordCount", MAX_ZONES.to_string());
    Ok(BeachRequest {
        url: with_outline(q, true).finish(),
    })
}

/// The forecast office of a point, from an `api.weather.gov` `/points` answer: its three-letter
/// id, in upper case. None for a body without one.
///
/// # Errors
/// [`BeachError::Parse`] for a body that is not JSON.
pub fn office_of_point(body: &[u8]) -> Result<Option<String>, BeachError> {
    let v: Value = serde_json::from_slice(body).map_err(|e| BeachError::Parse(e.to_string()))?;
    Ok(v.pointer("/properties/cwa")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| (3..=4).contains(&s.len())))
}

/// One shore zone and what its office forecast for it.
#[derive(Clone, Debug, PartialEq)]
pub struct BeachZone {
    /// The public forecast zone, in lower case: `nyz178`.
    pub id: String,
    /// `Southern Queens Beaches`.
    pub name: String,
    /// The issuing office, in upper case: `OKX`.
    pub office: Option<String>,
    /// The rip current risk; none for no forecast or for a word that is no category.
    pub rip: Option<RipRisk>,
    /// The risk as the forecaster wrote it, whatever the word.
    pub rip_text: Option<String>,
    pub uv: Option<String>,
    pub surf: Option<String>,
    pub water_temperature: Option<String>,
    pub weather: Option<String>,
    pub winds: Option<String>,
    /// `Rest of today`, `Tonight`.
    pub period: Option<String>,
    /// The product's date as written: `11/20/2026`.
    pub product_date: Option<String>,
    /// The product's time as written: `409 PM EST`.
    pub product_time: Option<String>,
    /// When the forecast was issued: the product's date and time when they read, else the time
    /// the service took it in.
    pub issued: Option<UnixTime>,
    /// The outlines, as polygons of rings (the first the outer boundary); empty when none was
    /// asked for.
    pub polygons: Vec<Vec<Vec<GeoPoint>>>,
}

impl BeachZone {
    /// Whether the office issued anything for the zone.
    pub fn has_forecast(&self) -> bool {
        self.rip_text.is_some()
            || [
                &self.uv,
                &self.surf,
                &self.water_temperature,
                &self.weather,
                &self.winds,
            ]
            .iter()
            .any(|f| f.is_some())
    }
}

/// The zones an answer holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Zones {
    pub zones: Vec<BeachZone>,
    /// The answer was cut at the service's limit.
    pub more: bool,
}

fn body(response: &Response<'_>) -> Result<Value, BeachError> {
    if !(200..=299).contains(&response.status) {
        return Err(BeachError::Status {
            status: response.status,
        });
    }
    let v: Value =
        serde_json::from_slice(response.body).map_err(|e| BeachError::Parse(e.to_string()))?;
    if let Some(e) = v.get("error") {
        return Err(BeachError::Service {
            code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }
    Ok(v)
}

/// A text attribute; null, absent and blank are no value.
fn text(attrs: &Value, key: &str) -> Option<String> {
    attrs
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The offset of a US time zone abbreviation, seconds east of UTC.
fn zone_offset_s(abbreviation: &str) -> Option<i64> {
    let hours = match abbreviation.to_ascii_uppercase().as_str() {
        "EST" | "CDT" => -5,
        "EDT" | "AST" => -4,
        "CST" | "MDT" => -6,
        "MST" | "PDT" => -7,
        "PST" | "AKDT" => -8,
        "AKST" => -9,
        "HST" | "HAST" => -10,
        "SST" => -11,
        "CHST" => 10,
        _ => return None,
    };
    Some(hours * 3_600)
}

/// `11/20/2026` and `409 PM EST` (or `1022 AM CDT`) as an instant.
fn product_time(date: &str, time: &str) -> Option<UnixTime> {
    let mut d = date.trim().split('/');
    let month: u8 = d.next()?.parse().ok()?;
    let day: u8 = d.next()?.parse().ok()?;
    let year: i32 = d.next()?.parse().ok()?;
    if d.next().is_some()
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || !(2000..=2200).contains(&year)
    {
        return None;
    }
    let mut t = time.split_whitespace();
    let clock = t.next()?;
    let half = t.next()?.to_ascii_uppercase();
    let offset = zone_offset_s(t.next()?)?;
    if t.next().is_some() || !(3..=4).contains(&clock.len()) {
        return None;
    }
    let (h, m) = clock.split_at(clock.len() - 2);
    let hour: i64 = h.parse().ok()?;
    let minute: i64 = m.parse().ok()?;
    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    let hour24 = match half.as_str() {
        "AM" => hour % 12,
        "PM" => hour % 12 + 12,
        _ => return None,
    };
    let days = day_number(LocalDate { year, month, day });
    Some(UnixTime(
        days * 86_400 + hour24 * 3_600 + minute * 60 - offset,
    ))
}

fn zone(f: &Value) -> Option<BeachZone> {
    let attrs = f.get("attributes").or_else(|| f.get("properties"))?;
    let id = text(attrs, "id")?.to_ascii_lowercase();
    let rip_text = text(attrs, "rip");
    let product_date = text(attrs, "productdat");
    let product_time_text = text(attrs, "producttim");
    let ingested = attrs
        .get("idp_ingestdate")
        .and_then(Value::as_i64)
        .filter(|ms| *ms > 0)
        .map(|ms| UnixTime(ms / 1_000));
    let issued = match (&product_date, &product_time_text) {
        (Some(d), Some(t)) => product_time(d, t),
        _ => None,
    }
    .or(ingested);
    Some(BeachZone {
        name: text(attrs, "beachname").unwrap_or_else(|| id.to_ascii_uppercase()),
        office: text(attrs, "siteid").map(|s| s.to_ascii_uppercase()),
        rip: rip_text.as_deref().and_then(RipRisk::from_word),
        rip_text,
        uv: text(attrs, "uv"),
        surf: text(attrs, "surf"),
        water_temperature: text(attrs, "wtemp"),
        weather: text(attrs, "weather"),
        winds: text(attrs, "winds"),
        period: text(attrs, "period"),
        product_date,
        product_time: product_time_text,
        issued,
        polygons: polygons(f),
        id,
    })
}

/// Reads the zones of a point or box answer. A row with no zone id is left out.
///
/// # Errors
/// [`BeachError::Status`] for an HTTP failure, [`BeachError::Service`] for an error object in a
/// 200 body and [`BeachError::Parse`] for a body with no feature list.
pub fn parse_zones(response: &Response<'_>) -> Result<Zones, BeachError> {
    let v = body(response)?;
    let rows = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| BeachError::Parse("no features".to_owned()))?;
    let zones = rows.iter().filter_map(zone).collect();
    let more = v
        .get("exceededTransferLimit")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(Zones { zones, more })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Coordinates;
    use crate::testutil::fixture_bytes;

    fn coords(latitude: f64, longitude: f64) -> Coordinates {
        Coordinates {
            latitude,
            longitude,
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        fixture_bytes("swimming", name)
    }

    fn zones(name: &str) -> Zones {
        parse_zones(&Response::new(200, &fixture(name))).unwrap()
    }

    fn bbox(w: f64, s: f64, e: f64, n: f64) -> Bbox {
        Bbox {
            west: w,
            south: s,
            east: e,
            north: n,
        }
    }

    fn query(url: &str) -> Vec<String> {
        url.split_once('?')
            .unwrap()
            .1
            .split('&')
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_layer_list_maps_offices_to_their_day_layers() {
        let list = parse_layer_list(&Response::new(
            200,
            &fixture("mapservices-marine-beachforecast-mapserver.json"),
        ))
        .unwrap();
        assert_eq!(
            list.office("okx"),
            Some(OfficeLayers {
                day1: 91,
                day2: Some(92)
            })
        );
        assert_eq!(
            list.office("PHI").map(|o| (o.day1, o.day2)),
            Some((97, Some(98)))
        );
        assert_eq!(list.office("LOT").map(|o| o.day1), Some(16));
        assert_eq!(list.office("BOX").map(|o| o.day1), Some(1));
        // `LIX Day1` has no space; a layer with no day is none.
        assert_eq!(
            list.office("LIX").map(|o| (o.day1, o.day2)),
            Some((88, Some(89)))
        );
        // The bare names of the last group (layers 127 to 129) are not days.
        assert_eq!(list.office("TAE").map(|o| o.day1), Some(115));
        assert_eq!(list.office("AKQ").map(|o| o.day1), Some(121));
        assert!(list.offices.len() > 30, "{}", list.offices.len());
    }

    #[test]
    fn a_layer_list_without_offices_is_an_error() {
        let r = parse_layer_list(&Response::new(
            200,
            br#"{"layers":[{"id":0,"name":"x","type":"Group Layer"}]}"#,
        ));
        assert!(matches!(r, Err(BeachError::Parse(_))));
    }

    #[test]
    fn layer_names_are_read_in_their_spellings() {
        assert_eq!(office_day("OKX Day 1"), Some(("OKX".into(), 1)));
        assert_eq!(office_day("LIX Day1"), Some(("LIX".into(), 1)));
        assert_eq!(office_day("box day 2"), Some(("BOX".into(), 2)));
        assert_eq!(office_day("OKX Day 3"), None);
        assert_eq!(office_day("TAE"), None);
        assert_eq!(office_day("Boston-Norton_WFO"), None);
    }

    #[test]
    fn boston_has_values_as_issued() {
        let z = zones("mapservices-beachforecast-box-day1-sample.json");
        assert!(z.more);
        assert_eq!(z.zones.len(), 2);
        let a = &z.zones[0];
        assert_eq!(a.id, "maz016");
        assert_eq!(a.name, "Eastern Norfolk Area Beaches");
        assert_eq!(a.office.as_deref(), Some("BOX"));
        assert_eq!(a.rip, Some(RipRisk::Low));
        assert_eq!(a.rip_text.as_deref(), Some("Low"));
        assert_eq!(a.uv.as_deref(), Some("Low"));
        assert_eq!(a.surf.as_deref(), Some("Around 2 feet."));
        assert_eq!(a.water_temperature.as_deref(), Some("in the lower 50s"));
        assert_eq!(a.period.as_deref(), Some("Rest of today"));
        assert_eq!(a.product_date.as_deref(), Some("11/20/2026"));
        assert_eq!(a.product_time.as_deref(), Some("409 PM EST"));
        assert!(a.has_forecast());
        // 4:09 PM EST is 21:09 UTC; the service took it in at 21:37 UTC.
        assert_eq!(a.issued, Some(UnixTime(1_795_208_940)));
        assert_eq!(a.polygons.len(), 0);
    }

    #[test]
    fn no_forecast_is_null_never_low() {
        for (name, id, office) in [
            (
                "mapservices-beachforecast-point-rockaway-okx-day1.json",
                "nyz178",
                "OKX",
            ),
            (
                "mapservices-beachforecast-point-sandyhook-phi-day1.json",
                "njz014",
                "PHI",
            ),
            (
                "mapservices-beachforecast-point-chicago-lot-day1.json",
                "ilz104",
                "LOT",
            ),
        ] {
            let z = zones(name);
            assert!(!z.more, "{name}");
            assert_eq!(z.zones.len(), 1, "{name}");
            let zone = &z.zones[0];
            assert_eq!(zone.id, id);
            assert_eq!(zone.office.as_deref(), Some(office));
            assert_eq!(zone.rip, None, "{name}");
            assert_eq!(zone.rip_text, None, "{name}");
            assert_eq!(zone.surf, None);
            assert_eq!(zone.water_temperature, None);
            assert_eq!(zone.uv, None);
            assert!(!zone.has_forecast(), "{name}");
            // No product time: the ingest time stands.
            assert!(zone.issued.is_some(), "{name}");
        }
        let rockaway = &zones("mapservices-beachforecast-point-rockaway-okx-day1.json").zones[0];
        assert_eq!(rockaway.name, "Southern Queens Beaches");
        assert_eq!(rockaway.issued, Some(UnixTime(1_790_596_678)));
    }

    #[test]
    fn a_service_error_in_a_200_body_is_an_error() {
        let body = br#"{"error":{"code":400,"message":"Invalid or missing input parameters.","details":[]}}"#;
        assert_eq!(
            parse_zones(&Response::new(200, body)),
            Err(BeachError::Service {
                code: 400,
                message: "Invalid or missing input parameters.".to_owned()
            })
        );
        assert!(matches!(
            parse_layer_list(&Response::new(200, body)),
            Err(BeachError::Service { .. })
        ));
        assert_eq!(
            parse_zones(&Response::new(503, b"")),
            Err(BeachError::Status { status: 503 })
        );
        assert!(matches!(
            parse_zones(&Response::new(200, b"<html>")),
            Err(BeachError::Parse(_))
        ));
        assert!(matches!(
            parse_zones(&Response::new(200, b"{}")),
            Err(BeachError::Parse(_))
        ));
    }

    #[test]
    fn words_that_are_no_category_are_kept_as_issued_and_blank_is_none() {
        let body = br#"{"features":[{"attributes":{"id":"MAZ007","beachname":"B","siteid":"box","rip":"Extreme","uv":"  ","surf":"","wtemp":null,"idp_ingestdate":null}}]}"#;
        let z = parse_zones(&Response::new(200, body)).unwrap();
        let z = &z.zones[0];
        assert_eq!(z.id, "maz007");
        assert_eq!(z.rip, None);
        assert_eq!(z.rip_text.as_deref(), Some("Extreme"));
        assert_eq!(z.uv, None);
        assert_eq!(z.surf, None);
        assert!(z.has_forecast());
        assert_eq!(z.issued, None);
        for (word, risk) in [
            ("LOW", RipRisk::Low),
            (" moderate ", RipRisk::Moderate),
            ("High", RipRisk::High),
        ] {
            assert_eq!(RipRisk::from_word(word), Some(risk));
        }
        assert_eq!(RipRisk::from_word("Safe"), None);
        assert_eq!(RipRisk::from_word(""), None);
    }

    #[test]
    fn outlines_are_read_with_their_holes() {
        let z = zones("mapservices-beachforecast-box-rings.synthetic.json");
        assert_eq!(z.zones.len(), 3);
        let a = &z.zones[0];
        assert_eq!(a.rip, Some(RipRisk::Moderate));
        assert_eq!(a.polygons.len(), 1);
        assert_eq!(a.polygons[0].len(), 1);
        assert_eq!(a.polygons[0][0].len(), 5);
        assert_eq!(
            a.polygons[0][0][0],
            GeoPoint {
                lat: 40.55,
                lon: -73.9
            }
        );
        // Two parts, the second with a hole.
        let b = &z.zones[1];
        assert_eq!(b.rip, None);
        assert_eq!(b.polygons.len(), 2);
        assert_eq!(b.polygons[1].len(), 2);
        // No geometry: a zone with no outline.
        assert_eq!(z.zones[2].polygons.len(), 0);
        assert_eq!(z.zones[2].rip, Some(RipRisk::High));
    }

    #[test]
    fn product_times_are_read_in_the_offices_zone() {
        assert_eq!(
            product_time("11/20/2026", "409 PM EST"),
            Some(UnixTime(1_795_208_940))
        );
        // Midnight and noon.
        assert_eq!(
            product_time("7/4/2026", "1200 AM EDT"),
            Some(UnixTime(1_783_137_600))
        );
        let noon = product_time("7/4/2026", "1200 PM CDT").unwrap();
        let midnight = product_time("7/4/2026", "1200 AM CDT").unwrap();
        assert_eq!(noon.0 - midnight.0, 12 * 3_600);
        assert_eq!(
            noon.0 - product_time("7/4/2026", "1200 PM EDT").unwrap().0,
            3_600
        );
        for bad in [
            "",
            "409 PM",
            "409 XM EST",
            "1360 PM EST",
            "409 PM XYZ",
            "9 PM EST",
        ] {
            assert_eq!(product_time("11/20/2026", bad), None, "{bad}");
        }
        assert_eq!(product_time("13/20/2026", "409 PM EST"), None);
        assert_eq!(product_time("2/30/2026", "409 PM EST"), None);
    }

    #[test]
    fn the_layer_list_request_asks_for_json() {
        assert_eq!(layer_list_request().url, format!("{BASE_URL}?f=json"));
    }

    #[test]
    fn a_point_query_sends_two_decimals_and_a_search_distance() {
        let p = RoundedPoint::new(coords(40.583_412, -73.824_999), 2);
        let r = point_request(p, 91, false);
        assert!(
            r.url.starts_with(&format!("{BASE_URL}/91/query?")),
            "{}",
            r.url
        );
        let q = query(&r.url);
        for want in [
            "geometry=-73.82,40.58",
            "geometryType=esriGeometryPoint",
            "distance=3000",
            "units=esriSRUnit_Meter",
            "returnGeometry=false",
            "f=json",
        ] {
            assert!(q.iter().any(|p| p == want), "{want} in {q:?}");
        }
        assert!(!r.url.contains("40.5834"), "{}", r.url);
        assert!(!r.url.contains("73.8249"), "{}", r.url);
        assert!(!r.url.contains("maxAllowableOffset"));
        // A finer precision than the limit is rounded to it.
        let fine = RoundedPoint::new(coords(40.583_412, -73.824_999), 5);
        assert!(
            point_request(fine, 91, true)
                .url
                .contains("geometry=-73.82,40.58&")
        );
        let drawn = query(&point_request(p, 91, true).url);
        assert!(drawn.iter().any(|p| p == "returnGeometry=true"));
        assert!(drawn.iter().any(|p| p == "outSR=4326"));
    }

    #[test]
    fn a_box_query_is_rounded_outward_and_capped() {
        let r = box_request(bbox(-73.9049, 40.5401, -73.7651, 40.6299), 91).unwrap();
        let q = query(&r.url);
        for want in [
            "geometry=-73.91,40.54,-73.76,40.63",
            "geometryType=esriGeometryEnvelope",
            "inSR=4326",
            "outSR=4326",
            "returnGeometry=true",
            "resultRecordCount=200",
            "maxAllowableOffset=0.001",
        ] {
            assert!(q.iter().any(|p| p == want), "{want} in {q:?}");
        }
        // Not a digit of the exact edges leaves the phone.
        assert!(
            !r.url.contains("9049") && !r.url.contains("7651"),
            "{}",
            r.url
        );
        // An edge already on the grid stays.
        let on = box_request(bbox(-74.4, 40.0, -74.0, 40.4), 1).unwrap();
        assert!(
            on.url.contains("geometry=-74.40,40.00,-74.00,40.40"),
            "{}",
            on.url
        );
        assert!(box_request(bbox(1.0, 0.0, 0.0, 1.0), 1).is_err());
        assert!(box_request(bbox(f64::NAN, 0.0, 1.0, 1.0), 1).is_err());
        assert!(box_request(bbox(0.0, 0.0, 1.0, 91.0), 1).is_err());
        assert!(box_request(bbox(0.0, 0.0, 1.5, 0.5), 1).is_err());
        assert!(box_request(bbox(0.0, 0.0, 1.0, 1.0), 1).is_ok());
    }

    #[test]
    fn the_office_of_a_point_is_its_forecast_office() {
        let body = fixture_bytes("nws-marine", "points-40.7033_-74.0170.geo.json");
        assert_eq!(office_of_point(&body), Ok(Some("OKX".to_owned())));
        assert_eq!(office_of_point(b"{}"), Ok(None));
        assert!(office_of_point(b"nope").is_err());
    }
}
