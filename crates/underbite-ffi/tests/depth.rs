//! The depth calls: elevation blocks in the grid store, shading, contours, breaks, the depth
//! inspector row, structure points, state lake contours, reefs and the GEBCO credit.

#![allow(clippy::assert_is_empty)]

use std::sync::Arc;

use serde_json::{Value, json};
use underbite_ffi::depth::{
    DEPTH_LAYER, StructureBand, StructureLayers, StructureType, depth_band_stops, depth_layer_info,
    gebco_capabilities_request, gebco_credit, parse_reefs, parse_state_contours,
    parse_structure_layers, parse_structure_points, reefs_request, state_contour_request,
    structure_band, structure_layers_request, structure_points_request,
};
use underbite_ffi::fetch::HostResponse;
use underbite_ffi::map::{
    GridStore, LayerDraw, MAX_STORE_BLOCKS, MAX_STORE_VALUES, MapError, map_catalogue,
};
use underbite_ffi::settings::default_settings_json;

const NOW: i64 = 1_794_000_000;

fn settings(edit: impl FnOnce(&mut Value)) -> String {
    let mut v: Value = serde_json::from_str(&default_settings_json()).unwrap();
    v["presentation"]["voice"] = json!("plain");
    edit(&mut v);
    v.to_string()
}

fn imperial() -> String {
    settings(|v| v["map"]["depth"]["contour_interval_m"] = json!(3.048))
}

fn metric() -> String {
    settings(|v| {
        v["units"] = json!({ "discharge": "m3s", "height": "m", "speed": "ms", "water_temp": "C",
            "pressure": "hPa", "distance": "km" });
        v["map"]["depth"]["contour_interval_m"] = json!(5.0);
    })
}

fn fixture(path: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/raw");
    std::fs::read(root.join(path)).unwrap()
}

fn response(status: u16, content_type: &str, body: Vec<u8>) -> HostResponse {
    HostResponse {
        status,
        body,
        content_type: Some(content_type.into()),
        date: None,
        expires: None,
        last_modified: None,
        etag: None,
        retry_after: None,
        rate_limit_remaining: None,
    }
}

fn json_response(body: &[u8]) -> HostResponse {
    response(200, "application/json", body.to_vec())
}

/// The recorded 240 by 160 export (40.0 to 40.8 N, 74.2 to 73.0 W) as the service answers it.
fn tiff() -> HostResponse {
    response(
        200,
        "image/tiff",
        fixture("bathymetry/dem-global-mosaic-export-f32.tif"),
    )
}

/// A store holding the recorded export for the harbour view at zoom 9.
fn store_with_dem() -> Arc<GridStore> {
    let store = GridStore::new();
    let request = store
        .dem_area_request(-74.2, 40.0, -73.0, 40.8, 9.0)
        .unwrap()
        .unwrap();
    store.add_dem(request.key, tiff(), NOW).unwrap();
    store
}

fn labels(lines: &[underbite_ffi::depth::ContourLine]) -> Vec<&str> {
    lines.iter().map(|l| l.label.as_str()).collect()
}

#[test]
fn elevation_requests_are_the_recorded_urls_and_the_store_remembers_what_it_holds() {
    let store = GridStore::new();
    let area = store
        .dem_area_request(-74.2, 40.0, -73.0, 40.8, 9.0)
        .unwrap()
        .unwrap();
    assert!(area.key.starts_with("dem/area/"), "{}", area.key);
    assert_eq!(area.node, "ncei_dem");
    assert!(area.request.url.starts_with(
        "https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer/exportImage?bbox=-74.2,40.0,-73.0,40.8&bboxSR=4326&imageSR=4326&size="
    ), "{}", area.request.url);
    let tile = store.dem_tile_request(10, 301, 385).unwrap().unwrap();
    assert_eq!(tile.key, "dem/tile/10/301/385/128");
    assert!(
        tile.request
            .url
            .contains("bboxSR=3857&imageSR=3857&size=128,128")
    );
    assert_eq!(store.block_count(), 0);

    store.add_dem(area.key.clone(), tiff(), NOW).unwrap();
    assert_eq!(store.block_count(), 1);
    // Held: the same view asks for nothing.
    assert_eq!(
        store
            .dem_area_request(-74.2, 40.0, -73.0, 40.8, 9.0)
            .unwrap(),
        None
    );
    store.clear();
    assert_eq!(store.block_count(), 0);
}

