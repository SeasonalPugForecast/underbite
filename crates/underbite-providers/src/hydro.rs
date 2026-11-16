//! USGS 3D Hydrography Program: flowlines and waterbodies near a spot.
//!
//! The 3DHP `MapServer` is an `ArcGIS` REST service (layer 50 Flowline, layer 60 Waterbody). This
//! module plans `query` requests and parses their Esri JSON into the geometry types of
//! `underbite_core::discover::hydro`. Those types are not snapshot fragments, so the module is
//! not a [`Source`](crate::source::Source): the host runs the [`HydroQuery`] values it gets from
//! [`flowlines_query`] and [`waterbodies_query`] and hands each response to [`parse_flowlines`]
//! or [`parse_waterbodies`]; [`next_page`] asks for the following page of a large answer.
//!
//! Location privacy: the request is centred on the spot rounded by the context
//! ([`FetchContext::round`]), and the search reaches at least [`MIN_RADIUS_M`] around that
//! centre, which covers the spot wherever it lies inside the rounding cell. The caller snaps the
//! exact spot to the returned geometry on the device, so the precise position never leaves it.
//!
//! The service answers errors under HTTP 200 with an `error` object, and silently ignores a
//! `geometry` it cannot parse, returning up to 2500 features; [`envelope_from_url`] reads the
//! geometry back so tests can check the string the service will see.

use serde::Deserialize;
use serde_json::Value;
use underbite_core::discover::hydro::{Flowline, FlowlineKind, Point, Waterbody, WaterbodyKind};
use underbite_core::provenance::{Attribution, SourceId};

use crate::request::{Coordinates, Response};
use crate::source::{FetchContext, SourceError};

/// The source id of 3DHP data.
pub const SOURCE: SourceId = SourceId::from_static("usgs-3dhp");

/// Query endpoint base.
pub const BASE_URL: &str = "https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer";

/// Smallest search radius around the rounded request centre, metres.
pub const MIN_RADIUS_M: f64 = 1_500.0;

/// Features per page; the service's `maxRecordCount`.
pub const PAGE_SIZE: u32 = 2_500;

/// Most pages one query follows.
pub const MAX_PAGES: u32 = 4;

/// Generalisation of waterbody outlines, degrees (a 901 KB reservoir becomes 5 KB).
pub const WATERBODY_GENERALISATION_DEG: f64 = 0.0005;

/// How long the host may reuse a response, seconds: the network changes rarely.
pub const MAX_AGE_S: u32 = 30 * 24 * 3600;

const METRES_PER_DEGREE: f64 = 111_320.0;

const FLOWLINE_FIELDS: &str = "id3dhp,gnisidlabel,featuretypelabel,streamorder,levelpath,\
hydrosequence,dnhydrosequence,uphydrosequence,lengthkm";
const WATERBODY_FIELDS: &str = "id3dhp,gnisid,gnisidlabel,featuretypelabel,areasqkm";

/// The credit every display of this data shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SOURCE,
        credit: "USGS TNM / NGTOC - 3D National Hydrographic Program (3DHP). Features may not \
                 reflect current conditions and are not for critical use."
            .to_string(),
        url: Some("https://www.usgs.gov/3d-hydrography-program".to_string()),
    }
}

/// A 3DHP layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Layer 50, polylines.
    Flowline,
    /// Layer 60, polygons.
    Waterbody,
}

impl Layer {
    /// The layer number in the service.
    pub fn number(self) -> u32 {
        match self {
            Self::Flowline => 50,
            Self::Waterbody => 60,
        }
    }
}

/// One planned GET of the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HydroQuery {
    /// The full URL.
    pub url: String,
    /// The layer queried.
    pub layer: Layer,
    /// Rows skipped before this page.
    pub offset: u32,
    /// How long the host may reuse the response, seconds.
    pub max_age_s: u32,
}

