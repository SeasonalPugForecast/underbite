//! The map calls: the worded catalogue and presets, tile requests and their classification,
//! satellite dates, the grid store's requests, rendering, arrows and point inspector, the ERDDAP
//! pacer, measuring and the radar loop.

use std::sync::Arc;

use serde_json::{Value, json};
use underbite_ffi::fetch::{HostMethod, HostResponse};
use underbite_ffi::map::{
    GridPacer, GridStore, LayerDates, LayerDraw, MAX_REQUESTS_PER_VIEW, MAX_STORE_BLOCKS,
    MAX_STORE_VALUES, MapError, MapPoint, TileOutcome, classify_tile, gibs_default_date,
    map_catalogue, map_presets, map_tile_size, measure, parse_time_axis, previous_date,
    radar_covers, radar_frame_request, radar_frames, radar_latest_valid, radar_meta_request,
    tile_request, time_axis_request,
};
use underbite_ffi::settings::default_settings_json;

fn settings(edit: impl FnOnce(&mut Value)) -> String {
    let mut v: Value = serde_json::from_str(&default_settings_json()).unwrap();
    edit(&mut v);
    v.to_string()
}

fn plain() -> String {
    settings(|v| v["presentation"]["voice"] = json!("plain"))
}

fn metric() -> String {
    settings(|v| {
        v["units"] = json!({ "discharge": "m3s", "height": "m", "speed": "ms", "water_temp": "C",
            "pressure": "hPa", "distance": "km" });
    })
}

/// The URL of the recorded NOAA chart tile at zoom 14 over The Battery
/// (`tests/fixtures/raw/ncds/noaa-charts-wmts-z14-x4823-y6160.png`).
const NCDS_Z14: &str = "https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/NOAACharts/MapServer/WMTS/tile/1.0.0/MarineChart_Services_NOAACharts/default/default028mm/12/6160/4823.png";

#[test]
fn the_catalogue_is_worded_with_credits_and_notices() {
    let layers = map_catalogue(plain()).unwrap();
    assert_eq!(layers.len(), 33);
    assert!(layers[..4].iter().all(|l| l.is_base));
    assert_eq!(layers[0].draw, LayerDraw::AppleStandard);
    assert!(layers[0].shown, "the default base map is shown");
    for l in &layers {
        assert!(!l.title.is_empty(), "{}", l.id);
        assert!(!l.credit.is_empty(), "{}", l.id);
        assert!((0.0..=1.0).contains(&l.opacity), "{}", l.id);
    }
    let by_id = |id: &str| layers.iter().find(|l| l.id == id).unwrap();
    let chart = by_id("noaa-chart");
    assert_eq!(chart.title, "NOAA chart");
    assert_eq!(chart.dates, LayerDates::Undated);
    assert_eq!(chart.notice.as_deref(), Some("Not for navigation."));
    assert!(chart.credit.contains("Not to be used for navigation."));
    let sst = by_id("sst");
    assert_eq!(
        (sst.group.as_str(), sst.group_title.as_str()),
        ("water", "Water")
    );
    assert_eq!(sst.draw, LayerDraw::GridRaster);
    assert_eq!(sst.dates, LayerDates::Daily);
    assert_eq!(sst.datasets, ["jplMURSST41"]);
    assert_eq!(sst.legend_caption.as_deref(), Some("Cold to warm"));
    assert_eq!(sst.notice, None);
    assert_eq!(sst.legend.first().unwrap().label, "32 °F");
    assert_eq!(sst.legend.last().unwrap().label, "86 °F");
    assert!(sst.acknowledgements[0].contains("MEaSUREs"));
    let currents = by_id("hf-radar-currents");
    assert_eq!(currents.draw, LayerDraw::GridArrows);
    assert!(currents.notice.is_some());
    assert_eq!(currents.legend.last().unwrap().label, "2.9 kn");
    let clarity = by_id("clarity");
    assert_eq!(clarity.datasets.len(), 2);
    assert_eq!(clarity.legend.first().unwrap().label, "0.02 1/m");
    let wind = by_id("wind");
    assert!(!wind.available);
    assert_eq!(wind.unavailable_text.as_deref(), Some("No source yet"));
    let reefs = by_id("artificial-reefs-ny");
    assert!(
        reefs
            .depth_reference
            .as_deref()
            .unwrap()
            .contains("no datum")
    );
    assert!(reefs.terms.is_some());
}