#[test]
fn bad_elevation_calls_are_refused_and_failures_come_back_as_errors() {
    let store = GridStore::new();
    assert!(store.dem_tile_request(30, 0, 0).is_err());
    assert!(store.dem_tile_request(3, 8, 0).is_err());
    assert!(
        store
            .dem_area_request(-73.0, 40.0, -74.0, 40.8, 9.0)
            .is_err()
    );
    assert!(
        store
            .dem_area_request(f64::NAN, 40.0, -73.0, 40.8, 9.0)
            .is_err()
    );
    assert!(
        store
            .dem_area_request(-74.0, 40.0, -73.0, 40.8, f64::NAN)
            .is_err()
    );
    // Too wide for one image of cells a pixel across.
    assert!(
        store
            .dem_area_request(-80.0, 35.0, -70.0, 45.0, 6.0)
            .is_err()
    );
    assert!(
        store
            .add_dem("sst/jplMURSST41/last/40/-74/1".into(), tiff(), NOW)
            .is_err()
    );
    assert!(
        store
            .add_dem("dem/tile/10/1/1/0".into(), tiff(), NOW)
            .is_err()
    );
    assert!(
        store
            .add_dem("dem/area/1/2/3/4/0".into(), tiff(), NOW)
            .is_err()
    );

    let key = "dem/area/-74200000/40000000/-73000000/40800000/5000".to_owned();
    let service_error =
        json_response(br#"{"error":{"code":400,"message":"Unable to complete operation."}}"#);
    assert!(matches!(
        store.add_dem(key.clone(), service_error, NOW),
        Err(MapError::Invalid { .. })
    ));
    let busy = response(503, "text/html", b"busy".to_vec());
    assert_eq!(
        store.add_dem(key.clone(), busy, NOW),
        Err(MapError::RetryLater { after_s: 60 })
    );
    let page = response(200, "text/html", b"<html>oops</html>".to_vec());
    assert!(store.add_dem(key.clone(), page, NOW).is_err());
    // A TIFF that is not an elevation image.
    let png = response(
        200,
        "image/png",
        fixture("bathymetry/gebco-wms-getmap-3857-z8-tile.png"),
    );
    assert!(store.add_dem(key, png, NOW).is_err());
    assert_eq!(store.block_count(), 0);
}

#[test]
fn contours_are_labelled_in_feet_for_imperial_settings_and_metres_for_metric() {
    let store = store_with_dem();
    let feet = store
        .contour_lines(-74.2, 40.0, -73.0, 40.8, 9.0, imperial())
        .unwrap();
    assert!(!feet.is_empty());
    for line in &feet {
        assert!(line.label.ends_with(" ft"), "{}", line.label);
        assert!(line.points.len() >= 2);
    }
    let found = labels(&feet);
    assert!(found.contains(&"30 ft"), "{found:?}");
    assert!(found.contains(&"50 ft"), "{found:?}");
    // Every fifth level of the interval is major: 50 ft is the fifth multiple of 10 ft.
    assert!(feet.iter().filter(|l| l.label == "50 ft").all(|l| l.major));
    assert!(feet.iter().filter(|l| l.label == "30 ft").all(|l| !l.major));
    assert!(feet.iter().any(|l| l.label_at.is_some()));
    let at = feet.iter().find_map(|l| l.label_at).unwrap();
    assert!((0.0..180.0).contains(&at.angle_deg));

    let metres = store
        .contour_lines(-74.2, 40.0, -73.0, 40.8, 9.0, metric())
        .unwrap();
    assert!(!metres.is_empty());
    let found = labels(&metres);
    assert!(found.iter().all(|l| l.ends_with(" m")), "{found:?}");
    assert!(
        found.contains(&"10 m") && found.contains(&"25 m"),
        "{found:?}"
    );
    assert!(metres.iter().filter(|l| l.label == "25 m").all(|l| l.major));
    assert!(
        metres
            .iter()
            .filter(|l| l.label == "10 m")
            .all(|l| !l.major)
    );
    // No line reaches the land: depth contours are below the shore.
    assert!(!found.contains(&"0 m"));
}

#[test]
fn contours_stay_bounded_and_empty_where_nothing_is_held() {
    let store = store_with_dem();
    // A tiny interval is doubled until at most 120 levels remain.
    let fine = settings(|v| v["map"]["depth"]["contour_interval_m"] = json!(0.05));
    let lines = store
        .contour_lines(-74.2, 40.0, -73.0, 40.8, 9.0, fine)
        .unwrap();
    let mut levels = labels(&lines);
    levels.sort_unstable();
    levels.dedup();
    assert!(
        !levels.is_empty() && levels.len() <= 120,
        "{}",
        levels.len()
    );
    let vertices: usize = lines.iter().map(|l| l.points.len()).sum();
    assert!(vertices <= underbite_ffi::depth::MAX_CONTOUR_VERTICES);
    // A view elsewhere has nothing; a bad one is refused.
    assert!(
        store
            .contour_lines(-60.0, 30.0, -59.0, 31.0, 9.0, imperial())
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .contour_lines(-59.0, 30.0, -60.0, 31.0, 9.0, imperial())
            .is_err()
    );
    let none = settings(|v| v["map"]["depth"]["contour_interval_m"] = json!(0.0));
    assert!(
        store
            .contour_lines(-74.2, 40.0, -73.0, 40.8, 9.0, none)
            .is_err()
    );
    assert!(
        GridStore::new()
            .contour_lines(-74.2, 40.0, -73.0, 40.8, 9.0, imperial())
            .unwrap()
            .is_empty()
    );
}

/// The point at the Ambrose Channel approach, where the recorded model reads about 29.6 m.
const AMBROSE: (f64, f64) = (40.4525, -73.8075);

fn sources(store: &GridStore, settings: &str) {
    let request = store
        .dem_sources_request(AMBROSE.0, AMBROSE.1, settings.to_owned())
        .unwrap();
    assert!(
        request
            .url
            .contains("/ImageServer/query?geometry=-73.81,40.45&"),
        "{}",
        request.url
    );
    store
        .add_dem_sources(
            AMBROSE.0,
            AMBROSE.1,
            json_response(&fixture("bathymetry/dem-global-mosaic-query-battery.json")),
        )
        .unwrap();
}

#[test]
fn the_inspector_row_says_approximate_depth_and_names_the_datum() {
    let store = store_with_dem();
    sources(&store, &imperial());
    let rows = store
        .inspect(
            AMBROSE.0,
            AMBROSE.1,
            vec!["sst".into(), DEPTH_LAYER.into()],
            imperial(),
            "America/New_York".into(),
        )
        .unwrap();
    // The SST layer has no block in this store: only the depth row.
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.layer, DEPTH_LAYER);
    assert_eq!(row.label, "Approximate depth");
    assert!(
        row.value_text.starts_with("approximate depth 9"),
        "{}",
        row.value_text
    );
    assert!(
        row.value_text.contains(" ft (NAVD 88, CUDEM)"),
        "{}",
        row.value_text
    );
    assert!(row.source.contains("Not for navigation."));
    assert_eq!(row.time_text, "");

    let metres = store
        .inspect(
            AMBROSE.0,
            AMBROSE.1,
            vec![DEPTH_LAYER.into()],
            metric(),
            "UTC".into(),
        )
        .unwrap();
    assert!(
        metres[0].value_text.starts_with("approximate depth 29."),
        "{}",
        metres[0].value_text
    );
    assert!(
        metres[0].value_text.contains(" m (NAVD 88"),
        "{}",
        metres[0].value_text
    );

    // Land at The Battery: a block covers it, with no value.
    let land = store
        .inspect(
            40.7006,
            -74.0142,
            vec![DEPTH_LAYER.into()],
            imperial(),
            "UTC".into(),
        )
        .unwrap();
    assert_eq!(land[0].value_text, "No value here");
    // Where no elevation block covers the point (only imagery does) there is no row.
    assert!(
        store
            .inspect(
                35.0,
                -60.0,
                vec![DEPTH_LAYER.into()],
                imperial(),
                "UTC".into()
            )
            .unwrap()
            .is_empty()
    );
    // Without the model list the datum is not stated, never guessed.
    let bare = store_with_dem();
    let rows = bare
        .inspect(
            AMBROSE.0,
            AMBROSE.1,
            vec![DEPTH_LAYER.into()],
            imperial(),
            "UTC".into(),
        )
        .unwrap();
    assert!(
        rows[0].value_text.ends_with("(datum not stated)"),
        "{}",
        rows[0].value_text
    );
}

