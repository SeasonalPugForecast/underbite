//! The USGS Protected Areas Database of the United States (PAD-US): public land and its access.
//!
//! Public-land polygons from the PAD-US public access layer, for the map only (sans-IO). They are
//! not part of the spot snapshot, so this module has no `Source` implementation: a host calls
//! [`query_request`] for a box and a page, sends the GET, and hands the body to [`parse_page`],
//! asking again at the next offset while [`PadusPage::more`] is set.
//!
//! The layer has no coded-value domain, so `Pub_Access` arrives as a two-letter string and its
//! meaning is held here ([`access_level`]): `OA` open, `RA` restricted (a permit, season or
//! registration), `XA` closed, `UK` unknown. An unmapped code is unknown, never open. The level
//! is the source's record and is not legal permission to fish; wording for that belongs to the
//! view.
//!
//! A point query misses (a polygon contains a point only when the point is inside it), so the
//! request is always a box, in WGS 84 degrees (`outSR=4326`; without it geometry comes in Web
//! Mercator). Credit: [`PADUS_CREDIT`].

use serde_json::Value;
use underbite_core::map::{Bbox, GeoPoint};
use underbite_core::model::nearby::{AccessFeature, AccessGeometry, AccessKind, AccessLevel};
use underbite_core::provenance::SourceId;

use crate::request::{MAX_COORDINATE_DECIMALS, QueryBuilder, Response};

const QUERY_URL: &str = "https://services.arcgis.com/v01gqwM5QqNysAAi/arcgis/rest/services/PADUS_Public_Access/FeatureServer/0/query";

/// Features asked for per page. The service allows 2000 (`maxRecordCount`); large parks are heavy
/// polygons, so pages are smaller.
pub const PAGE_SIZE: u32 = 500;

/// The attributes the parser reads.
const OUT_FIELDS: &str = "Unit_Nm,Pub_Access,GAP_Sts,Category,DesTp_Desc,MngNm_Desc,GIS_Acres";

/// The citation USGS asks for.
pub const PADUS_CREDIT: &str = "U.S. Geological Survey (USGS) Gap Analysis Project (GAP), 2024, Protected Areas Database of the United States (PAD-US) 4.1, https://doi.org/10.5066/P96WBCHS";

/// One GET for the host to send. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadusRequest {
    pub url: String,
}

/// A PAD-US request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PadusError {
    /// The request asks for something that cannot be sent.
    #[error("invalid PAD-US request: {0}")]
    InvalidRequest(&'static str),
    /// The service answered with an HTTP failure.
    #[error("PAD-US answered HTTP {status}")]
    Status { status: u16 },
    /// HTTP 200 with an `ArcGIS` error object.
    #[error("PAD-US reported an error ({code}): {message}")]
    Service { code: i64, message: String },
    /// The body is not the expected JSON.
    #[error("the PAD-US response is not the expected JSON: {0}")]
    Parse(String),
}

/// The level a `Pub_Access` code stands for (data dictionary, Table 14). Anything else, including
/// `UK` and an empty value, is [`AccessLevel::Unknown`].
pub fn access_level(code: &str) -> AccessLevel {
    match code.trim() {
        "OA" => AccessLevel::Open,
        "RA" => AccessLevel::Restricted,
        "XA" => AccessLevel::Closed,
        _ => AccessLevel::Unknown,
    }
}

/// Rounds the box outward to the coordinate precision sent to data sources, so it still covers
/// the area asked for.
fn outward(bbox: Bbox) -> Bbox {
    let scale = 10f64.powi(i32::from(MAX_COORDINATE_DECIMALS));
    Bbox {
        west: (bbox.west * scale).floor() / scale,
        south: (bbox.south * scale).floor() / scale,
        east: (bbox.east * scale).ceil() / scale,
        north: (bbox.north * scale).ceil() / scale,
    }
}