#[test]
fn the_catalogue_follows_the_settings_voice_units_and_layer_states() {
    let luna = map_catalogue(default_settings_json()).unwrap();
    let wind = luna.iter().find(|l| l.id == "wind").unwrap();
    assert_eq!(
        wind.unavailable_text.as_deref(),
        Some("Luna hasn't found a source yet")
    );
    let chart_luna = luna.iter().find(|l| l.id == "noaa-chart").unwrap();
    let chart_plain = map_catalogue(plain())
        .unwrap()
        .into_iter()
        .find(|l| l.id == "noaa-chart")
        .unwrap();
    assert_eq!(chart_luna.credit, chart_plain.credit);
    assert_eq!(chart_luna.notice, chart_plain.notice);

    let metric = map_catalogue(metric()).unwrap();
    let sst = metric.iter().find(|l| l.id == "sst").unwrap();
    assert_eq!(sst.legend.first().unwrap().label, "0.0 °C");

    let edited = settings(|v| {
        v["map"]["base"] = json!("noaa-chart");
        let layers = v["map"]["layers"].as_array_mut().unwrap();
        let sst = layers.iter_mut().find(|l| l["id"] == "sst").unwrap();
        sst["shown"] = json!(true);
        sst["opacity"] = json!(0.4);
        sst["ramp"] = json!("diverging");
        sst["range"] = json!([10.0, 20.0]);
    });
    let layers = map_catalogue(edited).unwrap();
    let sst = layers.iter().find(|l| l.id == "sst").unwrap();
    assert!(sst.shown);
    assert!((sst.opacity - 0.4).abs() < 1e-6);
    assert_eq!(sst.ramp.as_deref(), Some("diverging"));
    assert_eq!(sst.legend.first().unwrap().label, "50 °F");
    assert_eq!(sst.legend.last().unwrap().label, "68 °F");
    assert!(layers.iter().find(|l| l.id == "noaa-chart").unwrap().shown);
    assert!(
        !layers
            .iter()
            .find(|l| l.id == "apple-standard")
            .unwrap()
            .shown
    );

    assert!(map_catalogue("{".into()).is_err());
}

#[test]
fn presets_are_worded_and_an_unknown_active_one_falls_back() {
    let p = map_presets(plain()).unwrap();
    assert_eq!(p.active, "inshore-flats");
    assert_eq!(p.presets.len(), 6);
    assert_eq!(p.presets[0].name, "Inshore flats");
    assert!(p.presets.iter().all(|p| p.built_in && p.base.is_some()));

    let surf = settings(|v| v["map"]["active_preset"] = json!(" Surf "));
    assert_eq!(map_presets(surf).unwrap().active, "surf");
    let unknown = settings(|v| v["map"]["active_preset"] = json!("Sunday morning"));
    assert_eq!(map_presets(unknown).unwrap().active, "inshore-flats");

    let own = settings(|v| {
        v["map"]["presets"] = json!([{ "name": "Surf", "layers": [
            { "id": "buoys", "shown": true, "opacity": 0.5 },
            { "id": "radar", "shown": false }
        ] }]);
        v["map"]["active_preset"] = json!("Surf");
    });
    let p = map_presets(own).unwrap();
    assert_eq!(
        p.active, "Surf",
        "the user's own preset wins over a built-in id"
    );
    let mine = p.presets.last().unwrap();
    assert!(!mine.built_in);
    assert_eq!(mine.layers.len(), 1);
    assert_eq!(mine.layers[0].id, "buoys");
}

