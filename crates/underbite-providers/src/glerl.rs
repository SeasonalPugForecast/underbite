//! NOAA GLERL Great Lakes Surface Environmental Analysis (GLSEA) water temperature.
//!
//! The daily satellite analysis `GLSEA_ACSPO_GCS` on GLERL's ERDDAP server covers the Great Lakes
//! on a 0.014° grid. [`request`] asks for a small box of the newest day around a spot, through the
//! ERDDAP builder of [`crate::erddap`] (`GridHost::Glerl`); [`parse`] reads the table with that
//! module's parser and takes the nearest cell that holds a value, as a one-sample series of kind
//! [`ValueKind::Satellite`] with [`Relation::Satellite`] and the distance to the cell.
//!
//! The dataset is a lake mask: land and cells the analysis could not fill are `null`. A spot on
//! an inland lake inside the coverage box but outside the Great Lakes finds only `null` cells (or
//! a Great Lakes cell farther than [`MAX_CELL_DISTANCE_M`]) and gets no value, never the nearest
//! lake's. Datasets are keyed by id, never by title.

use underbite_core::map::grid::Grid;
use underbite_core::map::tiles::haversine_m;
use underbite_core::map::{Bbox, GeoPoint, GridHost, GridSource};
use underbite_core::provenance::{Attribution, Provenance, Quality, Relation, SourceId, ValueKind};
use underbite_core::series::{Sample, Series};

use crate::erddap::{
    ErddapError, ErddapRequest, GridTime, check_response as check_erddap, grid_request, parse_grid,
};
use crate::request::{Response, RoundedPoint};
use crate::source::SourceError;

/// The ERDDAP dataset id.
pub const DATASET: &str = "GLSEA_ACSPO_GCS";

/// The temperature variable, `degree_C`.
pub const VARIABLE: &str = "sst";

/// The dataset's grid spacing, degrees.
pub const NATIVE_STEP_DEG: f64 = 0.014_015_517;

/// Coverage of the dataset, degrees.
pub const COVERAGE: Bbox = Bbox {
    west: -92.419_950_7,
    south: 38.874_987_2,
    east: -75.881_640_3,
    north: 50.605_975_2,
};

/// Half the side of the box requested around a spot, degrees: about 5.5 km north-south, which
/// reaches every cell [`MAX_CELL_DISTANCE_M`] allows.
const HALF_BOX_DEG: f64 = 0.05;

/// The farthest a cell may be from the spot and still stand for it, metres (about two cells).
pub const MAX_CELL_DISTANCE_M: f64 = 3_000.0;

/// The credit every display of this data shows.
pub fn attribution() -> Attribution {
    Attribution {
        source: SourceId::GLERL,
        credit: "NOAA Great Lakes Environmental Research Laboratory (CoastWatch Great Lakes \
                 Node). Satellite analysis, not intended for legal use."
            .into(),
        url: Some("https://coastwatch.glerl.noaa.gov/glsea/glsea.html".into()),
    }
}

/// The dataset as a grid source.
pub fn grid_source() -> GridSource {
    GridSource {
        node: GridHost::Glerl,
        dataset: DATASET.to_owned(),
        variables: vec![VARIABLE.to_owned()],
        altitude_axis: false,
        native_step_deg: NATIVE_STEP_DEG,
    }
}

/// Whether `point` lies in the dataset's coverage box (which is not the same as being on a Great
/// Lake).
pub fn covers(point: GeoPoint) -> bool {
    point.lat >= COVERAGE.south
        && point.lat <= COVERAGE.north
        && point.lon >= COVERAGE.west
        && point.lon <= COVERAGE.east
}

fn geo(point: RoundedPoint) -> GeoPoint {
    let c = point.coordinates();
    GeoPoint {
        lat: c.latitude,
        lon: c.longitude,
    }
}

