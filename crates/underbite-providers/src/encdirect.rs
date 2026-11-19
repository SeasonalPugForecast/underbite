//! NOAA ENC Direct to GIS: chart structure points (wrecks, obstructions, rocks) by box (sans-IO).
//!
//! ENC Direct is an `ArcGIS` Server with one `MapServer` per chart scale band
//! (`enc_coastal`, `enc_approach`, `enc_harbour`) and one queryable layer per S-57 class and
//! geometry. **The layer ids differ per band** (harbour wrecks are 36, approach 39, coastal 33), so
//! they are never written down here: [`layers_request`] asks a band for its layer list at run time
//! and [`parse_layers`] finds the point layers by name; the host caches the answer
//! ([`LAYER_LIST_TTL_S`]). [`points_request`] then asks one layer for the points in a box, a page
//! of [`PAGE_SIZE`] at a time, and [`parse_points`] reads them into [`StructurePoint`]s.
//!
//! The service is intermittent in a way no status code shows: about one request in four answers
//! HTTP 200 with a `Could not find service` error body (or an HTML page from the web adaptor),
//! and the same request succeeds a moment later. Both parsers report that as
//! [`EncError::Transient`], and [`retry_after`] says whether and when to ask again (once for a bad
//! body, twice for 429 and 503).
//!
//! **No depth is shown.** `VALSOU` has no stated unit and no stated vertical datum (`VERDAT` is
//! empty for the New York wrecks), so a point keeps it only as
//! [`StructurePoint::raw_valsou_unverified`]. `CATWRK` is cut to 30 characters by the service and
//! `CATOBS` to 25.
//!
//! Not for navigation. Credit: [`underbite_core::map::enc::ENC_DIRECT_CREDIT`].

use serde_json::Value;
use underbite_core::map::enc::{SourceDate, StructureKind, StructurePoint};
use underbite_core::map::{Bbox, GeoPoint};

use crate::request::{QueryBuilder, Response};

const HOST: &str = "https://gis.charttools.noaa.gov/arcgis/rest/services/encdirect";

/// The most points a layer returns in one page (`maxRecordCount`).
pub const PAGE_SIZE: u32 = 1000;

/// How long a band's layer list stays good: the ids change only when NOAA republishes the
/// service.
pub const LAYER_LIST_TTL_S: u64 = 86_400;

/// A chart scale band of ENC Direct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EncBand {
    /// 1:150,001 to 1:600,000.
    Coastal,
    /// 1:50,001 to 1:150,000.
    Approach,
    /// 1:5,000 to 1:50,000.
    Harbour,
}

impl EncBand {
    /// The band that suits a Web Mercator zoom, or none when the view is wider than the coastal
    /// band. A zoom-`z` pixel at the equator is 1:559,082,264 / 2^z of a 96 dpi screen, so zooms 13
    /// and deeper are the harbour band (1:68,000 at the equator, 1:52,000 at 40 degrees),
    /// 11 and 12 the approach band and 9 and 10 the coastal band. The overview, general and berthing
    /// bands were not checked and are not used.
    pub const fn for_zoom(zoom: u8) -> Option<Self> {
        match zoom {
            13.. => Some(Self::Harbour),
            11..=12 => Some(Self::Approach),
            9..=10 => Some(Self::Coastal),
            _ => None,
        }
    }

    /// The shallowest zoom any band serves.
    pub const MIN_ZOOM: u8 = 9;

    /// The `MapServer` name under the ENC Direct folder.
    pub const fn service(self) -> &'static str {
        match self {
            Self::Coastal => "enc_coastal",
            Self::Approach => "enc_approach",
            Self::Harbour => "enc_harbour",
        }
    }
}

/// One GET for the host to send. The URL carries no key or personal data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncRequest {
    pub url: String,
}

/// An ENC Direct request could not be built, or its response read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncError {
    /// The request asks for something that cannot be sent.
    #[error("invalid ENC Direct request: {0}")]
    InvalidRequest(&'static str),
    /// The service answered with an HTTP failure.
    #[error("ENC Direct answered HTTP {status}")]
    Status { status: u16 },
    /// HTTP 200 with a body that is a transient failure of the server (`Could not find service`,
    /// or an HTML error page): ask again.
    #[error("ENC Direct answered with a transient error body")]
    Transient,
    /// HTTP 200 with another `ArcGIS` error object.
    #[error("ENC Direct reported an error ({code}): {message}")]
    Service { code: i64, message: String },
    /// The body is not the expected JSON.
    #[error("the response is not the expected JSON: {0}")]
    Parse(String),
}

