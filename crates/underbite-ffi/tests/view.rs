//! `view_model` and `attributions` from the sample snapshot, offline, and the sample view
//! resource.

use std::path::PathBuf;

use serde_json::{Value, json};
use underbite_core::model::snapshot::testdata::sample_snapshot;
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::view::{SpotEntry, SpotInfo, ViewError, attributions, view_model};

fn spot() -> SpotInfo {
    let s = sample_snapshot().spot;
    SpotInfo {
        id: s.id,
        name: s.name,
        latitude: s.latitude,
        longitude: s.longitude,
        water_type: "tidal_river_estuary".into(),
        time_zone: s.time_zone,
    }
}

fn entry(json: Option<String>, last_success: Option<i64>) -> SpotEntry {
    SpotEntry {
        spot: spot(),
        snapshot_json: json,
        last_success,
        error: None,
        refreshing: false,
    }
}

fn snapshot_json() -> String {
    serde_json::to_string(&sample_snapshot()).unwrap()
}

fn page(settings: &str, spots: Vec<SpotEntry>, now: i64) -> Value {
    let json = view_model(spots, 0, settings.into(), now).unwrap();
    serde_json::from_str(&json).unwrap()
}

fn fetched() -> i64 {
    sample_snapshot().fetched_at.0
}

#[test]
fn the_page_holds_cards_timeline_and_status() {
    let at = fetched();
    let p = page(
        &default_settings_json(),
        vec![entry(Some(snapshot_json()), Some(at))],
        at + 360,
    );
    assert_eq!(p["active"], 0);
    assert_eq!(p["spots"][0]["name"], "The Battery");
    assert_eq!(p["spots"][0]["water_type_label"], "Tidal river or estuary");
    assert_eq!(p["spots"][0]["status"]["updated"], "updated 6 min ago");
    let view = &p["view"];
    assert_eq!(view["header"]["name"], "The Battery");
    assert!(view["cards"].as_array().is_some_and(|c| c.len() > 3));
    assert!(view["cards"][0]["card"].is_string());
    assert!(view["timeline"].is_object());
    let text = view.to_string().to_lowercase();
    assert!(text.contains("not for navigation"));
}

#[test]
fn units_voice_and_card_order_come_from_the_settings() {
    let at = fetched();
    let mut doc: Value = serde_json::from_str(&default_settings_json()).unwrap();
    doc["units"]["height"] = json!("m");
    doc["units"]["water_temp"] = json!("C");
    doc["presentation"]["voice"] = json!("plain");
    doc["cards"]["tidal"] = json!({ "order": ["sources", "tide"], "hidden": [] });
    let p = page(
        &doc.to_string(),
        vec![entry(Some(snapshot_json()), Some(at))],
        at,
    );
    let cards = p["view"]["cards"].as_array().unwrap();
    let ids: Vec<_> = cards.iter().map(|c| c["card"].as_str().unwrap()).collect();
    assert_eq!(ids, ["sources", "tide"]);
    let tide = cards[1].to_string();
    assert!(tide.contains("\"unit\":\"m"), "{tide}");
    assert!(!tide.contains("\"unit\":\"ft"), "{tide}");
}

#[test]
fn a_spot_without_a_snapshot_has_no_page_and_the_list_still_shows() {
    let mut e = entry(None, None);
    e.error = Some("offline".into());
    e.refreshing = true;
    let p = page(&default_settings_json(), vec![e], 1_800_000_000);
    assert_eq!(p["view"], Value::Null);
    assert_eq!(p["spots"][0]["status"]["error"], "offline");
    assert_eq!(p["spots"][0]["status"]["refreshing"], true);
    assert_eq!(p["spots"][0]["status"]["updated"], Value::Null);
}

#[test]
fn the_active_spot_is_the_one_shown_and_a_rename_shows_at_once() {
    let at = fetched();
    let mut renamed = entry(Some(snapshot_json()), Some(at));
    renamed.spot.name = "Pier 1".into();
    let spots = vec![entry(None, None), renamed];
    let json = view_model(spots, 1, default_settings_json(), at).unwrap();
    let p: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(p["active"], 1);
    assert_eq!(p["view"]["header"]["name"], "Pier 1");
    // Past the end of the list there is nothing to show.
    let json = view_model(vec![entry(None, None)], 5, default_settings_json(), at).unwrap();
    let p: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(p["view"], Value::Null);
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let at = fetched();
    let bad_snapshot = view_model(
        vec![entry(Some("{".into()), None)],
        0,
        default_settings_json(),
        at,
    );
    assert!(matches!(bad_snapshot, Err(ViewError::Invalid { .. })));
    let bad_settings = view_model(Vec::new(), 0, "{\"units\":3}".into(), at);
    assert!(matches!(bad_settings, Err(ViewError::Settings { .. })));
    let mut bad_type = entry(None, None);
    bad_type.spot.water_type = "swamp".into();
    let r = view_model(vec![bad_type], 0, default_settings_json(), at);
    assert!(matches!(r, Err(ViewError::Invalid { .. })));
}

#[test]
fn attributions_list_the_credits_of_a_snapshot() {
    let credits = attributions(snapshot_json()).unwrap();
    assert_ne!(credits, []);
    assert!(
        credits
            .iter()
            .all(|c| !c.source.is_empty() && !c.text.is_empty())
    );
    assert!(credits.iter().any(|c| c.text.contains("CO-OPS")));
    assert!(attributions("nope".into()).is_err());
}

fn resource(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../ios/UnderbiteKit/Tests/UnderbiteKitTests/Resources")
        .join(name)
}

/// The file holds the Battery page the app's tests read. `UNDERBITE_WRITE_SAMPLES=1 cargo test`
/// rewrites it.
#[test]
fn sample_view_is_current() {
    let at = fetched();
    let p = page(
        &default_settings_json(),
        vec![entry(Some(snapshot_json()), Some(at))],
        at + 360,
    );
    let json = serde_json::to_string_pretty(&p).unwrap() + "\n";
    let path = resource("sample-view.json");
    if std::env::var_os("UNDERBITE_WRITE_SAMPLES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &json).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        kept, json,
        "run `UNDERBITE_WRITE_SAMPLES=1 cargo test -p underbite-ffi`"
    );
}