/// The request for the newest day in a small box around `point`.
///
/// # Errors
/// [`SourceError::Unsupported`] for a point outside the dataset's coverage.
pub fn request(point: RoundedPoint) -> Result<ErddapRequest, SourceError> {
    let at = geo(point);
    if !covers(at) {
        return Err(SourceError::Unsupported(
            "GLSEA covers the Great Lakes only".into(),
        ));
    }
    let area = Bbox {
        west: (at.lon - HALF_BOX_DEG).max(COVERAGE.west),
        south: (at.lat - HALF_BOX_DEG).max(COVERAGE.south),
        east: (at.lon + HALF_BOX_DEG).min(COVERAGE.east),
        north: (at.lat + HALF_BOX_DEG).min(COVERAGE.north),
    };
    grid_request(&grid_source(), GridTime::Last, area, 1).map_err(map_error)
}

fn map_error(e: ErddapError) -> SourceError {
    match e {
        ErddapError::NoData => SourceError::NoData(Some("GLSEA has no data here".into())),
        ErddapError::Temporary => SourceError::Status {
            status: 503,
            message: Some("GLERL ERDDAP is temporarily unavailable".into()),
        },
        ErddapError::RateLimited { retry_after_s } => SourceError::RateLimited { retry_after_s },
        ErddapError::Status { status } => SourceError::Status {
            status,
            message: None,
        },
        ErddapError::InvalidRequest(what) => {
            SourceError::Unsupported(format!("invalid GLSEA request: {what}"))
        }
        ErddapError::BadTable(what) => SourceError::Parse(what),
        ErddapError::Grid(e) => SourceError::Parse(e.to_string()),
    }
}

/// The nearest cell with a value, and its distance from `at`, metres.
fn nearest_valid(grid: &Grid, at: GeoPoint) -> Option<(f32, f64)> {
    let mut best: Option<(f32, f64)> = None;
    for (i, &lat) in grid.lats.iter().enumerate() {
        for (j, &lon) in grid.lons.iter().enumerate() {
            let Some(v) = grid.value(i, j).filter(|v| v.is_finite()) else {
                continue;
            };
            let d = haversine_m(at, GeoPoint { lat, lon });
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((v, d));
            }
        }
    }
    best
}