#[test]
fn a_chart_tile_request_is_the_recorded_url() {
    let fetch = tile_request("noaa-chart".into(), None, 14, 4823, 6160)
        .unwrap()
        .unwrap();
    assert_eq!(fetch.request.url, NCDS_Z14);
    assert_eq!(fetch.request.method, HostMethod::Get);
    assert_eq!((fetch.z, fetch.x, fetch.y), (14, 4823, 6160));
    assert_eq!(fetch.crop, None);
    assert_eq!(fetch.request.max_age_s, Some(86_400));
    // Outside the cache's levels there is nothing to fetch.
    assert_eq!(
        tile_request("noaa-chart".into(), None, 1, 0, 0).unwrap(),
        None
    );
}

#[test]
fn satellite_tiles_take_a_date_and_over_zoom_past_their_level() {
    let fetch = tile_request("sst-imagery".into(), Some("2026-11-07".into()), 7, 37, 48)
        .unwrap()
        .unwrap();
    assert_eq!(
        fetch.request.url,
        "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GHRSST_L4_MUR_Sea_Surface_Temperature/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png"
    );
    let deeper = tile_request("sst-imagery".into(), Some("2026-11-07".into()), 9, 149, 194)
        .unwrap()
        .unwrap();
    assert_eq!((deeper.z, deeper.x, deeper.y), (7, 37, 48));
    let crop = deeper.crop.unwrap();
    assert_eq!((crop.left, crop.top, crop.size), (0.25, 0.5, 0.25));
    assert!(deeper.request.url.ends_with("/7/48/37.png"));
    // Radar starts at zoom 6.
    assert_eq!(tile_request("radar".into(), None, 5, 9, 12).unwrap(), None);
    assert!(
        tile_request("radar".into(), None, 8, 75, 96)
            .unwrap()
            .unwrap()
            .request
            .url
            .contains("nexrad-n0q-900913")
    );
}

#[test]
fn bad_tile_requests_are_refused() {
    let refused = |r: Result<_, MapError>| matches!(r, Err(MapError::Invalid { .. }));
    assert!(refused(tile_request("sst".into(), None, 7, 37, 48)));
    assert!(refused(tile_request("nope".into(), None, 7, 37, 48)));
    assert!(refused(tile_request("noaa-chart".into(), None, 3, 8, 0)));
    assert!(refused(tile_request(
        "sst-imagery".into(),
        Some("2026-02-30".into()),
        7,
        37,
        48
    )));
}

#[test]
fn tile_responses_are_classified_per_service() {
    let ncds = |status, ct: &str, len| {
        classify_tile("noaa-chart".into(), status, Some(ct.into()), len, 0).unwrap()
    };
    assert_eq!(ncds(200, "image/png", 9_000), TileOutcome::Image);
    assert_eq!(
        ncds(200, "text/html", 700),
        TileOutcome::Retry { after_s: 3 }
    );
    assert_eq!(
        classify_tile("noaa-chart".into(), 200, Some("text/html".into()), 700, 1).unwrap(),
        TileOutcome::Failed
    );
    assert_eq!(
        classify_tile("sst-imagery".into(), 404, None, 0, 0).unwrap(),
        TileOutcome::NoData
    );
    assert_eq!(
        classify_tile("sst-imagery".into(), 400, None, 0, 0).unwrap(),
        TileOutcome::TooDeep
    );
    assert!(classify_tile("sst".into(), 200, None, 0, 0).is_err());
    assert_eq!(map_tile_size(), 256);
}

#[test]
fn satellite_dates_start_yesterday_and_step_back() {
    // 2026-11-07T09:00:00Z.
    let d = gibs_default_date(1_794_042_000);
    assert_eq!((d.date.as_str(), d.label.as_str()), ("2026-11-06", "Nov 6"));
    let back = previous_date("2026-01-01".into()).unwrap();
    assert_eq!(
        (back.date.as_str(), back.label.as_str()),
        ("2025-12-31", "Dec 31")
    );
    assert!(previous_date("yesterday".into()).is_err());
}

// ---------------------------------------------------------------------------------------------
// Gridded layers
// ---------------------------------------------------------------------------------------------