/// The `query` for the public-access polygons that meet a box, one page.
///
/// The box edges are rounded outward to 2 decimals (about 1 km). `offset` is the number of
/// features to skip (0, then `offset + PAGE_SIZE` while [`PadusPage::more`] is set); pages are
/// ordered by `OBJECTID` so their boundaries hold. `tolerance_deg` simplifies the geometry on the
/// server (`maxAllowableOffset`, degrees: 0.0005 is about 55 m; larger for a wider view); zero
/// or less asks for full geometry. The answer is `GeoJSON` in WGS 84.
///
/// # Errors
/// [`PadusError::InvalidRequest`] for a box that is not finite, outside the world or inverted, or
/// a tolerance that is not finite.
pub fn query_request(
    bbox: Bbox,
    offset: u32,
    tolerance_deg: f64,
) -> Result<PadusRequest, PadusError> {
    let ok = [bbox.west, bbox.south, bbox.east, bbox.north]
        .iter()
        .all(|v| v.is_finite())
        && bbox.west <= bbox.east
        && bbox.south <= bbox.north
        && bbox.west >= -180.0
        && bbox.east <= 180.0
        && bbox.south >= -90.0
        && bbox.north <= 90.0;
    if !ok {
        return Err(PadusError::InvalidRequest("box"));
    }
    if !tolerance_deg.is_finite() {
        return Err(PadusError::InvalidRequest("tolerance"));
    }
    let b = outward(bbox);
    let mut q = QueryBuilder::new(QUERY_URL)
        .param("f", "geojson")
        .param(
            "geometry",
            format!("{:.2},{:.2},{:.2},{:.2}", b.west, b.south, b.east, b.north),
        )
        .param("geometryType", "esriGeometryEnvelope")
        .param("inSR", "4326")
        .param("spatialRel", "esriSpatialRelIntersects")
        .param("outSR", "4326")
        .param("outFields", OUT_FIELDS)
        .param("returnGeometry", "true")
        .param("orderByFields", "OBJECTID")
        .param("resultOffset", offset.to_string())
        .param("resultRecordCount", PAGE_SIZE.to_string());
    if tolerance_deg > 0.0 {
        q = q
            .param("maxAllowableOffset", format!("{tolerance_deg}"))
            .param("geometryPrecision", "5");
    }
    Ok(PadusRequest { url: q.finish() })
}

/// The features one page returned.
#[derive(Clone, Debug, PartialEq)]
pub struct PadusPage {
    /// Public land, one feature per polygon (a multi-part area gives several).
    pub features: Vec<AccessFeature>,
    /// Rows the page held.
    pub rows: usize,
    /// Rows left out for missing or invalid geometry.
    pub skipped: usize,
    /// Whether another page may hold more: the service said so, or the page was full.
    pub more: bool,
}

