//! Widget and Live Activity payloads from the recorded scenarios' snapshots: The Battery's tide,
//! the Croton's flow, a trip's go/no-go and its day's Live Activity, and the next trip.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use underbite_core::time::parse_rfc3339;
use underbite_ffi::fetch::{
    FetchHost, FetchOptions, HostError, HostRequest, HostResponse, Keys, fetch_spot,
};
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::trip::TripPlaceData;
use underbite_ffi::view::SpotInfo;
use underbite_ffi::widgets::{WidgetError, next_trip, spot_widget, trip_activity, trip_widget};

fn now() -> i64 {
    parse_rfc3339("2026-11-08T12:00:00Z").expect("time").0
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

// ---------------------------------------------------------------------------------------------
// A host answering from the recorded scenarios
// ---------------------------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Route {
    #[serde(rename = "match")]
    matches: String,
    file: PathBuf,
    status: Option<u16>,
}

#[derive(serde::Deserialize)]
struct Routes {
    route: Vec<Route>,
}

struct Replay {
    base: PathBuf,
    routes: Vec<Route>,
    values: Mutex<BTreeMap<(String, String), String>>,
}

impl Replay {
    fn new(scenario: &str) -> Arc<Self> {
        let base = fixtures().join("replay").join(scenario);
        let text = std::fs::read_to_string(base.join("routes.toml")).expect("routes");
        let routes: Routes = toml::from_str(&text).expect("routes file");
        Arc::new(Self {
            base,
            routes: routes.route,
            values: Mutex::new(BTreeMap::new()),
        })
    }
}

#[async_trait::async_trait]
impl FetchHost for Replay {
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
        let route = self
            .routes
            .iter()
            .find(|r| request.url.contains(&r.matches))
            .ok_or_else(|| HostError::Failed {
                message: "no route".into(),
            })?;
        let path = self.base.join(&route.file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Ok(HostResponse {
            status: route.status.unwrap_or(200),
            body,
            content_type: Some("application/json".into()),
            date: None,
            expires: None,
            last_modified: None,
            etag: None,
            retry_after: None,
            rate_limit_remaining: None,
        })
    }

    fn load(&self, namespace: String, key: String) -> Option<String> {
        self.values.lock().unwrap().get(&(namespace, key)).cloned()
    }

    fn store(&self, namespace: String, key: String, value: String) {
        self.values.lock().unwrap().insert((namespace, key), value);
    }

    fn save_usage(&self, ledger_json: String) -> Result<(), HostError> {
        self.store("usage".into(), "ledger".into(), ledger_json);
        Ok(())
    }
}

fn spot(id: &str, name: &str, latitude: f64, longitude: f64, water_type: &str) -> SpotInfo {
    SpotInfo {
        id: id.into(),
        name: name.into(),
        latitude,
        longitude,
        water_type: water_type.into(),
        time_zone: "America/New_York".into(),
    }
}

fn battery() -> SpotInfo {
    spot(
        "battery",
        "The Battery",
        40.7033,
        -74.0170,
        "tidal_river_estuary",
    )
}

fn croton() -> SpotInfo {
    spot("croton", "Croton River", 41.2257, -73.8618, "river_stream")
}

fn snapshot(scenario: &str, spot: SpotInfo) -> String {
    pollster::block_on(fetch_spot(
        spot,
        default_settings_json(),
        Keys::default(),
        FetchOptions {
            allow_metered: true,
            infer_water_type: false,
        },
        now(),
        Vec::new(),
        Replay::new(scenario),
    ))
    .expect("fetched")
    .snapshot_json
}

fn trip(id: &str, places: &[&SpotInfo], first: &str, last: &str) -> String {
    let places: Vec<Value> = places
        .iter()
        .map(|s| {
            json!({
                "spot_id": s.id,
                "name": s.name,
                "latitude": s.latitude,
                "longitude": s.longitude,
                "time_zone": s.time_zone,
                "water_type": s.water_type,
            })
        })
        .collect();
    json!({
        "id": id,
        "name": "Harbor weekend",
        "places": places,
        "first": first,
        "last": last,
        "method": "shore",
        "modified_at": now(),
    })
    .to_string()
}

fn place(key: &str, snapshot_json: Option<String>) -> TripPlaceData {
    TripPlaceData {
        key: key.into(),
        snapshot_json,
        history_json: None,
    }
}

fn parse(json: &str) -> Value {
    serde_json::from_str(json).expect("payload json")
}

/// Whether a payload carries a spot's coordinates.
fn has_coordinates(json: &str) -> bool {
    ["40.7033", "74.017", "41.2257", "73.8618"]
        .iter()
        .any(|c| json.contains(c))
}

// The tests are named `widgets::*`, as the widgets filter selects them.
mod widgets {
    use super::*;

    // -----------------------------------------------------------------------------------------
    // Spots
    // -----------------------------------------------------------------------------------------

    #[test]
    fn the_battery_widget_has_its_tide_every_hour_with_its_credit() {
        let json = spot_widget(
            battery(),
            snapshot("battery", battery()),
            default_settings_json(),
            now(),
        )
        .expect("payload");
        assert!(!has_coordinates(&json));
        let p = parse(&json);
        assert_eq!(p["subject"], "spot");
        assert_eq!(p["id"], "battery");
        assert_eq!(p["name"], "The Battery");
        assert_eq!(p["generated_at"], now());
        assert!(p["places"][0]["tide_chart"].is_object(), "{p}");
        let entries = p["entries"].as_array().expect("entries");
        assert!(entries.len() >= 12, "{}", entries.len());
        assert_eq!(entries[0]["t"], now());
        for e in entries {
            let tide = &e["places"][0]["tide"];
            assert!(tide.is_object(), "{e}");
            assert!(
                tide["card"]["credit"]
                    .as_str()
                    .is_some_and(|c| c.contains("NOAA")),
                "{tide}"
            );
        }
    }