/// 2026-11-15T12:00:00Z.
const NOW: i64 = 1_794_744_000;

fn fixture(path: &str) -> Vec<u8> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/raw");
    std::fs::read(root.join(path)).unwrap()
}

fn response(status: u16, body: Vec<u8>) -> HostResponse {
    HostResponse {
        status,
        body,
        content_type: Some("application/json;charset=UTF-8".into()),
        date: None,
        expires: None,
        last_modified: None,
        etag: None,
        retry_after: None,
        rate_limit_remaining: None,
    }
}

fn mur() -> Vec<u8> {
    fixture("erddap/jplmursst41-sst-subset-2026-11-07.json")
}

/// The view around the recorded subsets: 40.0 to 40.8 N, 74.2 to 73.0 W, at zoom 9.
fn harbor(
    store: &GridStore,
    layer: &str,
    dataset: Option<&str>,
    time: Option<i64>,
    now: i64,
) -> Result<Vec<underbite_ffi::map::GridRequest>, MapError> {
    store.grid_requests(
        layer.into(),
        dataset.map(Into::into),
        time,
        -74.2,
        40.0,
        -73.0,
        40.8,
        9.0,
        now,
    )
}

/// A store holding the recorded MUR subset for the newest SST.
fn store_with_mur() -> Arc<GridStore> {
    let store = GridStore::new();
    let requests = harbor(&store, "sst", None, None, NOW).unwrap();
    store
        .add_grid(requests[0].key.clone(), response(200, mur()), NOW)
        .unwrap();
    store
}

#[test]
fn grid_requests_are_blocks_of_the_view_for_the_node() {
    let store = GridStore::new();
    let requests = harbor(&store, "sst", None, None, NOW).unwrap();
    assert!(!requests.is_empty() && requests.len() <= MAX_REQUESTS_PER_VIEW);
    for r in &requests {
        assert_eq!(r.node, "pfeg");
        assert!(
            r.request.url.starts_with(
                "https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.json?analysed_sst%5B(last)%5D"
            ),
            "{}",
            r.request.url
        );
        assert!(r.key.starts_with("sst/jplMURSST41/last/"), "{}", r.key);
        assert_eq!(r.request.max_age_s, Some(3_600));
    }
    // A held block is not asked again while it is fresh, and is after three hours.
    store
        .add_grid(requests[0].key.clone(), response(200, mur()), NOW)
        .unwrap();
    let again = harbor(&store, "sst", None, None, NOW + 60).unwrap();
    assert_eq!(again.len(), requests.len() - 1);
    let later = harbor(&store, "sst", None, None, NOW + 4 * 3_600).unwrap();
    assert_eq!(later.len(), requests.len());

    // Above the layer's shallowest zoom there is nothing to ask.
    let wide = store
        .grid_requests("sst".into(), None, None, -80.0, 35.0, -65.0, 45.0, 4.0, NOW)
        .unwrap();
    assert_eq!(wide, []);

    // A fixed time and the fallback dataset.
    let dated = harbor(
        &store,
        "clarity",
        Some("noaacwN20VIIRSkd490NSWUSDaily"),
        Some(1_794_683_995),
        NOW,
    )
    .unwrap();
    assert_eq!(dated[0].node, "coastwatch");
    assert!(dated[0].request.url.contains(
        "noaacwN20VIIRSkd490NSWUSDaily.json?kd_490%5B(2026-11-14T19:19:55Z)%5D%5B(0.0)%5D"
    ));
    assert!(
        dated[0]
            .key
            .starts_with("clarity/noaacwN20VIIRSkd490NSWUSDaily/1794683995/")
    );
    assert_eq!(dated[0].request.max_age_s, Some(86_400));

    let refused = |r: Result<_, MapError>| matches!(r, Err(MapError::Invalid { .. }));
    assert!(refused(harbor(&store, "radar", None, None, NOW)));
    assert!(refused(harbor(
        &store,
        "clarity",
        Some("jplMURSST41"),
        None,
        NOW
    )));
}