/// Seconds to wait before asking again, or none when to give up. `attempt` counts the requests
/// already made for this thing (0 after the first response): a transient body is retried once, a
/// 429 or 503 twice.
pub fn retry_after(error: &EncError, attempt: u32) -> Option<u32> {
    match error {
        EncError::Transient if attempt < 1 => Some(3),
        EncError::Status { status: 429 } if attempt < 2 => Some(30),
        EncError::Status { status: 503 } if attempt < 2 => Some(10),
        _ => None,
    }
}

/// The request for a band's layer list.
pub fn layers_request(band: EncBand) -> EncRequest {
    let base = format!("{HOST}/{}/MapServer", band.service());
    EncRequest {
        url: QueryBuilder::new(&base).param("f", "json").finish(),
    }
}

/// The ids of a band's point layers, as the band's own list gives them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BandLayers {
    pub wrecks: Option<u32>,
    pub obstructions: Option<u32>,
    pub rocks: Option<u32>,
}

impl BandLayers {
    /// The layer of a kind.
    pub const fn id(&self, kind: StructureKind) -> Option<u32> {
        match kind {
            StructureKind::Wreck => self.wrecks,
            StructureKind::Obstruction => self.obstructions,
            StructureKind::Rock => self.rocks,
        }
    }
}

fn check(response: &Response<'_>) -> Result<(), EncError> {
    match response.status {
        200..=299 => Ok(()),
        status => Err(EncError::Status { status }),
    }
}

/// The JSON of a 200 response, or the error the body is.
fn json(response: &Response<'_>) -> Result<Value, EncError> {
    check(response)?;
    let v: Value = match serde_json::from_slice(response.body) {
        Ok(v) => v,
        Err(e) => {
            // The web adaptor answers a failure with an HTML page and status 200.
            let head = response.body.iter().find(|b| !b.is_ascii_whitespace());
            return Err(if head == Some(&b'<') {
                EncError::Transient
            } else {
                EncError::Parse(e.to_string())
            });
        }
    };
    if let Some(e) = v.get("error") {
        let message = e.get("message").and_then(Value::as_str).unwrap_or_default();
        return Err(if message.contains("Could not find service") {
            EncError::Transient
        } else {
            EncError::Service {
                code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: message.to_owned(),
            }
        });
    }
    Ok(v)
}

/// Reads a band's layer list: the wreck, obstruction and rock point layers, found by the end of
/// their names (`Harbor.Wreck_point`, `Obstruction_point`, `Underwater_Awash_Rock_point`), which
/// carry a band prefix. A kind the band lacks is none.
///
/// # Errors
/// [`EncError::Status`], [`EncError::Transient`], [`EncError::Service`] or [`EncError::Parse`]
/// for a body without a `layers` list.
pub fn parse_layers(response: &Response<'_>) -> Result<BandLayers, EncError> {
    let v = json(response)?;
    let layers = v
        .get("layers")
        .and_then(Value::as_array)
        .ok_or_else(|| EncError::Parse("no layers".to_owned()))?;
    let mut out = BandLayers::default();
    for layer in layers {
        let (Some(id), Some(name)) = (
            layer
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|i| u32::try_from(i).ok()),
            layer.get("name").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        let slot = if name.ends_with("wreck_point") {
            &mut out.wrecks
        } else if name.ends_with("obstruction_point") {
            &mut out.obstructions
        } else if name.ends_with("rock_point") {
            &mut out.rocks
        } else {
            continue;
        };
        slot.get_or_insert(id);
    }
    Ok(out)
}

