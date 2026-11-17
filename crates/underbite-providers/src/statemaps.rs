//! State lake contours (feature queries returning lines) and the New York artificial reef list (sans-IO).
//!
//! Two states publish lake contours as `ArcGIS` services: Indiana DNR (a `FeatureServer`) and Minnesota
//! DNR (a `MapServer`). [`contour_request`] builds the `query` by envelope, in WGS 84 degrees, as
//! `GeoJSON`, 2000 features a page and generalised for the zoom; [`parse_contours`] reads the answer
//! into [`Polyline`]s whose `value` is the depth **below the lake surface** in metres, positive
//! down. New York has no statewide contour layer; [`reefs_request`] and [`parse_reefs`] read the
//! NYSDEC artificial reef table (Socrata JSON).
//!
//! The reference of a contour depth is the lake surface, not a datum: Indiana's contours are "feet
//! below the surface" (some lakes adjusted to the legal gauge level) and Minnesota's are depths
//! below the lake surface of the original map. They are never the DEM's "approximate depth".
//!
//! Units. Indiana states feet in the layer description. Minnesota's layer does not state its
//! unit: feet is taken from the DNR lake DEM metadata (unit per DNR DEM metadata, unconfirmed for
//! the contour layer).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use underbite_core::map::{Bbox, GeoPoint, Polyline};
use underbite_core::time::UnixTime;

use crate::request::{QueryBuilder, Response};

/// Metres per international foot.
const M_PER_FT: f64 = 0.3048;
/// Metres per fathom (six feet).
const M_PER_FATHOM: f64 = 1.8288;

/// The most features either contour service returns in one page (`maxRecordCount`).
pub const PAGE_SIZE: u32 = 2000;

const IN_QUERY: &str =
    "https://gisdata.in.gov/server/rest/services/Hosted/Lake_Bathymetry/FeatureServer/0/query";
const MN_QUERY: &str = "https://enterprise.gisdata.mn.gov/aghost/rest/services/us_mn_state_dnr/water_lake_bathymetry/MapServer/0/query";
const NY_REEFS: &str = "https://data.ny.gov/resource/jsnj-ehv9.json";

/// The deepest zoom a generalisation tolerance is derived for.
const MAX_ZOOM: u8 = 24;

/// A state with a lake contour service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContourState {
    /// Indiana DNR, Division of Fish and Wildlife (`FeatureServer`; depth in feet below the surface).
    Indiana,
    /// Minnesota DNR (`MapServer` layer 0; negative depth in feet, unit per DNR DEM metadata,
    /// unconfirmed for the contour layer).
    Minnesota,
}

impl ContourState {
    /// Metres per unit of the source's depth values (feet in both states).
    const fn metres_per_unit(self) -> f64 {
        match self {
            Self::Indiana | Self::Minnesota => M_PER_FT,
        }
    }

    fn base(self) -> &'static str {
        match self {
            Self::Indiana => IN_QUERY,
            Self::Minnesota => MN_QUERY,
        }
    }

    /// The attribute fields to ask for.
    fn out_fields(self) -> &'static str {
        match self {
            Self::Indiana => {
                "objectid,contour,line_type,lake_name,survey_date,gauge_adjusted_level,lake_max_depth"
            }
            Self::Minnesota => "objectid,depth,abs_depth,dowlknum,lake_name",
        }
    }
}

/// One GET for the host to send. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateMapRequest {
    pub url: String,
}

/// A state map request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateMapError {
    /// The request asks for something that cannot be sent.
    #[error("invalid state map request: {0}")]
    InvalidRequest(&'static str),
    /// The service answered with an HTTP failure.
    #[error("the service answered HTTP {status}")]
    Status { status: u16 },
    /// The service answered 200 with an error object (`ArcGIS` does this for a bad query).
    #[error("the service reported an error ({code}): {message}")]
    Service { code: i64, message: String },
    /// The body is not the expected JSON.
    #[error("the response is not the expected JSON: {0}")]
    Parse(String),
}

/// The generalisation tolerance for a zoom, in degrees: one pixel of a 256-pixel tile at the
/// equator, so a request for a view drops detail the display cannot show.
pub fn tolerance_deg(zoom: u8) -> f64 {
    360.0 / 256.0 / f64::from(1u32 << u32::from(zoom.min(MAX_ZOOM)))
}