#[test]
fn the_mur_subset_renders_its_tile_and_inspects_to_fahrenheit() {
    let store = store_with_mur();
    let png = store
        .render_tile("sst".into(), None, None, None, 7, 37, 48)
        .unwrap()
        .unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(png.len() > 100);
    // The user's ramp and range draw too; nothing is held far away.
    let diverging = store.render_tile(
        "sst".into(),
        Some("diverging".into()),
        Some(10.0),
        Some(20.0),
        7,
        37,
        48,
    );
    assert!(diverging.unwrap().is_some());
    assert_eq!(
        store
            .render_tile("sst".into(), None, None, None, 7, 20, 48)
            .unwrap(),
        None
    );

    let rows = store
        .inspect(
            40.5,
            -73.5,
            vec!["sst".into(), "radar".into(), "chlorophyll".into()],
            default_settings_json(),
            "America/New_York".into(),
        )
        .unwrap();
    assert_eq!(rows.len(), 1, "only layers with a block over the point");
    let row = &rows[0];
    assert_eq!(row.layer, "sst");
    // 16.111 °C at that grid point.
    assert_eq!(row.value_text, "61 °F");
    assert_eq!(row.time_text, "Nov 7");
    assert!(row.source.contains("MUR"));
    assert_eq!(row.label, "Sea surface temperature");

    let metric_rows = store
        .inspect(
            40.5,
            -73.5,
            vec!["sst".into()],
            metric(),
            "America/New_York".into(),
        )
        .unwrap();
    assert_eq!(metric_rows[0].value_text, "16.1 °C");

    // Land at the subset's corner has no value.
    let land = store
        .inspect(40.0, -74.2, vec!["sst".into()], plain(), "UTC".into())
        .unwrap();
    assert_eq!(land[0].value_text, "No value here");
}

#[test]
fn currents_are_arrows_and_read_as_speed_and_direction() {
    let store = GridStore::new();
    // 2026-11-08T08:00:00Z.
    let time = 1_794_124_800;
    let requests = harbor(&store, "hf-radar-currents", None, Some(time), NOW).unwrap();
    let url = &requests[0].request.url;
    assert!(
        url.contains("ucsdHfrE6.json?water_u%5B(2026-11-08T08:00:00Z)%5D"),
        "{url}"
    );
    assert!(url.contains(",water_v%5B"), "{url}");
    let body = fixture("erddap/ucsdhfre6-currents-subset-2026-11-08t08.json");
    store
        .add_grid(requests[0].key.clone(), response(200, body), NOW)
        .unwrap();
    let arrows = store
        .arrows(
            "hf-radar-currents".into(),
            None,
            None,
            None,
            -74.2,
            40.0,
            -73.0,
            40.8,
            9.0,
        )
        .unwrap();
    assert_ne!(arrows.len(), 0);
    for a in &arrows {
        assert!((0.0..=1.0).contains(&a.fraction));
        assert!((0.0..360.0).contains(&a.toward_deg));
        assert_eq!(a.colour.alpha, 255);
    }
    let rows = store
        .inspect(
            40.021_62,
            -74.014_946,
            vec!["hf-radar-currents".into()],
            default_settings_json(),
            "America/New_York".into(),
        )
        .unwrap();
    // 0.12 m/s east and 0.07 m/s north: 0.14 m/s toward 60°.
    assert_eq!(rows[0].value_text, "0.3 kn toward ENE 60°");
    assert_eq!(rows[0].time_text, "Nov 8, 3:00 AM");
    let raster = store.render_tile("hf-radar-currents".into(), None, None, None, 9, 150, 192);
    assert!(raster.is_err());
    let arrows = store.arrows(
        "sst".into(),
        None,
        None,
        None,
        -74.2,
        40.0,
        -73.0,
        40.8,
        9.0,
    );
    assert!(arrows.is_err());
}

