//! U.S. Fish and Wildlife Service National Wetlands Inventory: the wetland codes around a point,
//! which say whether the water there is tidal.
//!
//! The service is an `ArcGIS` REST `MapServer` (layer 0, `Wetlands`, polygons). This module plans
//! the `query` request and parses the Esri JSON into the codes of
//! [`underbite_core::discover::tidal::Wetlands`], whose rule says whether they are tidal. It is
//! not a [`Source`](crate::source::Source): the answer is not a snapshot fragment. The host runs
//! the [`NwiQuery`] it gets from [`tidal_query`] and hands the response to [`parse`].
//!
//! - The request is an envelope of the point rounded by the context plus or minus 0.01 degree
//!   (more when the rounding cell is coarser than 0.01), rounded outward to two decimals, with
//!   no geometry and only the two fields the rule reads. Only that box leaves the device.
//! - The where clause is `1=1`: the service's firewall answers an `OR` with an HTML page, and a
//!   page that is not JSON is an error here, like an error object under HTTP 200.
//! - A truncated answer (`exceededTransferLimit`) is kept as such; the rule decides.

use serde::Deserialize;
use underbite_core::discover::tidal::Wetlands;
use underbite_core::map::Bbox;
use underbite_core::provenance::{Attribution, SourceId};

use crate::request::{Coordinates, Response};
use crate::source::{FetchContext, SourceError};
use crate::species::{fmt_degrees, request_box};

/// The source id of the inventory.
pub const SOURCE: SourceId = SourceId::from_static("usfws-nwi");

/// Query endpoint of the wetlands layer.
pub const BASE_URL: &str =
    "https://fwspublicservices.wim.usgs.gov/wetlandsmapservice/rest/services/Wetlands/MapServer/0";

/// Half the side of the box around the rounded point, degrees.
pub const HALF_SIDE_DEG: f64 = 0.01;

/// How long the host may reuse a response, seconds: the inventory is updated twice a year.
pub const MAX_AGE_S: u32 = 30 * 24 * 3600;

/// The credit every display of this data shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SOURCE,
        credit: "U.S. Fish and Wildlife Service, National Wetlands Inventory (2026), \
                 doi:10.7944/usfws.nwi.202605. Not for regulatory use."
            .to_string(),
        url: Some("https://doi.org/10.7944/USFWS.NWI".to_string()),
    }
}

/// One planned GET of the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NwiQuery {
    /// The full URL.
    pub url: String,
    /// How long the host may reuse the response, seconds.
    pub max_age_s: u32,
}

/// The wetland codes in a box around `spot`: the context-rounded point plus or minus
/// [`HALF_SIDE_DEG`] (plus the part of the rounding cell beyond 0.005 degree, so the box covers
/// the spot at any `coordinate_precision`), rounded outward to two decimals.
pub fn tidal_query(spot: Coordinates, ctx: &FetchContext) -> Result<NwiQuery, SourceError> {
    let centre = ctx.round(spot).coordinates();
    let (lat, lon) = (centre.latitude, centre.longitude);
    if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
        return Err(SourceError::Unsupported(format!(
            "coordinates out of range: {lat}, {lon}"
        )));
    }
    let cell_half = 0.5 * 10f64.powi(-i32::from(ctx.coordinate_precision()));
    let half = HALF_SIDE_DEG + (cell_half - 0.005).max(0.0);
    let sent = request_box(Bbox {
        west: (lon - half).max(-180.0),
        south: (lat - half).max(-90.0),
        east: (lon + half).min(180.0),
        north: (lat + half).min(90.0),
    })
    .ok_or_else(|| SourceError::Unsupported("envelope out of range".to_owned()))?;
    let [xmin, ymin, xmax, ymax] = [sent.west, sent.south, sent.east, sent.north].map(fmt_degrees);
    Ok(NwiQuery {
        url: format!(
            "{BASE_URL}/query?where=1=1&geometry={xmin},{ymin},{xmax},{ymax}\
&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects\
&outFields=ATTRIBUTE,WETLAND_TYPE&returnGeometry=false&f=json"
        ),
        max_age_s: MAX_AGE_S,
    })
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
}