/// The `query` for the points of one layer inside a box, one page.
///
/// `layer` is an id from [`parse_layers`] for the same band; `offset` is the number of features to
/// skip (0 for the first page, then `offset + PAGE_SIZE` while [`StructurePage::more`] is set).
/// The answer is `JSON` with every field (`outFields=*`) and positions in WGS 84 degrees; the box
/// is rounded outward to two decimals, so it never pins down a place finer than a spot's.
///
/// # Errors
/// [`EncError::InvalidRequest`] for a box that is not finite, outside the world or inverted.
pub fn points_request(
    band: EncBand,
    layer: u32,
    bbox: Bbox,
    offset: u32,
) -> Result<EncRequest, EncError> {
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
        return Err(EncError::InvalidRequest("box"));
    }
    let bbox = crate::species::request_box(bbox).ok_or(EncError::InvalidRequest("box"))?;
    let base = format!("{HOST}/{}/MapServer/{layer}/query", band.service());
    let url = QueryBuilder::new(&base)
        .param("f", "json")
        .param(
            "geometry",
            [bbox.west, bbox.south, bbox.east, bbox.north]
                .map(crate::species::fmt_degrees)
                .join(","),
        )
        .param("geometryType", "esriGeometryEnvelope")
        .param("inSR", "4326")
        .param("spatialRel", "esriSpatialRelIntersects")
        .param("outFields", "*")
        .param("returnGeometry", "true")
        .param("outSR", "4326")
        .param("orderByFields", "OBJECTID")
        .param("resultOffset", offset.to_string())
        .param("resultRecordCount", PAGE_SIZE.to_string())
        .finish();
    Ok(EncRequest { url })
}

/// The points one page returned.
#[derive(Clone, Debug, PartialEq)]
pub struct StructurePage {
    pub points: Vec<StructurePoint>,
    /// Features left out for a missing id or a position that is not a valid point.
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

fn number(props: &Value, key: &str) -> Option<f64> {
    props.get(key)?.as_f64().filter(|v| v.is_finite())
}

/// A feature's attributes and position, in either the `JSON` shape (`attributes` and a geometry
/// of `x` and `y`) or the `GeoJSON` one (`properties` and a `Point`).
fn feature_parts(f: &Value) -> (&Value, Option<GeoPoint>) {
    let (props, lon, lat) = if let Some(attributes) = f.get("attributes") {
        let g = f.get("geometry").unwrap_or(&Value::Null);
        (
            attributes,
            g.get("x").and_then(Value::as_f64),
            g.get("y").and_then(Value::as_f64),
        )
    } else {
        let c = f.pointer("/geometry/coordinates").and_then(Value::as_array);
        (
            f.get("properties").unwrap_or(&Value::Null),
            c.and_then(|c| c.first()).and_then(Value::as_f64),
            c.and_then(|c| c.get(1)).and_then(Value::as_f64),
        )
    };
    let position = match (lat, lon) {
        (Some(lat), Some(lon))
            if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) =>
        {
            Some(GeoPoint { lat, lon })
        }
        _ => None,
    };
    (props, position)
}