/// The surface temperature (°C) at `point` from the response to [`request`].
///
/// The series holds one sample at the analysis time (12:00Z of its day). Its provenance names the
/// cell's distance from the spot; the age of the value is the caller's to judge from the sample
/// time.
///
/// # Errors
/// The mapped failure for an ERDDAP error status; [`SourceError::NoData`] when no cell within
/// [`MAX_CELL_DISTANCE_M`] has a value (land, an inland lake, cloud); [`SourceError::Parse`] for a
/// body that is not the table or a unit other than `degree_C`.
pub fn parse(point: RoundedPoint, response: &Response<'_>) -> Result<Series<f32>, SourceError> {
    let body = check_erddap(response).map_err(map_error)?;
    let grid = parse_grid(&grid_source(), body).map_err(map_error)?;
    if grid.unit != "degree_C" {
        return Err(SourceError::Parse(format!(
            "unexpected temperature unit {}",
            grid.unit
        )));
    }
    let at = geo(point);
    let Some((value, distance)) =
        nearest_valid(&grid, at).filter(|&(_, d)| d <= MAX_CELL_DISTANCE_M)
    else {
        return Err(SourceError::NoData(Some(
            "GLSEA has no value near this spot".into(),
        )));
    };
    let provenance = Provenance {
        source: SourceId::GLERL,
        station: None,
        distance_m: Some(distance as f32),
        relation: Relation::Satellite,
        observed_at: grid.time,
        kind: ValueKind::Satellite,
        quality: Quality::Provisional,
    };
    Ok(Series::new(provenance, vec![Sample::new(grid.time, value)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Coordinates;
    use crate::testutil::fixture_bytes;
    use underbite_core::time::parse_rfc3339;

    const FIXTURE: &str = "glerl-glsea-acspo-sst-erie-2026-11-07.json";

    fn point(lat: f64, lon: f64) -> RoundedPoint {
        RoundedPoint::new(
            Coordinates {
                latitude: lat,
                longitude: lon,
            },
            2,
        )
    }

    fn run(p: RoundedPoint) -> Result<Series<f32>, SourceError> {
        let body = fixture_bytes("tier2", FIXTURE);
        parse(p, &Response::new(200, &body))
    }

    #[test]
    fn request_is_a_small_box_of_the_newest_day() {
        let r = request(point(42.85, -78.95)).expect("request");
        assert_eq!(r.node, GridHost::Glerl);
        assert_eq!(
            r.url,
            "https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?\
             sst%5B(last)%5D%5B(42.8):(42.9)%5D%5B(-79.0):(-78.9)%5D"
        );
    }

    #[test]
    fn request_is_clipped_to_coverage_and_refuses_outside_it() {
        let edge = request(point(38.88, -92.41)).expect("request");
        assert!(
            edge.url.contains("%5B(38.874987):(38.93)%5D"),
            "{}",
            edge.url
        );
        assert!(
            edge.url.contains("%5B(-92.419951):(-92.36)%5D"),
            "{}",
            edge.url
        );
        // The Battery is far outside.
        assert!(matches!(
            request(point(40.70, -74.02)),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn open_water_takes_the_cell_underneath() {
        // The fixture's cell at 42.8554, -78.9230 reads 14.88.
        let s = run(point(42.86, -78.92)).expect("series");
        let sample = s.latest().expect("a sample");
        assert!((sample.value - 14.88).abs() < 1e-4, "{}", sample.value);
        assert_eq!(
            sample.time,
            parse_rfc3339("2026-11-07T12:00:00Z").expect("time")
        );
        let p = s.provenance();
        assert_eq!(p.source, SourceId::GLERL);
        assert_eq!(p.relation, Relation::Satellite);
        assert_eq!(p.kind, ValueKind::Satellite);
        assert!(p.station.is_none());
        assert!(
            p.distance_m.is_some_and(|d| d < 1_000.0),
            "{:?}",
            p.distance_m
        );
    }

    #[test]
    fn a_missing_cell_falls_to_the_nearest_valid_one() {
        // The cell at 42.8834, -78.9650 is null; the one south of it reads 14.87.
        let s = run(point(42.88, -78.97)).expect("series");
        assert!(s.latest().is_some_and(|x| x.value.is_finite()));
        assert!(s.provenance().distance_m.is_some_and(|d| d > 0.0));
    }

    #[test]
    fn a_spot_with_only_missing_cells_around_it_has_no_value() {
        // North-west corner: land, the nearest water is farther than the limit.
        assert!(matches!(
            run(point(42.90, -78.99)),
            Err(SourceError::NoData(_))
        ));
    }

    #[test]
    fn an_all_null_table_has_no_value() {
        let text = String::from_utf8(fixture_bytes("tier2", FIXTURE)).expect("utf8");
        let json: serde_json::Value = serde_json::from_str(&text).expect("json");
        let mut json = json;
        for row in json["table"]["rows"].as_array_mut().expect("rows") {
            row[3] = serde_json::Value::Null;
        }
        let body = serde_json::to_vec(&json).expect("bytes");
        assert!(matches!(
            parse(point(42.86, -78.92), &Response::new(200, &body)),
            Err(SourceError::NoData(_))
        ));
    }

    #[test]
    fn failures_are_mapped() {
        let p = point(42.86, -78.92);
        let none = Response::new(
            404,
            b"Error { code=404; message=\"Not Found: Your query produced no matching results.\"; }",
        );
        assert!(matches!(parse(p, &none), Err(SourceError::NoData(_))));
        assert!(matches!(
            parse(p, &Response::new(503, b"")),
            Err(SourceError::Status { status: 503, .. })
        ));
        assert!(matches!(
            parse(p, &Response::new(200, b"{}")),
            Err(SourceError::Parse(_))
        ));
    }

    #[test]
    fn the_credit_names_glerl_and_the_legal_caveat() {
        let a = attribution();
        assert!(a.credit.contains("CoastWatch Great Lakes Node"));
        assert!(a.credit.contains("not intended for legal use"));
    }
}
