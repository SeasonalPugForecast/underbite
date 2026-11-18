//! Alert rules as the app runs them: editing them in the settings, the background plan, the
//! narrowed fetch answered from the recorded scenarios, evaluation with its words and state,
//! and the rules on the dashboard's alerts card.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use underbite_core::model::StormWatch;
use underbite_core::model::snapshot::testdata::sample_snapshot;
use underbite_core::time::parse_rfc3339;
use underbite_ffi::alerts::{
    AlertError, AlertGridResponse, AlertRuleState, AlertSpotData, AlertStep, AlertTargetKind,
    NamedPlace, alert_metrics, alert_rules, alert_texts, alert_threshold_text, describe_alert_rule,
    evaluate_alerts, fetch_alert_spot, parse_alert_threshold, plan_alerts, put_alert_rule,
    remove_alert_rule, set_alert_rule_enabled,
};
use underbite_ffi::fetch::{
    FetchHost, FetchOptions, HostError, HostRequest, HostResponse, Keys, fetch_spot,
    stored_stations,
};
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::view::{SpotEntry, SpotInfo, view_model};
use whether_core::storm::testdata as storm_data;

fn now() -> i64 {
    parse_rfc3339("2026-11-08T12:00:00Z").expect("time").0
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn raw(path: &str) -> Vec<u8> {
    let path = fixtures().join("raw").join(path);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// A host answering from the recorded scenarios, noting every URL asked for
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
    asked: Mutex<Vec<String>>,
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
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<String> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

fn response(status: u16, body: Vec<u8>) -> HostResponse {
    HostResponse {
        status,
        body,
        content_type: Some("application/json".into()),
        date: None,
        expires: None,
        last_modified: None,
        etag: None,
        retry_after: None,
        rate_limit_remaining: None,
    }
}

#[async_trait::async_trait]
impl FetchHost for Replay {
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
        self.asked.lock().unwrap().push(request.url.clone());
        let route = self
            .routes
            .iter()
            .find(|r| request.url.contains(&r.matches))
            .ok_or_else(|| HostError::Failed {
                message: "no route".into(),
            })?;
        let path = self.base.join(&route.file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Ok(response(route.status.unwrap_or(200), body))
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

// ---------------------------------------------------------------------------------------------
// Places, rules and settings
// ---------------------------------------------------------------------------------------------

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

fn named(spots: &[&SpotInfo]) -> Vec<NamedPlace> {
    spots
        .iter()
        .map(|s| NamedPlace {
            id: s.id.clone(),
            name: s.name.clone(),
        })
        .collect()
}

/// 800 cfs in m³/s.
const CFS_800: f64 = 22.653_47;

fn flow_rule(id: &str, below: f64) -> Value {
    json!({
        "id": id,
        "name": "Croton low water",
        "target": { "kind": "spot", "id": "croton" },
        "all_of": [{ "kind": "measurement", "metric": "discharge", "below": below }],
    })
}

fn storm_rule(id: &str, target: &Value) -> Value {
    json!({
        "id": id,
        "name": "Storm watch",
        "target": target,
        "all_of": [{ "kind": "storm_ahead" }],
    })
}

fn box_rule(id: &str) -> Value {
    json!({
        "id": id,
        "name": "Warm water off the Rockaways",
        "target": { "kind": "box", "south": 40.123, "west": -73.987, "north": 40.6, "east": -73.5 },
        "all_of": [{ "kind": "measurement", "metric": "sea_surface_temp", "above": 10.0 }],
    })
}

/// The default settings with `rules`, saved one by one as the editor does.
fn settings_with(rules: &[Value]) -> String {
    rules
        .iter()
        .fold(default_settings_json(), |settings, rule| {
            put_alert_rule(settings, rule.to_string()).expect("saved")
        })
}

fn trip(id: &str, place: &SpotInfo, first: &str, last: &str) -> String {
    json!({
        "id": id,
        "name": "Harbor weekend",
        "places": [{
            "spot_id": place.id,
            "name": place.name,
            "latitude": place.latitude,
            "longitude": place.longitude,
            "time_zone": place.time_zone,
            "water_type": place.water_type,
        }],
        "first": first,
        "last": last,
        "method": "shore",
        "modified_at": now(),
    })
    .to_string()
}

/// The spot's full snapshot from its recorded scenario at the test's time.
fn snapshot(scenario: &str, spot: SpotInfo) -> String {
    let out = pollster::block_on(fetch_spot(
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
    .expect("fetched");
    out.snapshot_json
}

fn data(spot: SpotInfo, snapshot_json: Option<String>) -> AlertSpotData {
    AlertSpotData {
        spot,
        snapshot_json,
    }
}

// The tests are named `alerts::*`, as the alerts filter selects them.
mod alerts {
    use super::*;

    // -----------------------------------------------------------------------------------------
    // Editing
    // -----------------------------------------------------------------------------------------

    #[test]
    fn the_rules_screen_says_that_ios_decides_when_checks_run() {
        let t = alert_texts(default_settings_json()).unwrap();
        assert!(t.background_timing.contains("when iOS allows it"));
        assert_eq!(t.check_now, "Check now");
        assert!(t.notifications_off.contains("Notifications are off"));
        assert!(alert_texts("{".to_owned()).is_err());
    }

    #[test]
    fn rules_are_saved_listed_turned_off_and_removed_through_the_settings() {
        let settings = settings_with(&[flow_rule("low", CFS_800)]);
        let doc: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(doc["alerts"][0]["id"], "low");

        let list = |settings: &str| {
            alert_rules(
                settings.to_owned(),
                named(&[&croton()]),
                Vec::new(),
                None,
                "America/New_York".into(),
                now(),
            )
            .expect("rows")
        };
        let rows = list(&settings);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "Croton low water");
        assert_eq!(row.target, AlertTargetKind::Spot);
        assert_eq!(row.target_id.as_deref(), Some("croton"));
        assert!(!row.orphaned);
        assert_eq!(row.words.target, "Croton River");
        assert_eq!(row.words.summary, "River flow under 800 cfs");
        assert_eq!(row.state, AlertRuleState::NotChecked);
        assert!(row.rule_json.contains("\"discharge\""));

        // Saving under the same id replaces the rule.
        let mut changed = flow_rule("low", CFS_800 / 2.0);
        changed["name"] = json!("Croton very low");
        let settings = put_alert_rule(settings, changed.to_string()).unwrap();
        let rows = list(&settings);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Croton very low");
        assert_eq!(rows[0].words.summary, "River flow under 400 cfs");

        let off = set_alert_rule_enabled(settings.clone(), "low".into(), false).unwrap();
        let rows = list(&off);
        assert!(!rows[0].enabled);
        assert_eq!(rows[0].state, AlertRuleState::Off);

        let gone = remove_alert_rule(settings, "low".into()).unwrap();
        assert_eq!(list(&gone).len(), 0);
    }

    #[test]
    fn a_rule_with_an_error_is_refused_with_its_path() {
        let mut bad = flow_rule("low", CFS_800);
        bad["all_of"] = json!([]);
        let err = put_alert_rule(default_settings_json(), bad.to_string()).unwrap_err();
        assert!(
            matches!(&err, AlertError::Invalid { what, .. } if what == "alerts[0].all_of"),
            "{err:?}"
        );
        let unknown = json!({
            "id": "x",
            "name": "x",
            "target": { "kind": "lake", "id": "a" },
            "all_of": [],
        });
        assert!(matches!(
            put_alert_rule(default_settings_json(), unknown.to_string()),
            Err(AlertError::Invalid { .. })
        ));
    }

    #[test]
    fn a_rule_whose_spot_is_gone_is_orphaned_and_a_box_never_is() {
        let settings = settings_with(&[flow_rule("low", CFS_800), box_rule("warm")]);
        let rows = alert_rules(
            settings.clone(),
            Vec::new(),
            Vec::new(),
            None,
            "UTC".into(),
            now(),
        )
        .unwrap();
        assert!(rows[0].orphaned);
        assert_eq!(rows[0].state, AlertRuleState::Orphaned);
        assert_eq!(rows[0].checked, None);
        assert!(!rows[1].orphaned);
        assert_eq!(rows[1].target, AlertTargetKind::Box);
        // A box is never shown as coordinates.
        assert!(!rows[1].words.target.contains("40."));

        let words = describe_alert_rule(
            box_rule("warm").to_string(),
            settings,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(words.conditions.len(), 1);
        assert!(!words.target.contains("73."));
    }

    #[test]
    fn thresholds_are_typed_in_the_settings_units() {
        let settings = default_settings_json();
        let metrics = alert_metrics(settings.clone()).unwrap();
        assert_eq!(metrics.len(), 10);
        let discharge = metrics.iter().find(|m| m.metric == "discharge").unwrap();
        assert_eq!(discharge.unit, "cfs");
        assert!(!discharge.in_boxes);
        assert!(
            metrics
                .iter()
                .find(|m| m.metric == "sea_surface_temp")
                .unwrap()
                .in_boxes
        );

        let v = parse_alert_threshold("discharge".into(), "800".into(), settings.clone())
            .unwrap()
            .unwrap();
        assert!((v - CFS_800).abs() < 0.01, "{v}");
        assert_eq!(
            alert_threshold_text("discharge".into(), v, settings.clone()).unwrap(),
            "800"
        );
        assert_eq!(
            parse_alert_threshold("discharge".into(), "  ".into(), settings.clone()).unwrap(),
            None
        );
        let refused = parse_alert_threshold("discharge".into(), "lots".into(), settings.clone());
        assert!(
            matches!(&refused, Err(AlertError::Threshold { message }) if !message.is_empty()),
            "{refused:?}"
        );
        assert!(matches!(
            parse_alert_threshold("depth".into(), "3".into(), settings),
            Err(AlertError::Invalid { .. })
        ));
    }

    // -----------------------------------------------------------------------------------------
    // The plan and the fetch
    // -----------------------------------------------------------------------------------------

    #[test]
    fn the_plan_puts_safety_first_and_asks_only_for_what_rules_read() {
        let mut off = flow_rule("off", CFS_800);
        off["enabled"] = json!(false);
        off["target"] = json!({ "kind": "spot", "id": "elsewhere" });
        let settings = settings_with(&[
            flow_rule("low", CFS_800),
            box_rule("warm"),
            storm_rule("storm", &json!({ "kind": "spot", "id": "battery" })),
            off,
            storm_rule("gone", &json!({ "kind": "spot", "id": "deleted" })),
        ]);
        let elsewhere = spot("elsewhere", "Elsewhere", 41.0, -74.0, "river_stream");
        let steps = plan_alerts(
            settings,
            vec![battery(), croton(), elsewhere],
            Vec::new(),
            now(),
        )
        .unwrap();
        assert_eq!(steps.len(), 3, "{steps:?}");

        // The storm watch first, then the flow, then the box (in the settings' order).
        let AlertStep::Spot { fetch: storm } = &steps[0] else {
            panic!("{:?}", steps[0]);
        };
        assert_eq!(storm.spot.id, "battery");
        assert_eq!(storm.rules, ["storm"]);
        assert_eq!(storm.priority, 0);
        assert_eq!(storm.measurements, ["marine_forecast", "marine_alerts"]);

        let AlertStep::Spot { fetch: flow } = &steps[1] else {
            panic!("{:?}", steps[1]);
        };
        assert_eq!(flow.spot.id, "croton");
        assert_eq!(flow.priority, 1);
        assert!(flow.measurements.contains(&"discharge".into()));
        assert!(!flow.measurements.contains(&"turbidity".into()));
        assert!(!flow.measurements.contains(&"water_temperature".into()));

        let AlertStep::Grid { grid } = &steps[2] else {
            panic!("{:?}", steps[2]);
        };
        assert_eq!(grid.rule, "warm");
        assert_eq!(grid.metric, "sea_surface_temp");
        assert_eq!(grid.node, "pfeg");
        let url = &grid.request.url;
        assert!(url.contains("jplMURSST41"), "{url}");
        // Rounded outward to two decimals, never the box as typed.
        assert!(url.contains("(40.12)") && url.contains("(-73.99)"), "{url}");
        assert!(!url.contains("40.123") && !url.contains("73.987"), "{url}");
    }

    #[test]
    fn a_trip_rule_fetches_the_trips_places_only_while_a_forecast_reaches_it() {
        let settings = settings_with(&[storm_rule(
            "trip-storm",
            &json!({ "kind": "trip", "id": "harbor" }),
        )]);
        let soon = trip("harbor", &battery(), "2026-11-09", "2026-11-10");
        let steps = plan_alerts(settings.clone(), vec![battery()], vec![soon], now()).unwrap();
        assert!(
            matches!(&steps[..], [AlertStep::Spot { fetch }] if fetch.spot.id == "battery"
            && fetch.rules == ["trip-storm"])
        );
        let far = trip("harbor", &battery(), "2027-06-01", "2027-06-03");
        assert_eq!(
            plan_alerts(settings, vec![battery()], vec![far], now())
                .unwrap()
                .len(),
            0
        );
    }

    /// Whether `url` carries the spot's coordinates beyond two decimals.
    fn exact(url: &str) -> bool {
        ["41.2257", "41.226", "73.8618", "73.862"]
            .iter()
            .any(|c| url.contains(c))
    }

    #[test]
    fn the_alert_fetch_is_smaller_rounds_coordinates_and_keeps_the_snapshot_whole() {
        let settings = settings_with(&[flow_rule("low", CFS_800)]);
        let host = Replay::new("croton");
        let full = pollster::block_on(fetch_spot(
            croton(),
            settings.clone(),
            Keys::default(),
            FetchOptions {
                allow_metered: true,
                infer_water_type: false,
            },
            now(),
            Vec::new(),
            host.clone(),
        ))
        .expect("full fetch");
        let full_asked = host.asked();
        let stations = stored_stations("croton".into(), host.clone());
        assert!(
            stations
                .as_ref()
                .is_some_and(|s| s.contains("water_temperature"))
        );

        let steps = plan_alerts(settings.clone(), vec![croton()], Vec::new(), now()).unwrap();
        let AlertStep::Spot { fetch } = steps.into_iter().next().unwrap() else {
            panic!("a spot step");
        };
        let later = now() + 3_600;
        let out = pollster::block_on(fetch_alert_spot(
            fetch,
            settings,
            Keys::default(),
            true,
            later,
            Vec::new(),
            host.clone(),
        ))
        .expect("alert fetch");
        let asked = host.asked();
        assert_ne!(asked.len(), 0);
        assert!(
            asked.len() < full_asked.len(),
            "{} requests, the full fetch {}",
            asked.len(),
            full_asked.len()
        );
        assert!(
            !asked
                .iter()
                .any(|u| u.contains("turbidity") || u.contains("63680")),
            "{asked:#?}"
        );
        for url in asked.iter().chain(&full_asked) {
            assert!(!exact(url), "{url}");
        }

        // What the alert fetch did not ask for is still there, from the full snapshot.
        let whole: Value = serde_json::from_str(&full.snapshot_json).unwrap();
        let merged: Value = serde_json::from_str(&out.snapshot_json).unwrap();
        for (key, value) in whole["water"].as_object().unwrap() {
            if !value.is_null() {
                assert!(!merged["water"][key].is_null(), "water.{key} lost");
            }
        }
        assert!(!merged["water"]["discharge_m3s"].is_null());
        // And it is the snapshot kept for the next fetch.
        let kept = host.load("spot".into(), underbite_fetch::store::spot_key("croton"));
        assert!(kept.is_some_and(|k| k.contains("discharge_m3s")));
        // The station choices still list every measurement the full fetch ranked.
        assert_eq!(stored_stations("croton".into(), host.clone()), stations);
    }

    // -----------------------------------------------------------------------------------------
    // Evaluation
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_met_rule_notifies_once_in_the_users_units_without_coordinates() {
        // Any flow at all: the rule is met by the recorded value.
        let mut rule = flow_rule("flowing", 0.0);
        rule["all_of"] = json!([{ "kind": "measurement", "metric": "discharge", "above": 0.01 }]);
        let settings = settings_with(&[rule]);
        let snapshot = snapshot("croton", croton());
        let run = |state: Option<Vec<u8>>| {
            evaluate_alerts(
                settings.clone(),
                vec![data(croton(), Some(snapshot.clone()))],
                Vec::new(),
                Vec::new(),
                state,
                "America/New_York".into(),
                now(),
            )
            .expect("run")
        };
        let first = run(None);
        assert_eq!(first.notifications.len(), 1, "{first:?}");
        let n = &first.notifications[0];
        assert_eq!(n.rule_id, "flowing");
        assert_eq!(n.title, "Croton low water");
        assert!(n.body.contains("cfs"), "{}", n.body);
        assert!(!n.time_sensitive);
        assert!(
            !n.body.contains("41.2") && !n.body.contains("73.8"),
            "{}",
            n.body
        );

        // The state remembers it: no second notification while the rule stays met.
        let second = run(Some(first.state.clone()));
        assert!(second.notifications.is_empty(), "{second:?}");

        // The list and the dashboard read the same state.
        let rows = alert_rules(
            settings.clone(),
            named(&[&croton()]),
            Vec::new(),
            Some(second.state.clone()),
            "America/New_York".into(),
            now(),
        )
        .unwrap();
        assert_eq!(rows[0].state, AlertRuleState::Met);
        assert!(rows[0].state_label.starts_with("Met since"), "{rows:?}");

        let entry = SpotEntry {
            spot: croton(),
            snapshot_json: Some(snapshot),
            last_success: Some(now()),
            error: None,
            refreshing: false,
        };
        let page = view_model(
            vec![entry],
            0,
            settings,
            now(),
            Some(second.state),
            Vec::new(),
        )
        .unwrap();
        assert!(page.contains("Croton low water"), "{page}");
        assert!(page.contains("Met since"));
    }

    #[test]
    fn a_hidden_name_stays_off_the_lock_screen() {
        let mut rule = flow_rule("flowing", 0.0);
        rule["all_of"] = json!([{ "kind": "measurement", "metric": "discharge", "above": 0.01 }]);
        rule["show_name"] = json!(false);
        let run = evaluate_alerts(
            settings_with(&[rule]),
            vec![data(croton(), Some(snapshot("croton", croton())))],
            Vec::new(),
            Vec::new(),
            None,
            "America/New_York".into(),
            now(),
        )
        .unwrap();
        let n = &run.notifications[0];
        assert!(!n.title.contains("Croton"), "{n:?}");
        assert!(!n.body.contains("Croton"), "{n:?}");
    }

    #[test]
    fn a_storm_is_time_sensitive_and_written_plainly() {
        let at = storm_data::STORM_NOW;
        let mut sample = sample_snapshot();
        sample.storm = Some(StormWatch::from_outlook(
            at,
            storm_data::noreaster(),
            Vec::new(),
        ));
        let place = SpotInfo {
            id: sample.spot.id.clone(),
            name: sample.spot.name.clone(),
            latitude: sample.spot.latitude,
            longitude: sample.spot.longitude,
            water_type: "tidal_river_estuary".into(),
            time_zone: sample.spot.time_zone.clone(),
        };
        let target = json!({ "kind": "spot", "id": sample.spot.id });
        let notice = |voice: &str| {
            let mut settings: Value =
                serde_json::from_str(&settings_with(&[storm_rule("storm", &target)])).unwrap();
            settings["presentation"]["voice"] = json!(voice);
            let run = evaluate_alerts(
                settings.to_string(),
                vec![data(
                    place.clone(),
                    Some(serde_json::to_string(&sample).unwrap()),
                )],
                Vec::new(),
                Vec::new(),
                None,
                "UTC".into(),
                at.0,
            )
            .unwrap();
            assert_eq!(run.notifications.len(), 1, "{run:?}");
            run.notifications[0].clone()
        };
        let luna = notice("luna");
        assert!(luna.time_sensitive);
        assert_eq!(luna, notice("plain"));
    }

    #[test]
    fn a_trip_rule_is_judged_at_its_places_and_a_missing_spot_is_orphaned() {
        let settings = settings_with(&[
            storm_rule("trip-storm", &json!({ "kind": "trip", "id": "harbor" })),
            storm_rule("lost", &json!({ "kind": "spot", "id": "deleted" })),
        ]);
        let harbor = trip("harbor", &battery(), "2026-11-09", "2026-11-10");
        let run = evaluate_alerts(
            settings.clone(),
            vec![data(battery(), Some(snapshot("battery", battery())))],
            vec![harbor.clone()],
            Vec::new(),
            None,
            "America/New_York".into(),
            now(),
        )
        .unwrap();
        let rows = alert_rules(
            settings.clone(),
            named(&[&battery()]),
            vec![NamedPlace {
                id: "harbor".into(),
                name: "Harbor weekend".into(),
            }],
            Some(run.state.clone()),
            "America/New_York".into(),
            now(),
        )
        .unwrap();
        assert_ne!(rows[0].state, AlertRuleState::Orphaned, "{rows:?}");
        assert_ne!(rows[0].state, AlertRuleState::NotChecked, "{rows:?}");
        assert_eq!(rows[1].state, AlertRuleState::Orphaned);

        // The battery's card lists the trip's rule, since the trip includes it.
        let entry = SpotEntry {
            spot: battery(),
            snapshot_json: Some(snapshot("battery", battery())),
            last_success: Some(now()),
            error: None,
            refreshing: false,
        };
        let with = view_model(
            vec![entry.clone()],
            0,
            settings.clone(),
            now(),
            Some(run.state.clone()),
            vec![harbor],
        )
        .unwrap();
        assert!(with.contains("Storm watch"), "{with}");
        let without =
            view_model(vec![entry], 0, settings, now(), Some(run.state), Vec::new()).unwrap();
        assert!(!without.contains("Storm watch"));
    }

    #[test]
    fn a_box_rule_reads_the_grid_inside_its_box() {
        let settings = settings_with(&[box_rule("warm")]);
        let steps = plan_alerts(settings.clone(), Vec::new(), Vec::new(), now()).unwrap();
        let [AlertStep::Grid { grid }] = &steps[..] else {
            panic!("{steps:?}");
        };
        // The recorded grid is of 2026-11-16, 09:00 UTC.
        let at = parse_rfc3339("2026-11-17T06:00:00Z").unwrap().0;
        let body = raw("erddap/jplmursst41-sst-mask-subset-2026-11-17.json");
        let run = evaluate_alerts(
            settings.clone(),
            Vec::new(),
            Vec::new(),
            vec![AlertGridResponse {
                key: grid.key.clone(),
                response: response(200, body),
            }],
            None,
            "America/New_York".into(),
            at,
        )
        .unwrap();
        assert_eq!(run.notifications.len(), 1, "{run:?}");
        let n = &run.notifications[0];
        assert_eq!(n.title, "Warm water off the Rockaways");
        assert!(n.body.contains("°F"), "{}", n.body);
        assert!(
            !n.body.contains("40.") && !n.body.contains("73."),
            "{}",
            n.body
        );

        // Without the grid there is nothing to go on, and nothing fires.
        let none = evaluate_alerts(
            settings,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            "America/New_York".into(),
            at,
        )
        .unwrap();
        assert_eq!(none.notifications.len(), 0);
    }

    #[test]
    fn unreadable_state_is_an_error_and_unreadable_trips_are_passed_over() {
        let settings = settings_with(&[flow_rule("low", CFS_800)]);
        let bad = evaluate_alerts(
            settings.clone(),
            Vec::new(),
            vec!["{".into()],
            Vec::new(),
            Some(b"nonsense".to_vec()),
            "UTC".into(),
            now(),
        );
        assert!(matches!(bad, Err(AlertError::Invalid { .. })));
        let fresh = evaluate_alerts(
            settings,
            Vec::new(),
            vec!["{".into()],
            Vec::new(),
            Some(Vec::new()),
            "UTC".into(),
            now(),
        )
        .unwrap();
        assert_eq!(fresh.notifications.len(), 0);
    }
}