/// Flowlines in a box around the spot, with geometry, ordered by `hydrosequence`.
///
/// The box is centred on the context-rounded spot and reaches `radius_m` (at least
/// [`MIN_RADIUS_M`]) in every direction.
pub fn flowlines_query(
    spot: Coordinates,
    radius_m: f64,
    ctx: &FetchContext,
) -> Result<HydroQuery, SourceError> {
    let centre = ctx.round(spot).coordinates();
    let (xmin, ymin, xmax, ymax) = envelope(centre, radius_m)?;
    let url = format!(
        "{BASE_URL}/50/query?where=1=1&geometry={xmin:.5},{ymin:.5},{xmax:.5},{ymax:.5}\
&geometryType=esriGeometryEnvelope&inSR=4326&outSR=4326&spatialRel=esriSpatialRelIntersects\
&outFields={FLOWLINE_FIELDS}&returnGeometry=true&geometryPrecision=5\
&orderByFields=hydrosequence%20ASC&resultRecordCount={PAGE_SIZE}&f=json"
    );
    Ok(query(url, Layer::Flowline, 0))
}

/// Waterbodies within `radius_m` (at least [`MIN_RADIUS_M`]) of the rounded spot, with outlines
/// generalised by [`WATERBODY_GENERALISATION_DEG`], ordered by `id3dhp`.
pub fn waterbodies_query(
    spot: Coordinates,
    radius_m: f64,
    ctx: &FetchContext,
) -> Result<HydroQuery, SourceError> {
    let centre = ctx.round(spot).coordinates();
    envelope(centre, radius_m)?;
    let distance = radius_m.max(MIN_RADIUS_M).round();
    let url = format!(
        "{BASE_URL}/60/query?where=1=1&geometry={:.5},{:.5}&geometryType=esriGeometryPoint\
&inSR=4326&outSR=4326&spatialRel=esriSpatialRelIntersects&distance={distance}\
&units=esriSRUnit_Meter&outFields={WATERBODY_FIELDS}&returnGeometry=true&geometryPrecision=5\
&maxAllowableOffset={WATERBODY_GENERALISATION_DEG}&orderByFields=id3dhp%20ASC\
&resultRecordCount={PAGE_SIZE}&f=json",
        centre.longitude, centre.latitude
    );
    Ok(query(url, Layer::Waterbody, 0))
}

fn query(url: String, layer: Layer, offset: u32) -> HydroQuery {
    HydroQuery {
        url,
        layer,
        offset,
        max_age_s: MAX_AGE_S,
    }
}

/// The envelope `(xmin, ymin, xmax, ymax)` around `centre`, validated.
fn envelope(centre: Coordinates, radius_m: f64) -> Result<(f64, f64, f64, f64), SourceError> {
    let (lat, lon) = (centre.latitude, centre.longitude);
    if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
        return Err(SourceError::Unsupported(format!(
            "coordinates out of range: {lat}, {lon}"
        )));
    }
    let radius = if radius_m.is_finite() {
        radius_m.max(MIN_RADIUS_M)
    } else {
        MIN_RADIUS_M
    };
    let dlat = radius / METRES_PER_DEGREE;
    let dlon = dlat / lat.clamp(-85.0, 85.0).to_radians().cos();
    Ok((
        (lon - dlon).max(-180.0),
        (lat - dlat).max(-90.0),
        (lon + dlon).min(180.0),
        (lat + dlat).min(90.0),
    ))
}