/// The `query` for the contours of a state inside a box, one page.
///
/// `zoom` sets `maxAllowableOffset` ([`tolerance_deg`]); `offset` is the number of features to
/// skip (0 for the first page; the next page starts at `offset + PAGE_SIZE` while
/// [`LakeContours::more`] is set).
///
/// # Errors
/// [`StateMapError::InvalidRequest`] for a box that is not finite, outside the world or inverted.
pub fn contour_request(
    state: ContourState,
    bbox: Bbox,
    zoom: u8,
    offset: u32,
) -> Result<StateMapRequest, StateMapError> {
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
        return Err(StateMapError::InvalidRequest("box"));
    }
    let geometry = format!(
        "{:.6},{:.6},{:.6},{:.6}",
        bbox.west, bbox.south, bbox.east, bbox.north
    );
    let mut q = QueryBuilder::new(state.base())
        .param("where", "1=1")
        .param("geometry", geometry)
        .param("geometryType", "esriGeometryEnvelope")
        .param("inSR", "4326")
        .param("spatialRel", "esriSpatialRelIntersects")
        .param("outFields", state.out_fields())
        .param("outSR", "4326")
        .param("geometryPrecision", "6")
        .param("maxAllowableOffset", format!("{:.8}", tolerance_deg(zoom)));
    if state == ContourState::Indiana {
        // Verified for the Indiana service; the Minnesota one is paged in its own order.
        q = q.param("orderByFields", "objectid");
    }
    let url = q
        .param("resultOffset", offset.to_string())
        .param("resultRecordCount", PAGE_SIZE.to_string())
        .param("f", "geojson")
        .finish();
    Ok(StateMapRequest { url })
}

/// A contour line with what the source says about it.
#[derive(Clone, Debug, PartialEq)]
pub struct ContourLine {
    /// The line; `value` is the depth below the lake surface in metres, positive down.
    pub line: Polyline,
    /// The lake's name as the source gives it.
    pub lake: Option<String>,
    /// Minnesota's DNR lake number (eight digits).
    pub lake_id: Option<String>,
    /// Indiana's index (labelled) contour, as against an intermediate one.
    pub index: Option<bool>,
    /// The survey date Indiana gives per feature.
    pub survey: Option<UnixTime>,
    /// Whether Indiana lists a gauge-adjusted level for the lake: the depths were adjusted to the
    /// lake's legal gauge level.
    pub gauge_adjusted: bool,
}

/// The contours one page returned.
#[derive(Clone, Debug, PartialEq)]
pub struct LakeContours {
    pub lines: Vec<ContourLine>,
    /// Features left out for a missing depth or no line of two valid points.
    pub skipped: usize,
    /// Whether another page may hold more: the service said so, or the page was full.
    pub more: bool,
}

fn check(response: &Response<'_>) -> Result<(), StateMapError> {
    match response.status {
        200..=299 => Ok(()),
        status => Err(StateMapError::Status { status }),
    }
}