/// The Web Mercator tile of zoom `z` holding a point.
fn tile_of(z: u8, lat: f64, lon: f64) -> (u32, u32) {
    let n = f64::from(1u32 << z);
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let y = ((1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor();
    (x as u32, y as u32)
}

#[test]
fn the_depth_tile_is_shaded_with_the_band_highlighted() {
    let store = store_with_dem();
    let (x, y) = tile_of(10, AMBROSE.0, AMBROSE.1);
    let plain = store
        .render_depth_tile(10, x, y, imperial())
        .unwrap()
        .unwrap();
    assert_eq!(&plain[..8], b"\x89PNG\r\n\x1a\n");
    assert!(plain.len() > 100);
    let banded = store
        .render_depth_tile(
            10,
            x,
            y,
            settings(|v| v["map"]["depth"]["band_m"] = json!([25.0, 35.0])),
        )
        .unwrap()
        .unwrap();
    assert_ne!(plain, banded, "the band changes the picture");
    let ramped = store
        .render_depth_tile(
            10,
            x,
            y,
            settings(|v| v["map"]["depth"]["ramp"] = json!("thermal")),
        )
        .unwrap()
        .unwrap();
    assert_ne!(plain, ramped);
    // Nothing held far away; a bad tile is refused.
    assert_eq!(store.render_depth_tile(10, 0, 0, imperial()).unwrap(), None);
    assert!(store.render_depth_tile(10, 5_000, 0, imperial()).is_err());
}

#[test]
fn depth_words_the_legend_and_the_band_stops_follow_the_settings() {
    let info = depth_layer_info(imperial()).unwrap();
    assert_eq!(info.id, DEPTH_LAYER);
    assert_eq!(info.title, "Water depth");
    assert_eq!(info.notice, "Not for navigation.");
    assert!(info.credit.contains("NOAA NCEI") && info.credit.contains("Not for navigation."));
    assert!(info.depth_reference.contains("not chart datum"));
    assert_eq!(info.ramp, "depth");
    assert_eq!(info.legend.first().unwrap().label, "0 ft");
    assert_eq!(info.legend.last().unwrap().label, "328.1 ft");
    let metres = depth_layer_info(metric()).unwrap();
    assert_eq!(metres.legend.last().unwrap().label, "100 m");

    let feet = depth_band_stops(imperial()).unwrap();
    assert!(feet.len() >= 3);
    assert!(feet.iter().all(|s| s.label.ends_with(" ft")));
    let m = depth_band_stops(metric()).unwrap();
    assert!(m.iter().all(|s| s.label.ends_with(" m")));
    assert_eq!(m.first().unwrap().value, 0.0);
}

/// A phone-sized view (390 by 844 points) at `zoom` around The Battery.
fn phone_view(zoom: u8) -> (f64, f64, f64, f64) {
    let (lat, lon): (f64, f64) = (40.70, -74.02);
    let deg_per_pt = 360.0 / (256.0 * f64::from(zoom).exp2());
    let half_w = 390.0 / 2.0 * deg_per_pt;
    let half_h = 844.0 / 2.0 * deg_per_pt * lat.to_radians().cos();
    (lon - half_w, lat - half_h, lon + half_w, lat + half_h)
}

#[test]
fn the_catalogue_lists_the_depth_layers_drawn_from_the_elevation_store() {
    let layers = map_catalogue(imperial()).unwrap();
    let by_id = |id: &str| layers.iter().find(|l| l.id == id).unwrap();
    let info = depth_layer_info(imperial()).unwrap();
    for id in ["depth-shading", "depth-contours"] {
        let l = by_id(id);
        assert_eq!(l.group, "depth", "{id}");
        assert_eq!(l.draw, LayerDraw::Depth, "{id}");
        assert!(l.available && !l.shown, "{id}");
        assert_eq!(l.credit, info.credit, "{id}");
        assert_eq!(l.notice.as_deref(), Some("Not for navigation."), "{id}");
        assert_eq!(
            l.depth_reference.as_deref(),
            Some(info.depth_reference.as_str())
        );
        assert_eq!(l.min_zoom, Some(8), "{id}");
        assert!(l.datasets.is_empty(), "{id}");
        // A phone's view at the layer's shallowest zoom fits one elevation area; one zoom out
        // is too wide.
        let store = GridStore::new();
        let ask = |zoom: u8| {
            let (west, south, east, north) = phone_view(zoom);
            store.dem_area_request(west, south, east, north, f64::from(zoom))
        };
        assert!(ask(8).unwrap().is_some());
        assert!(ask(7).is_err());
    }
    let shading = by_id("depth-shading");
    assert_eq!(shading.title, info.title);
    assert_eq!(
        shading.legend_caption.as_deref(),
        Some(info.legend_caption.as_str())
    );
    assert_eq!(shading.ramp.as_deref(), Some("depth"));
    assert_eq!(shading.legend, info.legend);
    let contours = by_id("depth-contours");
    assert_eq!(contours.title, "Depth contours");
    assert!(contours.legend.is_empty() && contours.ramp.is_none());

    // The shading follows the depth section's ramp, as its tiles do.
    let thermal = settings(|v| v["map"]["depth"]["ramp"] = json!("thermal"));
    let layers = map_catalogue(thermal.clone()).unwrap();
    let shading = layers.iter().find(|l| l.id == "depth-shading").unwrap();
    assert_eq!(shading.ramp.as_deref(), Some("thermal"));
    assert_eq!(shading.legend, depth_layer_info(thermal).unwrap().legend);
}

#[test]
fn the_gebco_credit_follows_the_capabilities_and_falls_back_to_the_layers_own() {
    assert!(
        gebco_capabilities_request()
            .url
            .starts_with("https://wms.gebco.net/mapserv?request=GetCapabilities")
    );
    let xml = fixture("bathymetry/gebco-wms-capabilities-1.3.0.xml");
    let credit = gebco_credit(response(200, "text/xml", xml), imperial()).unwrap();
    assert!(credit.contains("GEBCO_2026 Grid"), "{credit}");
    assert!(credit.contains("doi:10.5285/"), "{credit}");
    assert!(credit.ends_with("Not for navigation."));
    let layers = map_catalogue(imperial()).unwrap();
    let own = &layers
        .iter()
        .find(|l| l.id == "gebco-relief")
        .unwrap()
        .credit;
    let fallback = gebco_credit(response(503, "text/html", b"busy".to_vec()), imperial()).unwrap();
    assert_eq!(&fallback, own);
    let unreadable = gebco_credit(response(200, "text/xml", b"<x/>".to_vec()), imperial()).unwrap();
    assert_eq!(&unreadable, own);
}

#[test]
fn the_store_bound_covers_elevation_blocks() {
    let store = GridStore::new();
    for cell in 1..=120 {
        let key = format!("dem/area/-74200000/40000000/-73000000/40800000/{cell}");
        store.add_dem(key, tiff(), NOW).unwrap();
    }
    assert!(store.block_count() as usize <= MAX_STORE_BLOCKS);
    assert!(store.value_count() as usize <= MAX_STORE_VALUES);
    assert!(store.block_count() > 1);
}

// ---------------------------------------------------------------------------------------------
// Breaks
// ---------------------------------------------------------------------------------------------

/// A MUR block at one hundredth of a degree (stride 1) with a front across -73.75: 14 °C to the
/// west and 20 °C to the east over 0.04 degrees, in the shape ERDDAP answers.
fn front_block() -> Vec<u8> {
    let template: Value = serde_json::from_slice(&fixture(
        "erddap/jplmursst41-sst-mask-subset-2026-11-17.json",
    ))
    .unwrap();
    let mut rows = Vec::new();
    for i in 0..=50 {
        let lat = 40.0 + f64::from(i) * 0.01;
        for j in 0..=50 {
            let lon = -74.0 + f64::from(j) * 0.01;
            let t = ((lon + 73.77) / 0.04).clamp(0.0, 1.0);
            let sst = 14.0 + 6.0 * t * t * (3.0 - 2.0 * t);
            rows.push(json!([
                "2026-11-16T09:00:00Z",
                (lat * 100.0).round() / 100.0,
                (lon * 100.0).round() / 100.0,
                (sst * 1000.0).round() / 1000.0,
                1
            ]));
        }
    }
    let mut v = template;
    v["table"]["rows"] = Value::Array(rows);
    v.to_string().into_bytes()
}

fn store_with_front() -> Arc<GridStore> {
    let store = GridStore::new();
    let requests = store
        .grid_requests(
            "sst".into(),
            None,
            None,
            -74.0,
            40.0,
            -73.5,
            40.5,
            12.0,
            NOW,
        )
        .unwrap();
    assert_eq!(requests.len(), 1);
    store
        .add_grid(requests[0].key.clone(), json_response(&front_block()), NOW)
        .unwrap();
    store
}

#[test]
fn breaks_follow_a_front_and_state_their_strength_in_the_users_units() {
    let store = store_with_front();
    let imperial_breaks = store
        .break_lines("sst".into(), -74.0, 40.0, -73.5, 40.5, imperial())
        .unwrap();
    assert!(!imperial_breaks.is_empty());
    for b in &imperial_breaks {
        assert!(b.points.len() >= 2);
        assert!(
            b.strength_text.ends_with(" °F per mi"),
            "{}",
            b.strength_text
        );
        for p in &b.points {
            assert!((-73.82..=-73.72).contains(&p.lon), "{p:?}");
        }
    }
    let metric_breaks = store
        .break_lines("sst".into(), -74.0, 40.0, -73.5, 40.5, metric())
        .unwrap();
    assert_eq!(metric_breaks.len(), imperial_breaks.len());
    assert!(
        metric_breaks
            .iter()
            .all(|b| b.strength_text.ends_with(" °C per km"))
    );
    // A threshold above the front's strength draws nothing.
    let stiff = settings(|v| v["map"]["breaks"]["sst_c_per_km"] = json!(50.0));
    assert!(
        store
            .break_lines("sst".into(), -74.0, 40.0, -73.5, 40.5, stiff)
            .unwrap()
            .is_empty()
    );
    // A view beside the front has none of its pieces.
    assert!(
        store
            .break_lines("sst".into(), -73.6, 40.0, -73.5, 40.5, imperial())
            .unwrap()
            .is_empty()
    );
    assert!(vertex_count(&imperial_breaks) <= underbite_ffi::depth::MAX_BREAK_VERTICES);
}

fn vertex_count(lines: &[underbite_ffi::depth::BreakLine]) -> usize {
    lines.iter().map(|l| l.points.len()).sum()
}

#[test]
fn breaks_need_a_layer_that_has_them_and_blocks_near_the_grids_own_resolution() {
    let store = store_with_front();
    assert!(
        store
            .break_lines("radar".into(), -74.0, 40.0, -73.5, 40.5, imperial())
            .is_err()
    );
    assert!(
        store
            .break_lines("sst".into(), -73.5, 40.0, -74.0, 40.5, imperial())
            .is_err()
    );
    assert!(
        store
            .break_lines("nope".into(), -74.0, 40.0, -73.5, 40.5, imperial())
            .is_err()
    );
    // Nothing held for the layer: no breaks.
    assert!(
        GridStore::new()
            .break_lines("sst".into(), -74.0, 40.0, -73.5, 40.5, imperial())
            .unwrap()
            .is_empty()
    );
    // The recorded subset is strided by five: too coarse, so it draws no breaks.
    let coarse = GridStore::new();
    let requests = coarse
        .grid_requests("sst".into(), None, None, -74.2, 40.0, -73.0, 40.8, 9.0, NOW)
        .unwrap();
    coarse
        .add_grid(
            requests[0].key.clone(),
            json_response(&fixture(
                "erddap/jplmursst41-sst-mask-subset-2026-11-17.json",
            )),
            NOW,
        )
        .unwrap();
    assert!(
        coarse
            .break_lines("sst".into(), -74.2, 40.0, -73.0, 40.8, imperial())
            .unwrap()
            .is_empty()
    );
}

// ---------------------------------------------------------------------------------------------
// Structure points
// ---------------------------------------------------------------------------------------------

#[test]
fn structure_bands_follow_the_zoom_and_layer_ids_are_asked_for() {
    assert_eq!(structure_band(8), None);
    assert_eq!(structure_band(9), Some(StructureBand::Coastal));
    assert_eq!(structure_band(12), Some(StructureBand::Approach));
    assert_eq!(structure_band(14), Some(StructureBand::Harbour));
    let request = structure_layers_request(StructureBand::Approach);
    assert_eq!(
        request.url,
        "https://gis.charttools.noaa.gov/arcgis/rest/services/encdirect/enc_approach/MapServer?f=json"
    );
    assert_eq!(request.max_age_s, Some(86_400));
    let layers = parse_structure_layers(
        json_response(
            br#"{"layers":[{"id":36,"name":"Approach.Obstruction_point"},
                {"id":37,"name":"Approach.Underwater_Awash_Rock_point"},{"id":39,"name":"Approach.Wreck_point"}]}"#,
        ),
        0,
    )
    .unwrap();
    assert_eq!(
        layers,
        StructureLayers {
            wrecks: Some(39),
            obstructions: Some(36),
            rocks: Some(37)
        }
    );
    // The first transient body is retried after a pause, the second is final.
    let transient = br#"{"error":{"code":500,"message":"Error handling service request :Could not find service. Service may be stopped or it may not be configured.","details":[]}}"#;
    assert_eq!(
        parse_structure_layers(json_response(transient), 0),
        Err(MapError::RetryLater { after_s: 3 })
    );
    assert!(matches!(
        parse_structure_layers(json_response(transient), 1),
        Err(MapError::Invalid { .. })
    ));
}