#[test]
fn erddap_failures_come_back_as_errors() {
    let store = GridStore::new();
    let key = harbor(&store, "sst", None, None, NOW).unwrap()[0]
        .key
        .clone();
    let none = br#"Error {
    code=404;
    message="Not Found: Your query produced no matching results. (nRows = 0)";
}"#;
    assert_eq!(
        store.add_grid(key.clone(), response(404, none.to_vec()), NOW),
        Err(MapError::NoData)
    );
    assert_eq!(
        store.add_grid(key.clone(), response(503, Vec::new()), NOW),
        Err(MapError::RetryLater { after_s: 60 })
    );
    let mut busy = response(429, Vec::new());
    busy.retry_after = Some("120".into());
    assert_eq!(
        store.add_grid(key.clone(), busy, NOW),
        Err(MapError::RetryLater { after_s: 120 })
    );
    let mut html = response(200, b"<html></html>".to_vec());
    html.content_type = Some("text/html".into());
    assert!(matches!(
        store.add_grid(key, html, NOW),
        Err(MapError::Invalid { .. })
    ));
    for bad in [
        "sst/jplMURSST41/last/40",
        "radar/x/last/40/-74/1",
        "sst/jplMURSST41/now/40/-74/1",
        "sst/jplMURSST41/last/40/-74/0",
    ] {
        let added = store.add_grid(bad.into(), response(200, mur()), NOW);
        assert!(matches!(added, Err(MapError::Invalid { .. })), "{bad}");
    }
    assert_eq!(store.block_count(), 0);
}