    #[test]
    fn the_croton_widget_has_its_flow_in_the_settings_units() {
        let json = spot_widget(
            croton(),
            snapshot("croton", croton()),
            default_settings_json(),
            now(),
        )
        .expect("payload");
        assert!(!has_coordinates(&json));
        let p = parse(&json);
        let flow = &p["entries"][0]["places"][0]["flow"];
        assert!(flow.is_object(), "{p}");
        assert!(flow.to_string().contains("cfs"), "{flow}");
        assert!(p["entries"][0]["places"][0]["tide"].is_null());

        // Metric settings give a metric payload.
        let mut settings = parse(&default_settings_json());
        settings["units"]["discharge"] = json!("m3s");
        let metric = spot_widget(
            croton(),
            snapshot("croton", croton()),
            settings.to_string(),
            now(),
        )
        .expect("payload");
        assert!(
            !parse(&metric)["entries"][0]["places"][0]["flow"]
                .to_string()
                .contains("cfs")
        );
    }

    #[test]
    fn the_app_s_name_for_the_spot_wins_and_bad_input_is_refused() {
        let mut renamed = croton();
        renamed.name = "Home water".into();
        let json = spot_widget(
            renamed,
            snapshot("croton", croton()),
            default_settings_json(),
            now(),
        )
        .unwrap();
        assert_eq!(parse(&json)["name"], "Home water");
        assert!(matches!(
            spot_widget(croton(), "{".into(), default_settings_json(), now()),
            Err(WidgetError::Invalid { .. })
        ));
        assert!(matches!(
            spot_widget(croton(), "{}".into(), "{\"units\":3}".into(), now()),
            Err(WidgetError::Settings { .. })
        ));
    }

    // -----------------------------------------------------------------------------------------
    // Trips
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_trip_widget_has_its_go_no_go_and_its_places() {
        let t = trip(
            "harbor",
            &[&battery(), &croton()],
            "2026-11-08",
            "2026-11-10",
        );
        let places = vec![
            place("battery", Some(snapshot("battery", battery()))),
            place("croton", Some(snapshot("croton", croton()))),
        ];
        let json = trip_widget(t, places, default_settings_json(), now()).expect("payload");
        assert!(!has_coordinates(&json));
        let p = parse(&json);
        assert_eq!(p["subject"], "trip");
        assert_eq!(p["id"], "harbor");
        assert_eq!(p["places"].as_array().map(Vec::len), Some(2));
        let first = &p["entries"][0];
        assert!(first["trip"]["compact"].is_object(), "{first}");
        assert_eq!(first["places"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn a_trip_without_snapshots_still_has_a_payload() {
        let t = trip("harbor", &[&battery()], "2026-11-08", "2026-11-09");
        let p = parse(
            &trip_widget(
                t.clone(),
                vec![place("battery", None)],
                default_settings_json(),
                now(),
            )
            .unwrap(),
        );
        assert_eq!(p["places"].as_array().map(Vec::len), Some(0));
        assert!(p["entries"][0]["trip"].is_object());
        let a = parse(&trip_activity(t, Vec::new(), default_settings_json(), now()).unwrap());
        assert_eq!(a["title"], "Harbor weekend");
        assert!(a["next_tide_change"].is_null());
    }

    #[test]
    fn the_trip_day_activity_counts_down_to_the_next_tide_change() {
        let t = trip("harbor", &[&battery()], "2026-11-08", "2026-11-08");
        let json = trip_activity(
            t,
            vec![place("battery", Some(snapshot("battery", battery())))],
            default_settings_json(),
            now(),
        )
        .expect("activity");
        assert!(!has_coordinates(&json));
        let a = parse(&json);
        assert_eq!(a["title"], "Harbor weekend");
        assert_eq!(a["place"], "The Battery");
        let next = &a["next_tide_change"];
        assert!(next["at"].as_i64().is_some_and(|t| t > now()), "{a}");
        assert!(next["label"].as_str().is_some_and(|l| !l.is_empty()));
        assert!(a["ends_at"].as_i64().is_some_and(|t| t > now()));
        assert!(
            a["credit"].as_str().is_some_and(|c| c.contains("NOAA")),
            "{a}"
        );
    }

    #[test]
    fn the_next_trip_is_the_soonest_not_over() {
        let over = trip("over", &[&battery()], "2026-11-01", "2026-11-07");
        let under_way = trip("now", &[&battery()], "2026-11-07", "2026-11-09");
        let later = trip("later", &[&croton()], "2026-12-01", "2026-12-02");
        assert_eq!(
            next_trip(
                vec![later.clone(), "{".into(), over.clone(), under_way],
                now()
            )
            .as_deref(),
            Some("now")
        );
        assert_eq!(
            next_trip(vec![later, over.clone()], now()).as_deref(),
            Some("later")
        );
        assert_eq!(next_trip(vec![over], now()), None);
        assert_eq!(next_trip(Vec::new(), now()), None);
    }
}