/// An `ArcGIS` error object in a 200 response, if the body is one.
fn service_error(v: &Value) -> Option<StateMapError> {
    let e = v.get("error")?;
    Some(StateMapError::Service {
        code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
        message: e
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

fn position(v: &Value) -> Option<GeoPoint> {
    let a = v.as_array()?;
    let lon = a.first()?.as_f64()?;
    let lat = a.get(1)?.as_f64()?;
    ((-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat))
        .then_some(GeoPoint { lat, lon })
}

fn line(v: &Value) -> Option<Vec<GeoPoint>> {
    let pts: Option<Vec<GeoPoint>> = v.as_array()?.iter().map(position).collect();
    pts.filter(|p| p.len() >= 2)
}

/// The parts of a feature's geometry: a `LineString` is one, a `MultiLineString` several. A part
/// that is not a line of valid points is dropped.
fn parts(geometry: &Value) -> Vec<Vec<GeoPoint>> {
    let coords = geometry.get("coordinates");
    match (geometry.get("type").and_then(Value::as_str), coords) {
        (Some("LineString"), Some(c)) => line(c).into_iter().collect(),
        (Some("MultiLineString"), Some(c)) => c
            .as_array()
            .map(|a| a.iter().filter_map(line).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn number(props: &Value, key: &str) -> Option<f64> {
    props.get(key)?.as_f64().filter(|v| v.is_finite())
}

fn text(props: &Value, key: &str) -> Option<String> {
    props
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Reads a contour page.
///
/// # Errors
/// [`StateMapError::Status`] for an HTTP failure, [`StateMapError::Service`] for an `ArcGIS` error
/// object and [`StateMapError::Parse`] for a body that is not a `GeoJSON` feature collection.
pub fn parse_contours(
    state: ContourState,
    response: &Response<'_>,
) -> Result<LakeContours, StateMapError> {
    check(response)?;
    let v: Value =
        serde_json::from_slice(response.body).map_err(|e| StateMapError::Parse(e.to_string()))?;
    if let Some(e) = service_error(&v) {
        return Err(e);
    }
    let features = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| StateMapError::Parse("no features".to_owned()))?;
    let flagged = v.get("exceededTransferLimit").and_then(Value::as_bool) == Some(true)
        || v.pointer("/properties/exceededTransferLimit")
            .and_then(Value::as_bool)
            == Some(true);
    let indiana = state == ContourState::Indiana;
    let mut lines = Vec::new();
    let mut skipped = 0;
    for f in features {
        let props = f.get("properties").unwrap_or(&Value::Null);
        let depth = match state {
            ContourState::Indiana => number(props, "contour"),
            ContourState::Minnesota => {
                number(props, "depth").or_else(|| number(props, "abs_depth"))
            }
        };
        let geometry = f.get("geometry").map(parts).unwrap_or_default();
        let Some(depth) = depth.filter(|_| !geometry.is_empty()) else {
            skipped += 1;
            continue;
        };
        let value = depth.abs() * state.metres_per_unit();
        for points in geometry {
            lines.push(ContourLine {
                line: Polyline { points, value },
                lake: text(props, "lake_name"),
                lake_id: text(props, "dowlknum"),
                index: if indiana {
                    text(props, "line_type").map(|t| t.eq_ignore_ascii_case("INDEX"))
                } else {
                    None
                },
                survey: if indiana {
                    // Epoch milliseconds; whole seconds are enough for a survey date.
                    number(props, "survey_date").map(|ms| UnixTime((ms / 1000.0).floor() as i64))
                } else {
                    None
                },
                gauge_adjusted: indiana && number(props, "gauge_adjusted_level").is_some(),
            });
        }
    }
    Ok(LakeContours {
        lines,
        skipped,
        more: flagged || features.len() >= PAGE_SIZE as usize,
    })
}

/// The request for every NYSDEC artificial reef (16 rows; 50 leaves room).
pub fn reefs_request() -> StateMapRequest {
    StateMapRequest {
        url: QueryBuilder::new(NY_REEFS).param("$limit", "50").finish(),
    }
}

/// An artificial reef site: the general area where material was placed, not a point.
#[derive(Clone, Debug, PartialEq)]
pub struct ReefSite {
    pub name: String,
    /// The mean of the listed corners, for a marker.
    pub position: GeoPoint,
    /// The listed corners (northwest, northeast, southeast, southwest) that parsed.
    pub corners: Vec<GeoPoint>,
    pub body_of_water: Option<String>,
    /// "3.0 Nautical Miles South of Atlantic Beach", as written.
    pub location: Option<String>,
    /// "413 acres", as written.
    pub size: Option<String>,
    /// The minimum depth in metres, when the text has a number and a unit this reads; the source
    /// states no datum.
    pub min_depth_m: Option<f64>,
    pub max_depth_m: Option<f64>,
    /// The depth texts as the source writes them ("55 feet").
    pub min_depth_text: Option<String>,
    pub max_depth_text: Option<String>,
    pub comments: Option<String>,
}

#[derive(Deserialize)]
struct ReefRow {
    reef_name: Option<String>,
    body_of_water: Option<String>,
    description_of_location: Option<String>,
    size: Option<String>,
    minimum_depth: Option<String>,
    maximum_depth: Option<String>,
    comments: Option<String>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

/// The reefs of a response, and how many rows had no name or no readable corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Reefs {
    pub sites: Vec<ReefSite>,
    pub skipped: usize,
}

/// A depth text such as "55 feet" in metres: a number and a unit this reads (feet, fathoms,
/// metres, in any case). Anything else gives None: no unit is guessed.
pub fn parse_depth_m(text: &str) -> Option<f64> {
    let t = text.trim();
    let end = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(end);
    let n: f64 = num.replace(',', "").parse().ok()?;
    let per_unit = match unit
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "feet" | "foot" | "ft" => M_PER_FT,
        "fathoms" | "fathom" | "fm" => M_PER_FATHOM,
        "meters" | "meter" | "metres" | "metre" | "m" => 1.0,
        _ => return None,
    };
    (n.is_finite() && n >= 0.0).then_some(n * per_unit)
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

fn corner(row: &ReefRow, which: &str) -> Option<GeoPoint> {
    let get = |axis: &str| {
        row.rest
            .get(&format!("{which}_{axis}"))?
            .as_str()?
            .trim()
            .parse::<f64>()
            .ok()
    };
    let (lat, lon) = (get("latitude")?, get("longitude")?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
        .then_some(GeoPoint { lat, lon })
}

/// Reads the reef table.
///
/// # Errors
/// [`StateMapError::Status`] for an HTTP failure and [`StateMapError::Parse`] for a body that is
/// not a JSON array of rows.
pub fn parse_reefs(response: &Response<'_>) -> Result<Reefs, StateMapError> {
    check(response)?;
    let rows: Vec<ReefRow> =
        serde_json::from_slice(response.body).map_err(|e| StateMapError::Parse(e.to_string()))?;
    let mut sites = Vec::new();
    let mut skipped = 0;
    for row in rows {
        let corners: Vec<GeoPoint> = ["northwest", "northeast", "southeast", "southwest"]
            .iter()
            .filter_map(|w| corner(&row, w))
            .collect();
        let (Some(name), false) = (clean(row.reef_name.clone()), corners.is_empty()) else {
            skipped += 1;
            continue;
        };
        let n = corners.len() as f64;
        let position = GeoPoint {
            lat: corners.iter().map(|c| c.lat).sum::<f64>() / n,
            lon: corners.iter().map(|c| c.lon).sum::<f64>() / n,
        };
        let min_depth_text = clean(row.minimum_depth.clone());
        let max_depth_text = clean(row.maximum_depth.clone());
        sites.push(ReefSite {
            name,
            position,
            corners,
            body_of_water: clean(row.body_of_water),
            location: clean(row.description_of_location),
            size: clean(row.size),
            min_depth_m: min_depth_text.as_deref().and_then(parse_depth_m),
            max_depth_m: max_depth_text.as_deref().and_then(parse_depth_m),
            min_depth_text,
            max_depth_text,
            comments: clean(row.comments),
        });
    }
    Ok(Reefs { sites, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_bytes;

    const IN_FIXTURE: &str = "in-lake-bathymetry-crooked-creek-lake-2026-11-17.geojson";
    const MN_FIXTURE: &str = "mn-lake-bathymetry-pine-01000100-2026-11-17.geojson";
    const NY_FIXTURE: &str = "ny-artificial-reefs-socrata-2026-11-17.json";

    fn ok(body: &[u8]) -> Response<'_> {
        Response::new(200, body)
    }

    fn pa_box() -> Bbox {
        Bbox {
            west: -86.31,
            south: 39.11,
            east: -86.28,
            north: 39.13,
        }
    }

    #[test]
    fn the_indiana_request_matches_the_verified_query() {
        let r = contour_request(ContourState::Indiana, pa_box(), 14, 0).unwrap();
        assert!(r.url.starts_with(IN_QUERY), "{}", r.url);
        for part in [
            "where=1%3D1",
            "geometry=-86.310000,39.110000,-86.280000,39.130000",
            "geometryType=esriGeometryEnvelope",
            "inSR=4326",
            "spatialRel=esriSpatialRelIntersects",
            "outSR=4326",
            "geometryPrecision=6",
            "orderByFields=objectid",
            "resultOffset=0",
            "resultRecordCount=2000",
            "f=geojson",
        ] {
            assert!(r.url.contains(part), "{part} in {}", r.url);
        }
    }

    #[test]
    fn the_minnesota_request_pages_and_generalises() {
        let r = contour_request(ContourState::Minnesota, pa_box(), 10, 2000).unwrap();
        assert!(r.url.starts_with(MN_QUERY), "{}", r.url);
        assert!(r.url.contains("resultOffset=2000"));
        assert!(!r.url.contains("orderByFields"));
        // One pixel at zoom 10: 360 / 256 / 1024 degrees.
        assert!(r.url.contains("maxAllowableOffset=0.00137329"), "{}", r.url);
        assert!(tolerance_deg(0) > tolerance_deg(10));
        assert!(tolerance_deg(255).is_finite() && tolerance_deg(255) > 0.0);
    }

    #[test]
    fn bad_boxes_are_refused() {
        for bad in [
            Bbox {
                west: f64::NAN,
                ..pa_box()
            },
            Bbox {
                west: -86.0,
                east: -87.0,
                ..pa_box()
            },
            Bbox {
                north: 91.0,
                ..pa_box()
            },
        ] {
            assert_eq!(
                contour_request(ContourState::Indiana, bad, 10, 0),
                Err(StateMapError::InvalidRequest("box"))
            );
        }
    }

    #[test]
    fn indiana_contours_are_feet_below_the_surface_in_metres() {
        let body = fixture_bytes("state-maps", IN_FIXTURE);
        let c = parse_contours(ContourState::Indiana, &ok(&body)).unwrap();
        assert_eq!(c.skipped, 0);
        assert!(!c.more);
        // 18 features; multi-part lines may add parts.
        assert!(c.lines.len() >= 18, "{}", c.lines.len());
        let mut feet: Vec<i32> = c
            .lines
            .iter()
            .map(|l| (l.line.value / M_PER_FT).round() as i32)
            .collect();
        feet.sort_unstable();
        feet.dedup();
        assert_eq!(feet, [2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22]);
        let first = &c.lines[0];
        assert!((first.line.value - 20.0 * M_PER_FT).abs() < 1e-9);
        assert_eq!(first.lake.as_deref(), Some("Crooked Creek Lake"));
        assert_eq!(first.index, Some(true));
        assert_eq!(first.survey, Some(UnixTime(1_466_035_200)));
        assert!(!first.gauge_adjusted);
        assert!(first.line.points.len() >= 2);
        assert!((first.line.points[0].lon - -86.29759).abs() < 1e-9);
        assert!((first.line.points[0].lat - 39.124_361).abs() < 1e-9);
    }

    #[test]
    fn minnesota_depths_are_negative_feet_per_dnr_dem_metadata() {
        let body = fixture_bytes("state-maps", MN_FIXTURE);
        let c = parse_contours(ContourState::Minnesota, &ok(&body)).unwrap();
        assert_eq!(c.lines.len(), 8);
        assert_eq!(c.skipped, 0);
        let mut feet: Vec<i32> = c
            .lines
            .iter()
            .map(|l| (l.line.value / M_PER_FT).round() as i32)
            .collect();
        feet.sort_unstable();
        feet.dedup();
        assert_eq!(feet, [0, 5, 10, 15, 20, 25]);
        assert!(c.lines.iter().all(|l| l.line.value >= 0.0));
        assert_eq!(c.lines[0].lake_id.as_deref(), Some("01000100"));
        assert_eq!(c.lines[0].lake.as_deref(), Some("Pine"));
        assert_eq!(c.lines[0].survey, None);
        assert_eq!(c.lines[0].index, None);
    }

    #[test]
    fn a_full_page_or_the_flag_says_more() {
        let body = br#"{"type":"FeatureCollection","exceededTransferLimit":true,"features":[]}"#;
        assert!(
            parse_contours(ContourState::Minnesota, &ok(body))
                .unwrap()
                .more
        );
        let one = br#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"LineString","coordinates":[[-93.0,46.0],[-93.1,46.1]]},
             "properties":{"depth":-5}}]}"#;
        let c = parse_contours(ContourState::Minnesota, &ok(one)).unwrap();
        assert!(!c.more);
        assert_eq!(c.lines.len(), 1);
    }

    #[test]
    fn bad_features_are_skipped_and_bad_bodies_are_errors() {
        let body = br#"{"features":[
            {"geometry":{"type":"LineString","coordinates":[[-86.0,39.0],[-86.1,39.1]]},"properties":{"contour":null}},
            {"geometry":{"type":"LineString","coordinates":[[-86.0,39.0]]},"properties":{"contour":4}},
            {"geometry":null,"properties":{"contour":4}},
            {"geometry":{"type":"MultiLineString","coordinates":[[[-86.0,39.0],[-86.1,39.1]],[[-86.2,39.2],[-86.3,39.3]]]},
             "properties":{"contour":4,"gauge_adjusted_level":720.5,"line_type":"INTERMEDIATE"}}]}"#;
        let c = parse_contours(ContourState::Indiana, &ok(body)).unwrap();
        assert_eq!(c.skipped, 3);
        assert_eq!(c.lines.len(), 2);
        assert!(c.lines.iter().all(|l| l.gauge_adjusted));
        assert_eq!(c.lines[0].index, Some(false));

        let err = br#"{"error":{"code":400,"message":"Unable to complete operation."}}"#;
        assert_eq!(
            parse_contours(ContourState::Indiana, &ok(err)),
            Err(StateMapError::Service {
                code: 400,
                message: "Unable to complete operation.".to_owned()
            })
        );
        assert!(matches!(
            parse_contours(ContourState::Indiana, &ok(b"<html/>")),
            Err(StateMapError::Parse(_))
        ));
        assert_eq!(
            parse_contours(ContourState::Indiana, &Response::new(503, b"")),
            Err(StateMapError::Status { status: 503 })
        );
    }

    #[test]
    fn the_reef_request_is_the_socrata_table() {
        assert_eq!(
            reefs_request().url,
            "https://data.ny.gov/resource/jsnj-ehv9.json?$limit=50"
        );
    }

    #[test]
    fn all_sixteen_reefs_parse_with_their_depths() {
        let body = fixture_bytes("state-maps", NY_FIXTURE);
        let r = parse_reefs(&ok(&body)).unwrap();
        assert_eq!(r.skipped, 0);
        assert_eq!(r.sites.len(), 16);
        let atlantic = &r.sites[0];
        assert_eq!(atlantic.name, "Atlantic Beach Reef");
        assert_eq!(atlantic.corners.len(), 4);
        assert!((atlantic.position.lat - f64::midpoint(40.533_666_67, 40.5255)).abs() < 1e-9);
        assert!(
            (atlantic.position.lon - f64::midpoint(-73.728_333_33, -73.706_666_67)).abs() < 1e-9
        );
        assert_eq!(atlantic.body_of_water.as_deref(), Some("Atlantic Ocean"));
        assert_eq!(atlantic.size.as_deref(), Some("413 acres"));
        assert_eq!(atlantic.min_depth_text.as_deref(), Some("55 feet"));
        assert!((atlantic.min_depth_m.unwrap() - 55.0 * M_PER_FT).abs() < 1e-9);
        assert!((atlantic.max_depth_m.unwrap() - 64.0 * M_PER_FT).abs() < 1e-9);
        assert_eq!(atlantic.comments, None);
        // "32 Feet" has a capital letter in the source.
        let rockaway = r.sites.iter().find(|s| s.name == "Rockaway Reef").unwrap();
        assert!((rockaway.min_depth_m.unwrap() - 32.0 * M_PER_FT).abs() < 1e-9);
        assert_eq!(
            rockaway.comments.as_deref(),
            Some("Fish pots banned by state law")
        );
        assert!(
            r.sites
                .iter()
                .all(|s| s.min_depth_m.is_some() && s.max_depth_m.is_some())
        );
    }

    #[test]
    fn depth_text_is_read_or_left_unknown() {
        assert_eq!(parse_depth_m("55 feet"), Some(55.0 * M_PER_FT));
        assert_eq!(parse_depth_m(" 32 Feet "), Some(32.0 * M_PER_FT));
        assert_eq!(parse_depth_m("16 ft."), Some(16.0 * M_PER_FT));
        assert_eq!(parse_depth_m("1,200 feet"), Some(1200.0 * M_PER_FT));
        assert_eq!(parse_depth_m("16 fathoms"), Some(16.0 * M_PER_FATHOM));
        assert_eq!(parse_depth_m("12.5 metres"), Some(12.5));
        for unknown in ["", "deep", "55", "about 55 feet", "55 furlongs", "-5 feet"] {
            assert_eq!(parse_depth_m(unknown), None, "{unknown:?}");
        }
    }

    #[test]
    fn unreadable_reef_rows_are_skipped_and_depths_stay_unknown() {
        let body = br#"[
            {"reef_name":"A Reef","northwest_latitude":"40.5","northwest_longitude":"-73.7",
             "minimum_depth":"deep","maximum_depth":"60"},
            {"reef_name":"No Corners"},
            {"northwest_latitude":"40.5","northwest_longitude":"-73.7"}]"#;
        let r = parse_reefs(&ok(body)).unwrap();
        assert_eq!(r.skipped, 2);
        assert_eq!(r.sites.len(), 1);
        let a = &r.sites[0];
        assert_eq!(a.min_depth_m, None);
        assert_eq!(a.min_depth_text.as_deref(), Some("deep"));
        assert_eq!(a.max_depth_m, None);
        assert_eq!(a.max_depth_text.as_deref(), Some("60"));
        assert_eq!(a.corners.len(), 1);
        assert!(matches!(
            parse_reefs(&ok(b"{}")),
            Err(StateMapError::Parse(_))
        ));
    }
}