#[test]
fn structure_points_are_worded_and_never_show_a_depth() {
    let layers = StructureLayers {
        wrecks: Some(36),
        obstructions: None,
        rocks: None,
    };
    let request = structure_points_request(
        StructureBand::Harbour,
        layers,
        StructureType::Wreck,
        -74.2,
        40.35,
        -73.7,
        40.75,
        0,
    )
    .unwrap()
    .unwrap();
    assert!(
        request.url.contains("/enc_harbour/MapServer/36/query?"),
        "{}",
        request.url
    );
    assert!(request.url.contains("resultRecordCount=1000"));
    // No obstruction layer in the band: nothing to ask.
    assert_eq!(
        structure_points_request(
            StructureBand::Harbour,
            layers,
            StructureType::Obstruction,
            -74.2,
            40.35,
            -73.7,
            40.75,
            0
        )
        .unwrap(),
        None
    );
    assert!(
        structure_points_request(
            StructureBand::Harbour,
            layers,
            StructureType::Wreck,
            -73.7,
            40.35,
            -74.2,
            40.75,
            0
        )
        .is_err()
    );

    let body = fixture("ncds/encdirect-enc-harbour-wreck-point-query-lower-bay-2026-11-17.json");
    let page =
        parse_structure_points(StructureType::Wreck, json_response(&body), 0, imperial()).unwrap();
    assert!(!page.points.is_empty());
    assert_eq!(
        page.read as usize,
        page.points.len() + page.skipped as usize
    );
    for p in &page.points {
        assert_eq!(p.kind, StructureType::Wreck);
        assert!(p.id.starts_with("wreck/"));
        assert!(p.title == "Wreck" || p.title == "Fish haven", "{}", p.title);
        assert!((40.0..41.0).contains(&p.lat) && (-75.0..-73.0).contains(&p.lon));
        assert!(p.depth_note.starts_with("Charted depth not shown"));
        if let Some(category) = &p.category {
            assert!(category.starts_with("As charted: "), "{category}");
        }
        if let Some(surveyed) = &p.surveyed {
            assert!(surveyed.starts_with("Surveyed "), "{surveyed}");
        }
    }
    // The raw VALSOU is not in anything that crosses.
    let shown = format!("{page:?}");
    assert!(!shown.to_ascii_lowercase().contains("valsou"));

    let empty =
        fixture("ncds/encdirect-enc-approach-wreck-point-query-offshore-empty-2026-11-17.json");
    let none =
        parse_structure_points(StructureType::Wreck, json_response(&empty), 0, imperial()).unwrap();
    assert!(none.points.is_empty() && !none.more);
}