/// Reads one page of a point layer of `kind`.
///
/// # Errors
/// [`EncError::Status`] for an HTTP failure, [`EncError::Transient`] for the server's transient
/// bodies, [`EncError::Service`] for another `ArcGIS` error and [`EncError::Parse`] for a body that
/// holds no feature list.
pub fn parse_points(
    kind: StructureKind,
    response: &Response<'_>,
) -> Result<StructurePage, EncError> {
    let v = json(response)?;
    let features = v
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| EncError::Parse("no features".to_owned()))?;
    let flagged = v.get("exceededTransferLimit").and_then(Value::as_bool) == Some(true);
    let category_field = match kind {
        StructureKind::Wreck => Some("CATWRK"),
        StructureKind::Obstruction => Some("CATOBS"),
        StructureKind::Rock => None,
    };
    let mut points = Vec::new();
    let mut skipped = 0;
    for f in features {
        let (props, position) = feature_parts(f);
        let object_id = number(props, "OBJECTID")
            .filter(|n| *n >= 0.0 && n.fract() == 0.0)
            .map(|n| n as u64);
        let (Some(object_id), Some(position)) = (object_id, position) else {
            skipped += 1;
            continue;
        };
        let category = category_field.and_then(|k| text(props, k));
        let category_may_be_cut = match (&category, kind.category_limit()) {
            (Some(c), Some(limit)) => c.chars().count() >= limit,
            _ => false,
        };
        points.push(StructurePoint {
            object_id,
            kind,
            category,
            category_may_be_cut,
            name: text(props, "OBJNAM"),
            source_date: text(props, "SORDAT").and_then(|s| SourceDate::parse(&s)),
            position,
            chart_cell: text(props, "DSNM"),
            raw_valsou_unverified: number(props, "VALSOU"),
        });
    }
    Ok(StructurePage {
        points,
        skipped,
        more: flagged || features.len() >= PAGE_SIZE as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRECKS_LOWER_BAY: &[u8] = include_bytes!(
        "../../../tests/fixtures/raw/ncds/encdirect-enc-harbour-wreck-point-query-lower-bay-2026-11-17.json"
    );
    const WRECKS_GEOJSON: &[u8] = include_bytes!(
        "../../../tests/fixtures/raw/ncds/encdirect-enc-harbour-wreck-point-query-lower-bay-2026-11-17.geojson"
    );
    const WRECKS_SMALL_BOX: &[u8] = include_bytes!(
        "../../../tests/fixtures/raw/ncds/encdirect-enc-harbour-wreck-point-query-lower-bay-small-box-2026-11-17.json"
    );
    const OBSTRUCTIONS: &[u8] = include_bytes!(
        "../../../tests/fixtures/raw/ncds/encdirect-enc-harbour-obstruction-point-query-bight-2026-11-17.json"
    );
    const COASTAL_WRECKS: &[u8] = include_bytes!(
        "../../../tests/fixtures/raw/ncds/encdirect-enc-coastal-wreck-point-query-bight-2026-11-17.json"
    );
    const APPROACH_EMPTY: &[u8] = include_bytes!(
        "../../../tests/fixtures/raw/ncds/encdirect-enc-approach-wreck-point-query-offshore-empty-2026-11-17.json"
    );
    const HARBOUR_WRECK_LAYER: &[u8] = include_bytes!(
        "../../../tests/fixtures/raw/ncds/encdirect-enc-harbour-wreck-point-layer-2026-11-17.json"
    );
    const TRANSIENT: &[u8] = br#"{"error":{"code":500,"message":"Error handling service request :Could not find service. Service may be stopped or it may not be configured.","details":[]}}"#;

    fn ok(body: &[u8]) -> Response<'_> {
        Response::new(200, body)
    }

    const BIGHT: Bbox = Bbox {
        west: -74.20,
        south: 40.35,
        east: -73.70,
        north: 40.75,
    };

    #[test]
    fn bands_follow_the_zoom() {
        assert_eq!(EncBand::for_zoom(8), None);
        assert_eq!(EncBand::for_zoom(EncBand::MIN_ZOOM), Some(EncBand::Coastal));
        assert_eq!(EncBand::for_zoom(10), Some(EncBand::Coastal));
        assert_eq!(EncBand::for_zoom(11), Some(EncBand::Approach));
        assert_eq!(EncBand::for_zoom(12), Some(EncBand::Approach));
        assert_eq!(EncBand::for_zoom(13), Some(EncBand::Harbour));
        assert_eq!(EncBand::for_zoom(20), Some(EncBand::Harbour));
        assert_eq!(EncBand::for_zoom(EncBand::MIN_ZOOM - 1), None);
    }

    #[test]
    fn the_layer_list_request_names_the_band() {
        assert_eq!(
            layers_request(EncBand::Approach).url,
            "https://gis.charttools.noaa.gov/arcgis/rest/services/encdirect/enc_approach/MapServer?f=json"
        );
    }

    /// The ids the scout recorded per band (harbour 33, 34, 36; approach 36, 37, 39; coastal 30,
    /// 31, 33), in lists shaped like the service's. The ids exist only in these tests.
    #[test]
    fn layer_ids_are_read_per_band() {
        let harbour = br#"{"layers":[{"id":30,"name":"DangersP","subLayerIds":[33,34,36]},
            {"id":32,"name":"Harbor.Fishing_Facility_point"},{"id":33,"name":"Harbor.Obstruction_point"},
            {"id":34,"name":"Harbor.Underwater_Awash_Rock_point"},{"id":36,"name":"Harbor.Wreck_point"},
            {"id":99,"name":"Harbor.Obstruction_line"},{"id":158,"name":"Harbor.Wreck_area"}]}"#;
        let approach = br#"{"layers":[{"id":35,"name":"Approach.Fishing_Facility_point"},
            {"id":36,"name":"Approach.Obstruction_point"},{"id":37,"name":"Approach.Underwater_Awash_Rock_point"},
            {"id":39,"name":"Approach.Wreck_point"}]}"#;
        let coastal = br#"{"layers":[{"id":30,"name":"Coastal.Obstruction_point"},
            {"id":31,"name":"Coastal.Underwater_Awash_Rock_point"},{"id":33,"name":"Coastal.Wreck_point"}]}"#;
        let h = parse_layers(&ok(harbour)).unwrap();
        assert_eq!(
            (h.wrecks, h.obstructions, h.rocks),
            (Some(36), Some(33), Some(34))
        );
        let a = parse_layers(&ok(approach)).unwrap();
        assert_eq!(
            (a.wrecks, a.obstructions, a.rocks),
            (Some(39), Some(36), Some(37))
        );
        let c = parse_layers(&ok(coastal)).unwrap();
        assert_eq!(c.id(StructureKind::Wreck), Some(33));
        assert_eq!(c.id(StructureKind::Obstruction), Some(30));
        assert_eq!(c.id(StructureKind::Rock), Some(31));
        assert_eq!(
            parse_layers(&ok(br#"{"layers":[]}"#)).unwrap(),
            BandLayers::default()
        );
        assert!(matches!(
            parse_layers(&ok(br#"{"x":1}"#)),
            Err(EncError::Parse(_))
        ));
    }

    #[test]
    fn the_recorded_layer_metadata_names_a_wreck_layer() {
        // The layer's own metadata (not the list) carries the name the list matches on.
        let v: Value = serde_json::from_slice(HARBOUR_WRECK_LAYER).unwrap();
        assert_eq!(v["id"], 36);
        let list = format!(
            r#"{{"layers":[{{"id":{},"name":{}}}]}}"#,
            v["id"], v["name"]
        );
        let l = parse_layers(&ok(list.as_bytes())).unwrap();
        assert_eq!(l.wrecks, Some(36));
    }

    #[test]
    fn the_points_request_is_an_envelope_query_in_degrees() {
        let r = points_request(EncBand::Harbour, 36, BIGHT, 1000).unwrap();
        assert_eq!(
            r.url,
            "https://gis.charttools.noaa.gov/arcgis/rest/services/encdirect/enc_harbour/MapServer/36/query?f=json&geometry=-74.20,40.35,-73.70,40.75&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=%2A&returnGeometry=true&outSR=4326&orderByFields=OBJECTID&resultOffset=1000&resultRecordCount=1000"
        );
        for bad in [
            Bbox {
                west: 1.0,
                east: 0.0,
                ..BIGHT
            },
            Bbox {
                north: f64::NAN,
                ..BIGHT
            },
            Bbox {
                south: -91.0,
                ..BIGHT
            },
        ] {
            assert_eq!(
                points_request(EncBand::Coastal, 33, bad, 0),
                Err(EncError::InvalidRequest("box"))
            );
        }
    }

    #[test]
    fn recorded_wrecks_parse() {
        let page = parse_points(StructureKind::Wreck, &ok(WRECKS_SMALL_BOX)).unwrap();
        assert_eq!(page.points.len(), 3);
        assert!(!page.more);
        let p = page
            .points
            .iter()
            .find(|p| p.object_id == 81133)
            .expect("the wreck of the cross-check");
        assert_eq!(p.kind, StructureKind::Wreck);
        assert_eq!(p.chart_cell.as_deref(), Some("US5NY1BE.000"));
        assert_eq!(
            p.source_date,
            Some(SourceDate {
                year: 2013,
                month: Some(12),
                day: Some(20)
            })
        );
        assert!((p.position.lon + 73.921_896_3).abs() < 1e-6);
        assert!((p.position.lat - 40.600_561_5).abs() < 1e-6);
        assert_eq!(p.raw_valsou_unverified, Some(2.6));
        assert!(p.category.is_some());

        let page = parse_points(StructureKind::Wreck, &ok(WRECKS_LOWER_BAY)).unwrap();
        assert!(page.points.len() > 1);
        assert!(page.more, "the query was limited to five features");
    }

    #[test]
    fn the_geojson_shape_parses_too() {
        let page = parse_points(StructureKind::Wreck, &ok(WRECKS_GEOJSON)).unwrap();
        assert_eq!(page.points.len(), 3);
        assert!(page.more, "exceededTransferLimit is set");
        let p = &page.points[0];
        assert_eq!(p.object_id, 81820);
        assert_eq!(p.category.as_deref(), Some("dangerous wreck"));
        assert!(!p.category_may_be_cut);
        assert_eq!(p.raw_valsou_unverified, Some(1.2));
        assert!((p.position.lon + 74.096_025).abs() < 1e-6);
    }

    #[test]
    fn obstructions_without_a_depth_or_name_parse() {
        let page = parse_points(StructureKind::Obstruction, &ok(OBSTRUCTIONS)).unwrap();
        assert!(page.points.len() > 1);
        let p = &page.points[0];
        assert_eq!(p.kind, StructureKind::Obstruction);
        assert_eq!(p.category.as_deref(), Some("snag/stump"));
        assert_eq!(p.raw_valsou_unverified, None);
        assert_eq!(p.name, None);
        assert_eq!(
            p.source_date,
            Some(SourceDate {
                year: 2002,
                month: Some(5),
                day: None
            })
        );
        assert!(!p.is_fish_haven());
    }

    #[test]
    fn coastal_wrecks_and_an_empty_band() {
        let page = parse_points(StructureKind::Wreck, &ok(COASTAL_WRECKS)).unwrap();
        assert_eq!(page.points[0].raw_valsou_unverified, Some(20.1));
        assert_eq!(
            page.points[1].category.as_deref(),
            Some("non-dangerous wreck (test)")
        );
        let empty = parse_points(StructureKind::Wreck, &ok(APPROACH_EMPTY)).unwrap();
        assert!(empty.points.is_empty() && !empty.more);
    }

    #[test]
    fn a_cut_category_is_flagged_and_a_rock_has_none() {
        let body = br#"{"features":[
            {"attributes":{"OBJECTID":1,"CATWRK":"wreck showing any portion of h","VALSOU":null},"geometry":{"x":-74.0,"y":40.5}},
            {"attributes":{"OBJECTID":2,"CATWRK":"  ","SORDAT":"2013"},"geometry":{"x":-74.0,"y":40.5}},
            {"attributes":{"OBJECTID":3},"geometry":{"x":-200.0,"y":40.5}},
            {"attributes":{"CATWRK":"x"},"geometry":{"x":-74.0,"y":40.5}},
            {"attributes":{"OBJECTID":5},"geometry":{}}]}"#;
        let page = parse_points(StructureKind::Wreck, &ok(body)).unwrap();
        assert_eq!(page.points.len(), 2);
        assert_eq!(page.skipped, 3);
        assert!(page.points[0].category_may_be_cut);
        assert_eq!(page.points[0].raw_valsou_unverified, None);
        assert_eq!(page.points[1].category, None);
        let rock = br#"{"features":[{"attributes":{"OBJECTID":9,"CATWRK":"ignored","VALSOU":0.3,"SORDAT":"19990401"},"geometry":{"x":-74.0,"y":40.5}}]}"#;
        let r = parse_points(StructureKind::Rock, &ok(rock)).unwrap();
        assert_eq!(r.points[0].category, None);
        assert_eq!(r.points[0].kind, StructureKind::Rock);
    }

    #[test]
    fn a_full_page_may_have_more() {
        let one = r#"{"attributes":{"OBJECTID":1},"geometry":{"x":-74.0,"y":40.5}}"#;
        let body = format!(
            r#"{{"features":[{}]}}"#,
            vec![one; PAGE_SIZE as usize].join(",")
        );
        assert!(
            parse_points(StructureKind::Wreck, &ok(body.as_bytes()))
                .unwrap()
                .more
        );
    }

    #[test]
    fn the_transient_bodies_are_retried_once() {
        for body in [
            TRANSIENT,
            b"<html><body>Application Error</body></html>".as_slice(),
            b"\n  <!DOCTYPE html>".as_slice(),
        ] {
            let e = parse_points(StructureKind::Wreck, &ok(body)).unwrap_err();
            assert_eq!(e, EncError::Transient);
            assert_eq!(retry_after(&e, 0), Some(3));
            assert_eq!(retry_after(&e, 1), None);
        }
        assert_eq!(
            parse_layers(&ok(TRANSIENT)).unwrap_err(),
            EncError::Transient
        );
    }

    #[test]
    fn other_failures() {
        let permanent =
            br#"{"error":{"code":400,"message":"Unable to complete operation.","details":[]}}"#;
        let e = parse_points(StructureKind::Wreck, &ok(permanent)).unwrap_err();
        assert!(matches!(e, EncError::Service { code: 400, .. }));
        assert_eq!(retry_after(&e, 0), None);
        let busy = |status| parse_layers(&Response::new(status, b"")).unwrap_err();
        assert_eq!(retry_after(&busy(503), 1), Some(10));
        assert_eq!(retry_after(&busy(503), 2), None);
        assert_eq!(retry_after(&busy(429), 0), Some(30));
        assert_eq!(retry_after(&busy(404), 0), None);
        assert!(matches!(
            parse_points(StructureKind::Wreck, &ok(b"not json")),
            Err(EncError::Parse(_))
        ));
        assert!(matches!(
            parse_points(StructureKind::Wreck, &ok(b"{}")),
            Err(EncError::Parse(_))
        ));
    }
}
