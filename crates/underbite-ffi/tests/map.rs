//! The map calls: the worded catalogue and presets, tile requests and their classification, and
//! satellite dates.

use serde_json::{Value, json};
use underbite_ffi::fetch::HostMethod;
use underbite_ffi::map::{
    LayerDates, LayerDraw, MapError, TileOutcome, classify_tile, gibs_default_date, map_catalogue,
    map_presets, map_tile_size, previous_date, tile_request,
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