#[derive(Deserialize)]
struct RawFeature {
    attributes: Option<RawAttributes>,
}

#[derive(Deserialize)]
struct RawAttributes {
    #[serde(rename = "ATTRIBUTE")]
    attribute: Option<String>,
}

/// Parses an answer into the polygons' codes. A status other than 2xx, an `error` object under
/// HTTP 200, and a body that is not JSON (the firewall's HTML page) are errors; an empty
/// `features` list is not (it gives no codes).
pub fn parse(response: &Response<'_>) -> Result<Wetlands, SourceError> {
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
    let body = response.body.trim_ascii();
    if body.is_empty() {
        return Err(SourceError::NoData(Some("empty response".to_string())));
    }
    if body.first() != Some(&b'{') {
        return Err(SourceError::Parse(
            "the wetlands service did not answer with JSON".to_string(),
        ));
    }
    let raw: RawPage = serde_json::from_slice(body)
        .map_err(|e| SourceError::Parse(format!("invalid wetlands JSON: {e}")))?;
    if let Some(error) = raw.error {
        return Err(SourceError::Status {
            status: error.code.unwrap_or(response.status),
            message: error.message.filter(|m| !m.is_empty()),
        });
    }
    Ok(Wetlands {
        codes: raw
            .features
            .into_iter()
            .filter_map(|f| f.attributes?.attribute)
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty())
            .collect(),
        exceeded_transfer_limit: raw.exceeded == Some(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydro::envelope_from_url;
    use crate::testutil::fixture_bytes;
    use underbite_core::discover::tidal::Tide;
    use underbite_core::time::UnixTime;

    fn ctx() -> FetchContext {
        FetchContext::new(UnixTime(1_794_000_000), "underbite/test")
    }

    fn tide(name: &str) -> Tide {
        let body = fixture_bytes("tidal-limit", name);
        parse(&Response::new(200, &body)).unwrap().tide()
    }

    const TIDAL_RIVER: Tide = Tide::Tidal { estuarine: false };

    #[test]
    fn the_lower_hudson_is_estuarine() {
        assert_eq!(
            tide("nwi-0-battery-box.json"),
            Tide::Tidal { estuarine: true }
        );
    }

    #[test]
    fn poughkeepsie_albany_and_trenton_are_a_tidal_river() {
        assert_eq!(tide("nwi-0-poughkeepsie-box.json"), TIDAL_RIVER);
        assert_eq!(tide("nwi-0-albany-box.json"), TIDAL_RIVER);
        assert_eq!(tide("nwi-0-trenton-box.json"), TIDAL_RIVER);
    }

    #[test]
    fn the_croton_gauge_is_not_tidal_but_its_mouth_is() {
        assert_eq!(tide("nwi-0-croton-box.json"), Tide::NotTidal);
        assert_eq!(
            tide("nwi-0-croton-mouth-box.json"),
            Tide::Tidal { estuarine: true }
        );
    }

    #[test]
    fn the_hudson_is_tidal_below_the_troy_dam_and_not_above_it() {
        assert_eq!(tide("nwi-0-troy-below-box.json"), TIDAL_RIVER);
        assert_eq!(tide("nwi-0-troy-above-box.json"), Tide::NotTidal);
    }

    #[test]
    fn the_filtered_point_query_answer_reads_too() {
        assert_eq!(tide("nwi-0-albany-point-distance500-r1.json"), TIDAL_RIVER);
    }

    #[test]
    fn an_empty_answer_is_unknown() {
        let body = br#"{"displayFieldName":"x","features":[]}"#;
        let wetlands = parse(&Response::new(200, body)).unwrap();
        assert_eq!(wetlands.codes.len(), 0);
        assert_eq!(wetlands.tide(), Tide::Unknown);
    }

    #[test]
    fn a_truncated_answer_without_a_tidal_code_is_unknown() {
        let body = br#"{"exceededTransferLimit":true,"features":[
            {"attributes":{"ATTRIBUTE":"R3UBH"}},{"attributes":{"ATTRIBUTE":null}},{}]}"#;
        let wetlands = parse(&Response::new(200, body)).unwrap();
        assert_eq!(wetlands.codes, ["R3UBH"]);
        assert!(wetlands.exceeded_transfer_limit);
        assert_eq!(wetlands.tide(), Tide::Unknown);
        let body = br#"{"exceededTransferLimit":true,"features":[
            {"attributes":{"ATTRIBUTE":"E1UBL"}}]}"#;
        assert_eq!(
            parse(&Response::new(200, body)).unwrap().tide(),
            Tide::Tidal { estuarine: true }
        );
    }

    #[test]
    fn the_firewall_page_is_an_error() {
        let body = b"<html><head><title>Request Rejected</title></head><body>The URL you \
                     requested has been blocked.</body></html>";
        let err = parse(&Response::new(200, body)).unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)), "{err}");
        let err = parse(&Response::new(403, body)).unwrap_err();
        assert!(
            matches!(err, SourceError::Status { status: 403, .. }),
            "{err}"
        );
    }

    #[test]
    fn an_error_object_under_http_200_is_an_error() {
        let body =
            br#"{"error":{"code":400,"message":"Unable to complete operation.","details":[]}}"#;
        let err = parse(&Response::new(200, body)).unwrap_err();
        assert!(
            matches!(err, SourceError::Status { status: 400, .. }),
            "{err}"
        );
        assert!(matches!(
            parse(&Response::new(200, b"")).unwrap_err(),
            SourceError::NoData(_)
        ));
        assert!(matches!(
            parse(&Response::new(429, b"")).unwrap_err(),
            SourceError::RateLimited { .. }
        ));
    }

    fn at(latitude: f64, longitude: f64) -> Coordinates {
        Coordinates {
            latitude,
            longitude,
        }
    }

    #[test]
    fn the_box_is_the_rounded_point_plus_or_minus_a_hundredth_of_a_degree() {
        // Albany: the exact tap rounds to 42.65,-73.75.
        let q = tidal_query(at(42.6512, -73.7488), &ctx()).unwrap();
        assert_eq!(
            envelope_from_url(&q.url),
            Some([-73.76, 42.64, -73.74, 42.66])
        );
        // Poughkeepsie: the box of the recorded fixture.
        let q = tidal_query(at(41.7003, -73.9399), &ctx()).unwrap();
        assert_eq!(
            envelope_from_url(&q.url),
            Some([-73.95, 41.69, -73.93, 41.71])
        );
    }

    #[test]
    fn nothing_but_the_rounded_box_leaves_the_device() {
        let tap = at(42.651_234_5, -73.748_876_5);
        let q = tidal_query(tap, &ctx()).unwrap();
        for exact in ["42.6512", "73.7488", "42.651234", "73.748876"] {
            assert!(!q.url.contains(exact), "{}", q.url);
        }
        // Every number in the geometry has two decimals.
        let geometry = q
            .url
            .split("geometry=")
            .nth(1)
            .and_then(|r| r.split('&').next())
            .unwrap();
        for n in geometry.split(',') {
            assert_eq!(n.split_once('.').map(|(_, f)| f.len()), Some(2), "{n}");
        }
        assert!(q.url.starts_with(BASE_URL));
        assert!(q.url.contains("where=1=1"));
        assert!(!q.url.to_ascii_uppercase().contains(" OR "));
        assert!(q.url.contains("returnGeometry=false"));
        assert!(q.url.contains("outFields=ATTRIBUTE,WETLAND_TYPE"));
    }

    #[test]
    fn a_coarser_rounding_widens_the_box_to_cover_the_point() {
        let ctx = ctx().with_coordinate_precision(1);
        let tap = at(42.6951, -73.7949);
        let q = tidal_query(tap, &ctx).unwrap();
        let [xmin, ymin, xmax, ymax] = envelope_from_url(&q.url).unwrap();
        assert!(xmin <= tap.longitude && tap.longitude <= xmax, "{}", q.url);
        assert!(ymin <= tap.latitude && tap.latitude <= ymax, "{}", q.url);
    }

    #[test]
    fn a_point_out_of_range_is_refused() {
        assert!(tidal_query(at(95.0, 0.0), &ctx()).is_err());
        assert!(tidal_query(at(f64::NAN, 0.0), &ctx()).is_err());
    }
}