fn text(props: &Value, key: &str) -> Option<String> {
    props
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn point(v: &Value) -> Option<GeoPoint> {
    let a = v.as_array()?;
    let lon = a.first()?.as_f64()?;
    let lat = a.get(1)?.as_f64()?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
        .then_some(GeoPoint { lat, lon })
}

/// A ring of `[lon, lat]` pairs; none when it is too short or holds an invalid position.
fn ring(v: &Value) -> Option<Vec<GeoPoint>> {
    let r = v
        .as_array()?
        .iter()
        .map(point)
        .collect::<Option<Vec<_>>>()?;
    (r.len() >= 3).then_some(r)
}

fn rings(v: &Value) -> Option<Vec<Vec<GeoPoint>>> {
    let r = v.as_array()?.iter().map(ring).collect::<Option<Vec<_>>>()?;
    (!r.is_empty()).then_some(r)
}

/// Whether the ring runs clockwise (x east, y north).
fn clockwise(r: &[GeoPoint]) -> bool {
    let mut sum = 0.0;
    for (i, a) in r.iter().enumerate() {
        let b = r[(i + 1) % r.len()];
        sum += (b.lon - a.lon) * (b.lat + a.lat);
    }
    sum > 0.0
}

/// The polygons of one feature, in either shape: `GeoJSON` (`Polygon` or `MultiPolygon`) or `JSON`
/// (`rings`, where a clockwise ring starts a polygon and the counter-clockwise ones that follow
/// are its holes). Empty for a missing or invalid geometry.
pub(crate) fn polygons(f: &Value) -> Vec<Vec<Vec<GeoPoint>>> {
    let Some(g) = f.get("geometry").filter(|g| !g.is_null()) else {
        return Vec::new();
    };
    if let Some(r) = g.get("rings") {
        let Some(all) = rings(r) else {
            return Vec::new();
        };
        let mut out: Vec<Vec<Vec<GeoPoint>>> = Vec::new();
        for r in all {
            match out.last_mut() {
                Some(p) if !clockwise(&r) => p.push(r),
                _ => out.push(vec![r]),
            }
        }
        return out;
    }
    let coords = g.get("coordinates");
    match g.get("type").and_then(Value::as_str) {
        Some("Polygon") => coords.and_then(rings).into_iter().collect(),
        Some("MultiPolygon") => coords
            .and_then(Value::as_array)
            .and_then(|parts| parts.iter().map(rings).collect::<Option<Vec<_>>>())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Reads one page, `GeoJSON` as [`query_request`] asks for it or `JSON` (`attributes` and
/// `rings`). A row without usable geometry is counted in [`PadusPage::skipped`].
///
/// # Errors
/// [`PadusError::Status`] for an HTTP failure, [`PadusError::Service`] for an `ArcGIS` error and
/// [`PadusError::Parse`] for a body that holds no feature list.
pub fn parse_page(response: &Response<'_>) -> Result<PadusPage, PadusError> {
    if !(200..=299).contains(&response.status) {
        return Err(PadusError::Status {
            status: response.status,
        });
    }
    let v: Value =
        serde_json::from_slice(response.body).map_err(|e| PadusError::Parse(e.to_string()))?;
    if let Some(e) = v.get("error") {
        return Err(PadusError::Service {
            code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        });
    }
    let rows = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| PadusError::Parse("no features".to_owned()))?;
    let mut features = Vec::new();
    let mut skipped = 0;
    for f in rows {
        let props = f
            .get("attributes")
            .or_else(|| f.get("properties"))
            .unwrap_or(&Value::Null);
        let parts = polygons(f);
        if parts.is_empty() {
            skipped += 1;
            continue;
        }
        let name = text(props, "Unit_Nm");
        let access = access_level(
            props
                .get("Pub_Access")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        for p in parts {
            features.push(AccessFeature {
                source: SourceId::PADUS,
                kind: AccessKind::PublicLand,
                name: name.clone(),
                geometry: AccessGeometry::Polygon(p),
                access,
            });
        }
    }
    let flagged = v
        .get("exceededTransferLimit")
        .or_else(|| v.pointer("/properties/exceededTransferLimit"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(PadusPage {
        features,
        rows: rows.len(),
        skipped,
        more: flagged || rows.len() >= PAGE_SIZE as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;

    fn bbox(w: f64, s: f64, e: f64, n: f64) -> Bbox {
        Bbox {
            west: w,
            south: s,
            east: e,
            north: n,
        }
    }

    #[test]
    fn codes_map_and_unknown_is_never_open() {
        assert_eq!(access_level("OA"), AccessLevel::Open);
        assert_eq!(access_level("RA"), AccessLevel::Restricted);
        assert_eq!(access_level("XA"), AccessLevel::Closed);
        assert_eq!(access_level("UK"), AccessLevel::Unknown);
        for odd in ["", "oa", "Open", "??"] {
            assert_eq!(access_level(odd), AccessLevel::Unknown, "{odd}");
        }
    }

    #[test]
    fn request_is_an_envelope_rounded_outward() {
        let r = query_request(bbox(-73.9049, 41.1701, -73.8651, 41.2299), 500, 0.0005).unwrap();
        let q = r.url.split_once('?').unwrap().1;
        for want in [
            "geometry=-73.91,41.17,-73.86,41.23",
            "geometryType=esriGeometryEnvelope",
            "inSR=4326",
            "outSR=4326",
            "orderByFields=OBJECTID",
            "resultOffset=500",
            "resultRecordCount=500",
            "maxAllowableOffset=0.0005",
            "geometryPrecision=5",
            "f=geojson",
        ] {
            assert!(q.split('&').any(|p| p == want), "{want} in {q}");
        }
        let full = query_request(bbox(0.0, 0.0, 1.0, 1.0), 0, 0.0).unwrap();
        assert!(!full.url.contains("maxAllowableOffset"));
    }

    #[test]
    fn request_rejects_bad_input() {
        assert!(query_request(bbox(1.0, 0.0, 0.0, 1.0), 0, 0.0).is_err());
        assert!(query_request(bbox(f64::NAN, 0.0, 1.0, 1.0), 0, 0.0).is_err());
        assert!(query_request(bbox(0.0, 0.0, 1.0, 91.0), 0, 0.0).is_err());
        assert!(query_request(bbox(0.0, 0.0, 1.0, 1.0), 0, f64::INFINITY).is_err());
    }

    #[test]
    fn parses_geojson_page_with_multipolygon() {
        let body = fixture_bytes(
            "padus",
            "padus-croton-geojson-offset0.0005-5rows-2026-11-17.json",
        );
        let page = parse_page(&Response::new(200, &body)).unwrap();
        assert_eq!(page.rows, 5);
        assert_eq!(page.skipped, 0);
        assert!(page.more, "exceededTransferLimit is true");
        assert!(page.features.len() >= 5, "a multipolygon adds features");
        let first = &page.features[0];
        assert_eq!(first.source, SourceId::PADUS);
        assert_eq!(first.kind, AccessKind::PublicLand);
        assert_eq!(first.access, AccessLevel::Unknown);
        assert_eq!(
            first.name.as_deref(),
            Some("VILLAGE OF CROTON-ON-HUDS Land")
        );
        let AccessGeometry::Polygon(rings) = &first.geometry else {
            panic!("polygon");
        };
        assert_eq!(rings[0].len(), 6);
        assert_eq!(rings[0][0].lon, -73.89556);
        assert_eq!(page.features[1].access, AccessLevel::Open);
    }

    #[test]
    fn attribute_only_rows_are_skipped_but_counted() {
        let body = fixture_bytes("tier2", "padus-public-access-croton-point.json");
        let page = parse_page(&Response::new(200, &body)).unwrap();
        assert_eq!(page.rows, 10);
        assert_eq!(page.skipped, 10);
        assert_eq!(page.features.len(), 0);
        assert!(page.more);
    }

    #[test]
    fn parses_esri_json_rings_into_polygons_with_holes() {
        let body = br#"{"exceededTransferLimit":false,"features":[
          {"attributes":{"Unit_Nm":" Croton Gorge County Park ","Pub_Access":"OA"},
           "geometry":{"rings":[
             [[-73.88,41.20],[-73.88,41.22],[-73.86,41.22],[-73.86,41.20],[-73.88,41.20]],
             [[-73.875,41.205],[-73.865,41.205],[-73.865,41.215],[-73.875,41.215],[-73.875,41.205]],
             [[-73.80,41.20],[-73.80,41.21],[-73.79,41.21],[-73.79,41.20],[-73.80,41.20]]]}},
          {"attributes":{"Unit_Nm":"No shape","Pub_Access":"XA"}},
          {"attributes":{"Unit_Nm":"Bad","Pub_Access":"RA"},"geometry":{"rings":[[[999,1],[1,1],[1,2]]]}}]}"#;
        let page = parse_page(&Response::new(200, body)).unwrap();
        assert_eq!((page.rows, page.skipped, page.more), (3, 2, false));
        assert_eq!(page.features.len(), 2);
        assert_eq!(
            page.features[0].name.as_deref(),
            Some("Croton Gorge County Park")
        );
        let AccessGeometry::Polygon(a) = &page.features[0].geometry else {
            panic!("polygon");
        };
        assert_eq!(a.len(), 2, "outer ring and one hole");
        let AccessGeometry::Polygon(b) = &page.features[1].geometry else {
            panic!("polygon");
        };
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn errors_and_full_pages() {
        assert_eq!(
            parse_page(&Response::new(503, b"")),
            Err(PadusError::Status { status: 503 })
        );
        assert!(matches!(
            parse_page(&Response::new(
                200,
                br#"{"error":{"code":400,"message":"bad"}}"#
            )),
            Err(PadusError::Service { code: 400, .. })
        ));
        assert!(matches!(
            parse_page(&Response::new(200, b"<html>")),
            Err(PadusError::Parse(_))
        ));
        assert!(matches!(
            parse_page(&Response::new(200, br"{}")),
            Err(PadusError::Parse(_))
        ));
        let rows = vec![r#"{"attributes":{"Pub_Access":"OA"}}"#; PAGE_SIZE as usize].join(",");
        let body = format!(r#"{{"features":[{rows}]}}"#);
        assert!(
            parse_page(&Response::new(200, body.as_bytes()))
                .unwrap()
                .more
        );
        assert!(PADUS_CREDIT.contains("10.5066/P96WBCHS"));
    }
}