/// The `geometry` of a query URL as `[xmin, ymin, xmax, ymax]`, if it parses as four ordered
/// numbers inside the WGS84 range (what the service needs, or it ignores the filter).
pub fn envelope_from_url(url: &str) -> Option<[f64; 4]> {
    let value = url
        .split_once('?')?
        .1
        .split('&')
        .find_map(|p| p.strip_prefix("geometry="))?;
    let numbers: Vec<f64> = value
        .split(',')
        .map(|n| n.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()?;
    let [xmin, ymin, xmax, ymax] = <[f64; 4]>::try_from(numbers).ok()?;
    let ordered = xmin < xmax && ymin < ymax;
    let in_range = xmin >= -180.0 && xmax <= 180.0 && ymin >= -90.0 && ymax <= 90.0;
    (ordered && in_range).then_some([xmin, ymin, xmax, ymax])
}

/// The query for the page after `current`, when `response` says rows remain and the page cap
/// allows another.
pub fn next_page(
    current: &HydroQuery,
    response: &Response<'_>,
) -> Result<Option<HydroQuery>, SourceError> {
    let page = read(response)?;
    if page.exceeded != Some(true) || page.features.is_empty() {
        return Ok(None);
    }
    let offset = current.offset.saturating_add(PAGE_SIZE);
    if offset >= PAGE_SIZE * MAX_PAGES {
        return Ok(None);
    }
    let mut url = current.url.clone();
    if let Some(at) = url.find("&resultOffset=") {
        let end = url[at + 1..].find('&').map_or(url.len(), |e| at + 1 + e);
        url.replace_range(at..end, "");
    }
    url.push_str("&resultOffset=");
    url.push_str(&offset.to_string());
    Ok(Some(query(url, current.layer, offset)))
}

/// Parses a layer 50 answer.
pub fn parse_flowlines(response: &Response<'_>) -> Result<Vec<Flowline>, SourceError> {
    read(response)?.features.iter().map(flowline).collect()
}

/// Parses a layer 60 answer.
pub fn parse_waterbodies(response: &Response<'_>) -> Result<Vec<Waterbody>, SourceError> {
    read(response)?.features.iter().map(waterbody).collect()
}

#[derive(Deserialize)]
struct RawPage {
    error: Option<RawError>,
    #[serde(default)]
    features: Vec<RawFeature>,
    #[serde(rename = "exceededTransferLimit")]
    exceeded: Option<bool>,
}

#[derive(Deserialize)]
struct RawError {
    code: Option<u16>,
    message: Option<String>,
    #[serde(default)]
    details: Vec<String>,
}

#[derive(Deserialize)]
struct RawFeature {
    #[serde(default)]
    attributes: Attributes,
    geometry: Option<Value>,
}

type Attributes = serde_json::Map<String, Value>;

/// Status and error-object checks, then the features.
fn read(response: &Response<'_>) -> Result<RawPage, SourceError> {
    let body = response.body.trim_ascii();
    match response.status {
        200..=299 => {}
        429 => {
            return Err(SourceError::RateLimited {
                retry_after_s: response.retry_after,
            });
        }
        404 | 410 => return Err(SourceError::NotFound(None)),
        status => {
            return Err(SourceError::Status {
                status,
                message: None,
            });
        }
    }
    if body.is_empty() {
        return Err(SourceError::NoData(Some("empty response".to_string())));
    }
    let mut raw: RawPage = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("invalid 3DHP JSON: {e}")))?;
    if let Some(error) = raw.error.take() {
        let mut message = error.message.unwrap_or_default();
        for detail in &error.details {
            message.push_str(": ");
            message.push_str(detail);
        }
        return Err(SourceError::Status {
            status: error.code.unwrap_or(response.status),
            message: Some(message).filter(|m| !m.is_empty()),
        });
    }
    Ok(raw)
}