// ---------------------------------------------------------------------------------------------
// State lake contours and reefs
// ---------------------------------------------------------------------------------------------

#[test]
fn state_contours_are_requested_inside_their_coverage_and_labelled_below_the_surface() {
    // A box in Indiana.
    let request = state_contour_request(
        "lake-contours-in".into(),
        -86.31,
        39.11,
        -86.28,
        39.14,
        14,
        0,
    )
    .unwrap()
    .unwrap();
    assert!(
        request
            .url
            .contains("Lake_Bathymetry/FeatureServer/0/query"),
        "{}",
        request.url
    );
    // Outside the state's data, or too wide a view for the layer's zoom: nothing is asked.
    assert_eq!(
        state_contour_request("lake-contours-in".into(), -74.2, 40.0, -73.0, 40.8, 14, 0).unwrap(),
        None
    );
    assert_eq!(
        state_contour_request(
            "lake-contours-in".into(),
            -86.31,
            39.11,
            -86.28,
            39.14,
            6,
            0
        )
        .unwrap(),
        None
    );
    assert!(state_contour_request("sst".into(), -86.31, 39.11, -86.28, 39.14, 14, 0).is_err());
    assert!(state_contour_request("nope".into(), -86.31, 39.11, -86.28, 39.14, 14, 0).is_err());

    let body = fixture("state-maps/in-lake-bathymetry-mill-pond-lake.synthetic.geojson");
    let page = parse_state_contours(
        "lake-contours-in".into(),
        json_response(&body),
        15,
        imperial(),
    )
    .unwrap();
    assert!(page.lines.len() >= 18);
    assert_eq!(page.read as usize, page.lines.len());
    let found: Vec<&str> = page.lines.iter().map(|l| l.label.as_str()).collect();
    assert!(
        found.contains(&"20 ft") && found.contains(&"2 ft"),
        "{found:?}"
    );
    assert!(page.lines.iter().any(|l| l.major));
    assert!(page.lines.iter().any(|l| l.label_at.is_some()));
    assert_eq!(page.lines[0].lake.as_deref(), Some("Mill Pond Lake"));
    let metres = parse_state_contours(
        "lake-contours-in".into(),
        json_response(&body),
        15,
        metric(),
    )
    .unwrap();
    assert!(metres.lines.iter().any(|l| l.label == "6.1 m"));

    let mn = fixture("state-maps/mn-lake-bathymetry-alder-99000100.synthetic.geojson");
    let page = parse_state_contours(
        "lake-contours-mn".into(),
        json_response(&mn),
        15,
        imperial(),
    )
    .unwrap();
    assert!(!page.lines.is_empty());
    assert!(page.lines.iter().all(|l| !l.major));
    assert!(
        parse_state_contours(
            "lake-contours-in".into(),
            json_response(b"{"),
            15,
            imperial()
        )
        .is_err()
    );
}

#[test]
fn reefs_are_listed_with_their_depths_in_the_users_unit() {
    assert_eq!(
        reefs_request().url,
        "https://data.ny.gov/resource/jsnj-ehv9.json?$limit=50"
    );
    let body = fixture("state-maps/ny-artificial-reefs-socrata-2026-11-17.json");
    let reefs = parse_reefs(json_response(&body), imperial()).unwrap();
    assert_eq!(reefs.len(), 16);
    for r in &reefs {
        assert!(!r.name.is_empty());
        assert!(r.corners.len() <= 4);
        if let Some(depth) = &r.depth_text {
            assert!(
                depth.starts_with("Depth ") && depth.ends_with(" ft"),
                "{depth}"
            );
        }
    }
    assert!(reefs.iter().any(|r| r.depth_text.is_some()));
    let metres = parse_reefs(json_response(&body), metric()).unwrap();
    assert!(
        metres
            .iter()
            .filter_map(|r| r.depth_text.as_deref())
            .all(|d| d.ends_with(" m"))
    );
    assert!(parse_reefs(json_response(b"[1"), imperial()).is_err());
}