#[test]
fn the_store_stays_within_its_bound_and_is_shared_across_threads() {
    let store = GridStore::new();
    harbor(&store, "sst", None, None, NOW).unwrap();
    let body = mur();
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let store = Arc::clone(&store);
            let body = body.clone();
            std::thread::spawn(move || {
                for i in 0..150 {
                    let key = format!("sst/jplMURSST41/last/{}/{}/1", t, -180 + i);
                    store
                        .add_grid(key, response(200, body.clone()), NOW)
                        .unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(store.block_count() as usize, MAX_STORE_BLOCKS);
    assert!(store.value_count() as usize <= MAX_STORE_VALUES);
    store.clear();
    assert_eq!((store.block_count(), store.value_count()), (0, 0));
}

#[test]
fn time_axes_are_requested_and_labelled() {
    let request = time_axis_request("clarity".into(), None, NOW).unwrap();
    assert_eq!(
        request.url,
        "https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?time%5B(2026-11-01):1:(last)%5D"
    );
    let hourly = time_axis_request("hf-radar-currents".into(), None, NOW).unwrap();
    assert!(hourly.url.contains("time%5B(2026-11-13):1:(last)%5D"));
    let body = fixture("erddap/noaacwn21viirskd490nswusdaily-time-2026-10-20-to-last.json");
    let times = parse_time_axis(
        "clarity".into(),
        None,
        response(200, body),
        plain(),
        "America/New_York".into(),
    )
    .unwrap();
    assert_eq!(times.first().unwrap().label, "Oct 21");
    let last = times.last().unwrap();
    assert_eq!((last.time, last.label.as_str()), (1_794_684_142, "Nov 14"));
    assert!(time_axis_request("radar".into(), None, NOW).is_err());
}

// ---------------------------------------------------------------------------------------------
// Pacing
// ---------------------------------------------------------------------------------------------

#[test]
fn the_pacer_allows_ten_a_minute_and_pauses_after_busy() {
    let pacer = GridPacer::new();
    for _ in 0..10 {
        assert!(pacer.may_send("pfeg".into(), NOW).unwrap().send);
    }
    let held = pacer.may_send("pfeg".into(), NOW + 5).unwrap();
    assert_eq!((held.send, held.wait_s), (false, 55));
    // Each server has its own minute.
    assert!(pacer.may_send("coastwatch".into(), NOW + 5).unwrap().send);
    assert!(pacer.may_send("pfeg".into(), NOW + 60).unwrap().send);

    pacer.record("glerl".into(), 503, None, NOW).unwrap();
    let paused = pacer.may_send("glerl".into(), NOW + 1).unwrap();
    assert_eq!((paused.send, paused.wait_s), (false, 59));
    pacer
        .record("glerl".into(), 429, Some(600), NOW + 61)
        .unwrap();
    assert_eq!(
        pacer.may_send("glerl".into(), NOW + 62).unwrap().wait_s,
        599
    );
    assert!(pacer.may_send("ncei".into(), NOW).is_err());
    assert!(pacer.record("erddap".into(), 200, None, NOW).is_err());
}

#[test]
fn the_pacer_state_round_trips_through_json() {
    let pacer = GridPacer::new();
    assert!(pacer.may_send("pfeg".into(), NOW).unwrap().send);
    pacer.record("coastwatch".into(), 503, None, NOW).unwrap();
    let json = pacer.to_json();
    assert_eq!(
        json,
        r#"{"nodes":{"pfeg":{"sent":[1794744000]},"coastwatch":{"paused_until":1794744060,"last_pause_s":60}}}"#
    );
    let back = GridPacer::from_json(json.clone()).unwrap();
    assert_eq!(back.to_json(), json);
    assert!(!back.may_send("coastwatch".into(), NOW + 30).unwrap().send);
    assert!(GridPacer::from_json("not json".into()).is_err());
    assert_eq!(
        GridPacer::from_json("{}".into()).unwrap().to_json(),
        r#"{"nodes":{}}"#
    );
}

// ---------------------------------------------------------------------------------------------
// Measuring and radar
// ---------------------------------------------------------------------------------------------

#[test]
fn a_measure_is_worded_in_the_users_units_with_its_notice() {
    // The Battery to Sandy Hook, about 25 km to the south.
    let battery = MapPoint {
        lat: 40.7006,
        lon: -74.0142,
    };
    let sandy_hook = MapPoint {
        lat: 40.4669,
        lon: -74.0094,
    };
    let us = measure(battery, sandy_hook, plain()).unwrap();
    assert_eq!(us.distance, "16 mi");
    assert_eq!(us.bearing, "S 179°");
    assert_eq!(us.line, "16 mi at S 179°");
    assert_eq!(us.notice, "Not for navigation.");
    let si = measure(battery, sandy_hook, metric()).unwrap();
    assert_eq!(si.distance, "26 km");
    let off = MapPoint {
        lat: 91.0,
        lon: 0.0,
    };
    assert!(measure(battery, off, plain()).is_err());
}

#[test]
fn the_radar_loop_is_labelled_in_the_users_clock() {
    let meta = radar_meta_request();
    assert_eq!(
        meta.url,
        "https://mesonet.agron.iastate.edu/data/gis/images/4326/USCOMP/n0q_0.json"
    );
    assert!(meta.fresh);
    let latest =
        radar_latest_valid(br#"{"meta": {"valid": "2026-10-10T20:15:00Z"}}"#.to_vec()).unwrap();
    assert!(radar_latest_valid(b"{}".to_vec()).is_err());
    let frames = radar_frames(latest, plain(), "America/New_York".into()).unwrap();
    assert_eq!(frames.len(), 6);
    let newest = frames.last().unwrap();
    assert_eq!((newest.minutes_ago, newest.valid), (0, latest));
    assert_eq!(newest.label, "4:15 PM");
    assert_eq!(frames[0].label, "3:50 PM");
    assert_eq!(
        newest.tile_template,
        "https://mesonet.agron.iastate.edu/c/tile.py/1.0.0/ridge::USCOMP-N0Q-202610102015/{z}/{x}/{y}.png"
    );
    let tile = radar_frame_request(latest, 8, 75, 96).unwrap().unwrap();
    assert_eq!(
        tile.url,
        "https://mesonet.agron.iastate.edu/c/tile.py/1.0.0/ridge::USCOMP-N0Q-202610102015/8/75/96.png"
    );
    assert_eq!(radar_frame_request(latest, 5, 9, 12).unwrap(), None);
    assert!(radar_frame_request(latest, 8, 300, 96).is_err());
    assert!(radar_covers(40.7, -74.0));
    assert!(!radar_covers(51.5, -0.12));
}