fn text(a: &Attributes, key: &str) -> Option<String> {
    a.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn number(a: &Attributes, key: &str) -> Option<f64> {
    a.get(key)?.as_f64().filter(|v| v.is_finite())
}

/// A whole number the service stores as a double or an integer.
fn whole(a: &Attributes, key: &str) -> Option<u64> {
    let v = number(a, key)?;
    // Doubles hold integers exactly up to 2^53.
    (0.0..=9.0e15).contains(&v).then(|| v.round() as u64)
}

fn required_whole(a: &Attributes, key: &str) -> Result<u64, SourceError> {
    whole(a, key).ok_or_else(|| SourceError::Parse(format!("missing or invalid {key}")))
}

fn flowline(f: &RawFeature) -> Result<Flowline, SourceError> {
    let a = &f.attributes;
    let id =
        text(a, "id3dhp").ok_or_else(|| SourceError::Parse("flowline without id3dhp".into()))?;
    Ok(Flowline {
        id,
        name: text(a, "gnisidlabel"),
        kind: FlowlineKind::from_label(text(a, "featuretypelabel").as_deref().unwrap_or("")),
        stream_order: whole(a, "streamorder").and_then(|o| u8::try_from(o).ok()),
        level_path: required_whole(a, "levelpath")?,
        hydrosequence: required_whole(a, "hydrosequence")?,
        dn_hydrosequence: whole(a, "dnhydrosequence").unwrap_or(0),
        up_hydrosequence: whole(a, "uphydrosequence").unwrap_or(0),
        length_km: number(a, "lengthkm"),
        paths: parts(f.geometry.as_ref(), "paths")?,
    })
}

fn waterbody(f: &RawFeature) -> Result<Waterbody, SourceError> {
    let a = &f.attributes;
    let id =
        text(a, "id3dhp").ok_or_else(|| SourceError::Parse("waterbody without id3dhp".into()))?;
    Ok(Waterbody {
        id,
        gnis_id: whole(a, "gnisid"),
        name: text(a, "gnisidlabel"),
        kind: WaterbodyKind::from_label(text(a, "featuretypelabel").as_deref().unwrap_or("")),
        area_km2: number(a, "areasqkm").unwrap_or(0.0),
        rings: parts(f.geometry.as_ref(), "rings")?,
    })
}

/// The `[x=lon, y=lat]` parts under `key` of an Esri geometry, as points; no geometry gives
/// no parts.
fn parts(geometry: Option<&Value>, key: &str) -> Result<Vec<Vec<Point>>, SourceError> {
    let Some(list) = geometry.and_then(|g| g.get(key)) else {
        return Ok(Vec::new());
    };
    let bad = || SourceError::Parse(format!("malformed {key} geometry"));
    list.as_array()
        .ok_or_else(bad)?
        .iter()
        .map(|part| {
            part.as_array()
                .ok_or_else(bad)?
                .iter()
                .map(|v| {
                    let xy = v.as_array().ok_or_else(bad)?;
                    let x = xy.first().and_then(Value::as_f64).ok_or_else(bad)?;
                    let y = xy.get(1).and_then(Value::as_f64).ok_or_else(bad)?;
                    Ok(Point::new(y, x))
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;
    use underbite_core::time::UnixTime;

    fn ctx() -> FetchContext {
        FetchContext::new(UnixTime(1_794_000_000), "underbite/test")
    }

    fn croton() -> Coordinates {
        Coordinates {
            latitude: 41.2250,
            longitude: -73.8589,
        }
    }

    fn fx(name: &str) -> Vec<u8> {
        fixture_bytes("hydro", name)
    }

    #[test]
    fn croton_flowlines_have_geometry_and_the_level_path() {
        let body = fx("hydro-50-croton-envelope.json");
        let lines = parse_flowlines(&Response::new(200, &body)).unwrap();
        assert_eq!(lines.len(), 21);
        let on_path: Vec<_> = lines.iter().filter(|l| l.level_path == 3_767_999).collect();
        assert_ne!(on_path.len(), 0);
        let first = on_path.iter().min_by_key(|l| l.hydrosequence).unwrap();
        assert_eq!(first.name.as_deref(), Some("Croton River"));
        assert!(!first.paths.is_empty() && first.paths.iter().all(|p| p.len() >= 2));
        for p in lines.iter().flat_map(|l| l.paths.iter().flatten()) {
            assert!((41.0..41.4).contains(&p.latitude), "latitude first: {p:?}");
            assert!(
                (-74.0..-73.7).contains(&p.longitude),
                "longitude second: {p:?}"
            );
        }
        let summary: Vec<_> = lines
            .iter()
            .map(|l| {
                (
                    l.id.as_str(),
                    l.name.as_deref(),
                    l.kind,
                    l.stream_order,
                    l.level_path,
                    l.hydrosequence,
                    l.dn_hydrosequence,
                    l.up_hydrosequence,
                    l.paths.iter().map(Vec::len).collect::<Vec<_>>(),
                )
            })
            .collect();
        insta::assert_yaml_snapshot!(summary);
    }

    #[test]
    fn flowlines_without_geometry_parse() {
        let body = fx("hydro-50-croton-point-distance.json");
        let lines = parse_flowlines(&Response::new(200, &body)).unwrap();
        assert_eq!(lines.len(), 11);
        assert!(lines.iter().all(|l| l.paths.is_empty()));
    }

    #[test]
    fn battery_flowlines_and_waterbodies() {
        let body = fx("hydro-50-battery-envelope.json");
        let lines = parse_flowlines(&Response::new(200, &body)).unwrap();
        assert_eq!(lines.len(), 4);
        assert!(
            lines
                .iter()
                .any(|l| l.kind == FlowlineKind::WaterbodyConnector)
        );
        let body = fx("hydro-60-battery-envelope.json");
        let waters = parse_waterbodies(&Response::new(200, &body)).unwrap();
        assert_eq!(waters.len(), 4);
        assert!(waters.iter().all(|w| w.gnis_id.is_none()));
        let bay = waters
            .iter()
            .find(|w| w.name.as_deref() == Some("Upper Bay"))
            .unwrap();
        assert_eq!(bay.kind, WaterbodyKind::River);
        assert!((bay.area_km2 - 43.07).abs() < 0.01);
    }

    #[test]
    fn new_croton_reservoir_polygon() {
        let body = fx("hydro-60-new-croton-reservoir-geom.json");
        let waters = parse_waterbodies(&Response::new(200, &body)).unwrap();
        let [w] = waters.as_slice() else {
            panic!("one waterbody")
        };
        assert_eq!(w.id, "IZTBN");
        assert_eq!(w.name.as_deref(), Some("New Croton Reservoir"));
        assert_eq!(w.rings.len(), 5);
        for p in w.rings.iter().flatten() {
            assert!((41.2..41.3).contains(&p.latitude) && (-73.9..-73.7).contains(&p.longitude));
        }
        assert!(w.distance_m(Point::new(41.23, -73.84)) < 1_000.0);
        assert!(w.distance_m(Point::new(41.27, -73.78)) > 0.0);
    }

    #[test]
    fn great_sacandaga_pieces_share_a_gnis_id() {
        let body = fx("hydro-60-great-sacandaga-point.json");
        let waters = parse_waterbodies(&Response::new(200, &body)).unwrap();
        let lake: Vec<_> = waters
            .iter()
            .filter(|w| w.gnis_id == Some(973_352))
            .collect();
        assert!(lake.len() >= 2);
        assert!(lake.iter().all(|w| w.rings.is_empty()));
        assert!(lake.iter().any(|w| (w.area_km2 - 6.40).abs() < 0.01));
        insta::assert_yaml_snapshot!(waters);
    }

    #[test]
    fn error_object_under_200() {
        for name in ["hydro-error-bad-where.json", "hydro-error-bad-layer.json"] {
            let body = fx(name);
            let err = parse_flowlines(&Response::new(200, &body)).unwrap_err();
            assert!(
                matches!(
                    err,
                    SourceError::Status {
                        status: 400,
                        message: Some(_)
                    }
                ),
                "{err:?}"
            );
            assert!(parse_waterbodies(&Response::new(200, &body)).is_err());
        }
    }

    #[test]
    fn http_errors_map() {
        assert!(matches!(
            parse_flowlines(&Response::new(500, b"oops")),
            Err(SourceError::Status { status: 500, .. })
        ));
        assert!(matches!(
            parse_flowlines(&Response::new(429, b"")),
            Err(SourceError::RateLimited { .. })
        ));
        assert!(matches!(
            parse_flowlines(&Response::new(200, b"<html>")),
            Err(SourceError::Parse(_))
        ));
        let none = parse_flowlines(&Response::new(200, br#"{"features":[]}"#)).unwrap();
        assert_eq!(none.len(), 0);
    }

    #[test]
    fn paging_follows_the_transfer_limit_flag() {
        let q = flowlines_query(croton(), 0.0, &ctx()).unwrap();
        let body = fx("hydro-50-croton-paging-offset3-count3.json");
        let next = next_page(&q, &Response::new(200, &body)).unwrap().unwrap();
        assert_eq!(next.offset, PAGE_SIZE);
        assert!(next.url.ends_with(&format!("&resultOffset={PAGE_SIZE}")));
        assert_eq!(envelope_from_url(&next.url), envelope_from_url(&q.url));
        let mut last = next;
        while let Some(n) = next_page(&last, &Response::new(200, &body)).unwrap() {
            assert_eq!(n.url.matches("resultOffset").count(), 1);
            last = n;
        }
        assert_eq!(last.offset, PAGE_SIZE * (MAX_PAGES - 1));
        let done = fx("hydro-50-croton-envelope.json");
        assert_eq!(next_page(&q, &Response::new(200, &done)).unwrap(), None);
    }

    #[test]
    fn flowline_request_is_a_validated_envelope_around_the_rounded_spot() {
        let q = flowlines_query(croton(), 0.0, &ctx()).unwrap();
        assert!(
            q.url
                .starts_with(&format!("{BASE_URL}/50/query?where=1=1&geometry="))
        );
        for need in [
            "geometryType=esriGeometryEnvelope",
            "inSR=4326",
            "outSR=4326",
            "spatialRel=esriSpatialRelIntersects",
            "returnGeometry=true",
            "f=json",
            "orderByFields=hydrosequence%20ASC",
        ] {
            assert!(q.url.contains(need), "{need}");
        }
        assert!(!q.url.contains(' '));
        let [xmin, ymin, xmax, ymax] = envelope_from_url(&q.url).expect("valid envelope");
        // Centred on the rounded spot (41.23, -73.86), not the exact one.
        assert!((f64::midpoint(xmin, xmax) + 73.86).abs() < 1e-4);
        assert!((f64::midpoint(ymin, ymax) - 41.23).abs() < 1e-4);
        // The exact spot is inside, and the box reaches at least 1.5 km each way.
        assert!(xmin < -73.8589 && -73.8589 < xmax && ymin < 41.2250 && 41.2250 < ymax);
        assert!((ymax - ymin) * METRES_PER_DEGREE / 2.0 >= MIN_RADIUS_M - 1.0);
        assert!(!q.url.contains("73.8589") && !q.url.contains("41.225"));
        assert_eq!(q.max_age_s, MAX_AGE_S);
    }

    #[test]
    fn waterbody_request_uses_a_point_with_distance() {
        let q = waterbodies_query(croton(), 3000.0, &ctx()).unwrap();
        assert!(q.url.contains(
            "/60/query?where=1=1&geometry=-73.86000,41.23000&geometryType=esriGeometryPoint"
        ));
        assert!(q.url.contains("distance=3000&units=esriSRUnit_Meter"));
        assert!(q.url.contains("outSR=4326"));
        assert!(q.url.contains("maxAllowableOffset=0.0005"));
        assert!(!q.url.contains(' '));
    }

    #[test]
    fn bad_coordinates_are_refused_and_envelope_validation_catches_junk() {
        let nan = Coordinates {
            latitude: f64::NAN,
            longitude: 0.0,
        };
        assert!(flowlines_query(nan, 0.0, &ctx()).is_err());
        let high = Coordinates {
            latitude: 95.0,
            longitude: 0.0,
        };
        assert!(waterbodies_query(high, 0.0, &ctx()).is_err());
        assert_eq!(envelope_from_url("x?geometry=abc&f=json"), None);
        assert_eq!(envelope_from_url("x?geometry=1,2,0,3&f=json"), None);
        assert_eq!(envelope_from_url("x?geometry=-190,2,0,3&f=json"), None);
    }

    #[test]
    fn credit_names_the_program() {
        let a = attribution();
        assert!(
            a.credit
                .starts_with("USGS TNM / NGTOC - 3D National Hydrographic Program (3DHP)")
        );
        assert_eq!(a.source.as_str(), "usgs-3dhp");
    }
}
