//! Trips as the app runs them: the places to fetch, the plan of history and tide requests
//! answered from recorded responses, and the trip's view.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use underbite_core::time::parse_rfc3339;
use underbite_ffi::fetch::{
    CachedResponse, FetchHost, FetchOptions, HostError, HostRequest, HostResponse, Keys, fetch_spot,
};
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::trip::{
    TripAppStatus, TripPack, TripPlaceData, TripPlan, TripResponse, TripSharing, plan_trip,
    trip_problem, trip_spots, trip_view,
};
use underbite_ffi::view::SpotInfo;

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

    /// The recorded answer to `url`, or none when no route matches.
    fn answer(&self, url: &str) -> Option<HostResponse> {
        let route = self.routes.iter().find(|r| url.contains(&r.matches))?;
        let path = self.base.join(&route.file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Some(response(route.status.unwrap_or(200), body))
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
        self.answer(&request.url).ok_or_else(|| HostError::Failed {
            message: "no route".into(),
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

fn spot(id: &str, latitude: f64, longitude: f64, water_type: &str) -> SpotInfo {
    SpotInfo {
        id: id.into(),
        name: id.into(),
        latitude,
        longitude,
        water_type: water_type.into(),
        time_zone: "America/New_York".into(),
    }
}

fn battery() -> SpotInfo {
    SpotInfo {
        name: "The Battery".into(),
        ..spot("battery", 40.7033, -74.0170, "tidal_river_estuary")
    }
}

fn croton() -> SpotInfo {
    spot("croton", 41.2257, -73.8618, "river_stream")
}

/// The spot's snapshot from its recorded scenario at the test's time.
fn snapshot(scenario: &str, spot: SpotInfo) -> String {
    let out = pollster::block_on(fetch_spot(
        spot,
        default_settings_json(),
        Keys::default(),
        FetchOptions {
            allow_metered: true,
            infer_water_type: false,
            refresh_metadata: false,
        },
        now(),
        Vec::new(),
        Replay::new(scenario),
    ))
    .expect("fetched");
    out.snapshot_json
}

fn trip(places: &[(&SpotInfo, Option<&str>)], first: &str, last: &str) -> String {
    let places: Vec<Value> = places
        .iter()
        .map(|(s, region)| {
            json!({
                "spot_id": s.id,
                "name": s.name,
                "latitude": s.latitude,
                "longitude": s.longitude,
                "time_zone": s.time_zone,
                "water_type": s.water_type,
                "region": region,
            })
        })
        .collect();
    json!({
        "id": "trip-1",
        "name": "Harbor weekend",
        "places": places,
        "first": first,
        "last": last,
        "method": "shore",
        "modified_at": now(),
    })
    .to_string()
}

fn data(key: &str, snapshot_json: Option<String>, history_json: Option<String>) -> TripPlaceData {
    TripPlaceData {
        key: key.into(),
        snapshot_json,
        history_json,
    }
}

const NOT_SHARED: TripAppStatus = TripAppStatus {
    pack: TripPack::NotDownloaded,
    sharing: TripSharing::NotShared,
};

/// Answers every request of `plan` from `scenario`'s routes; requests without a route get none.
fn answer(plan: &TripPlan, scenario: &str) -> Vec<TripResponse> {
    let replay = Replay::new(scenario);
    plan.requests()
        .into_iter()
        .filter_map(|r| {
            Some(TripResponse {
                response: replay.answer(&r.request.url)?,
                id: r.id,
            })
        })
        .collect()
}

fn view(trip_json: &str, places: Vec<TripPlaceData>, cached: Vec<CachedResponse>) -> Value {
    let json = trip_view(
        trip_json.to_owned(),
        places,
        cached,
        NOT_SHARED,
        default_settings_json(),
        now(),
    )
    .expect("view");
    serde_json::from_str(&json).expect("view json")
}

/// The cell of `kind` on day `day` at the first place.
fn cell<'a>(view: &'a Value, day: usize, kind: &str) -> &'a Value {
    view["days"][day]["places"][0]["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .find(|c| c["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} cell"))
}

// ---------------------------------------------------------------------------------------------
// Checking a trip and the places to fetch
// ---------------------------------------------------------------------------------------------

#[test]
fn a_trip_problem_is_a_sentence_and_a_good_trip_has_none() {
    let good = trip(&[(&battery(), None)], "2026-11-08", "2026-11-10");
    assert_eq!(trip_problem(good), None);
    let backwards = trip(&[(&battery(), None)], "2026-11-10", "2026-11-08");
    assert_eq!(
        trip_problem(backwards).as_deref(),
        Some("The last day is before the first.")
    );
    let long = trip(&[(&battery(), None)], "2026-11-01", "2026-12-15");
    assert_eq!(
        trip_problem(long).as_deref(),
        Some("The trip is longer than 31 days.")
    );
    let mut nowhere = battery();
    nowhere.time_zone = "Mars/Olympus".into();
    let lost = trip(&[(&nowhere, None)], "2026-11-08", "2026-11-08");
    assert_eq!(
        trip_problem(lost).as_deref(),
        Some("The Battery: unknown time zone.")
    );
    assert_eq!(
        trip_problem("{}".into()).as_deref(),
        Some("That is not a trip this app can read.")
    );
}

#[test]
fn places_within_the_forecasts_reach_ask_for_one_and_area_only_places_have_no_spot() {
    let soon = trip(&[(&battery(), None)], "2026-11-08", "2026-11-10");
    let spots = trip_spots(soon, now()).expect("spots");
    assert_eq!(spots.len(), 1);
    assert_eq!(spots[0].key, "battery");
    assert!(spots[0].forecast);
    assert_eq!(spots[0].spot.as_ref().map(|s| s.latitude), Some(40.7033));

    let later = trip(&[(&battery(), None)], "2027-01-08", "2027-01-10");
    assert!(!trip_spots(later, now()).expect("spots")[0].forecast);

    let over = trip(&[(&battery(), None)], "2026-10-01", "2026-10-03");
    assert!(!trip_spots(over, now()).expect("spots")[0].forecast);

    let mut v: Value =
        serde_json::from_str(&trip(&[(&battery(), None)], "2026-11-08", "2026-11-08")).unwrap();
    v["places"][0]["latitude"] = Value::Null;
    v["places"][0]["longitude"] = Value::Null;
    v["places"][0]["fuzz"] = "area_only".into();
    let spots = trip_spots(v.to_string(), now()).expect("spots");
    assert_eq!(spots[0].spot, None);
    assert!(trip_spots("not json".into(), now()).is_err());
}

// ---------------------------------------------------------------------------------------------
// The Battery: tides from the station's constants, forecasts from the snapshot
// ---------------------------------------------------------------------------------------------

#[test]
fn a_battery_weekend_has_certain_tides_every_day_and_forecast_cells() {
    let snapshot_json = snapshot("battery", battery());
    let trip_json = trip(&[(&battery(), Some("US-NY"))], "2026-11-08", "2026-11-10");
    let places = vec![data("battery", Some(snapshot_json.clone()), None)];
    let plan = plan_trip(
        trip_json.clone(),
        places.clone(),
        Vec::new(),
        default_settings_json(),
        Keys::default(),
        true,
        now(),
    )
    .expect("plan");
    let requests = plan.requests();
    let urls: Vec<&str> = requests.iter().map(|r| r.request.url.as_str()).collect();
    assert!(
        urls.iter()
            .any(|u| u.ends_with("/stations/8518750/datums.json?units=metric")),
        "{urls:?}"
    );
    assert!(
        urls.iter()
            .any(|u| u.ends_with("/stations/8518750/harcon.json?units=metric")),
        "{urls:?}"
    );
    assert!(requests.iter().all(|r| r.place == "battery"));
    // The constants are not metered; every id is distinct.
    assert!(
        requests
            .iter()
            .filter(|r| r.source == "coops")
            .all(|r| !r.metered)
    );
    let mut ids: Vec<&str> = requests.iter().map(|r| r.id.as_str()).collect();
    ids.dedup();
    assert_eq!(ids.len(), requests.len());

    let results = plan.results(answer(&plan, "battery"));
    let keys: Vec<&str> = results.cacheable.iter().map(|c| c.key.as_str()).collect();
    assert!(
        keys.iter()
            .any(|k| k.ends_with("8518750/harcon.json?units=metric")),
        "{keys:?}"
    );
    assert!(
        keys.iter()
            .any(|k| k.ends_with("8518750/datums.json?units=metric")),
        "{keys:?}"
    );

    // With the constants kept, the plan asks for them no more.
    let again = plan_trip(
        trip_json.clone(),
        places,
        results.cacheable.clone(),
        default_settings_json(),
        Keys::default(),
        true,
        now(),
    )
    .expect("plan");
    assert!(again.requests().iter().all(|r| r.source != "coops"));

    let histories: BTreeMap<String, String> = results
        .histories
        .iter()
        .map(|h| (h.key.clone(), h.history_json.clone()))
        .collect();
    let v = view(
        &trip_json,
        vec![data(
            "battery",
            Some(snapshot_json),
            histories.get("battery").cloned(),
        )],
        results.cacheable,
    );
    let days = v["days"].as_array().expect("days");
    assert_eq!(days.len(), 3);
    for day in 0..3 {
        let tides = cell(&v, day, "tides");
        assert_eq!(tides["basis"], "certain", "day {day}: {tides}");
        assert!(!tides["lines"].as_array().unwrap().is_empty(), "{tides}");
        assert_eq!(cell(&v, day, "sun")["basis"], "certain");
        assert_eq!(cell(&v, day, "moon")["basis"], "certain");
    }
    // The first day's windows rest on the snapshot's forecasts (currents, pressure); the
    // recorded weather (from Nov 2) reaches no day, so none has a verdict.
    let windows = cell(&v, 0, "windows");
    assert_eq!(windows["basis"], "forecast", "{windows}");
    assert!(
        windows["basis_label"]
            .as_str()
            .unwrap()
            .starts_with("forecast, issued"),
        "{windows}"
    );
    assert_eq!(cell(&v, 0, "verdict")["basis"], "unavailable");
    assert_eq!(v["title"], "Harbor weekend");
    let checklist = v["checklist"].as_array().expect("checklist");
    assert!(
        checklist.iter().any(|r| r["kind"] == "licence"),
        "{checklist:?}"
    );
}

#[test]
fn the_tides_come_from_the_snapshot_without_kept_constants() {
    let snapshot_json = snapshot("battery", battery());
    let trip_json = trip(&[(&battery(), None)], "2026-11-08", "2026-11-08");
    let v = view(
        &trip_json,
        vec![data("battery", Some(snapshot_json), None)],
        Vec::new(),
    );
    let tides = cell(&v, 0, "tides");
    assert_eq!(tides["basis"], "certain", "{tides}");
}

#[test]
fn a_subordinate_station_asks_for_its_offsets_and_its_references_constants() {
    let mut snapshot: Value = serde_json::from_str(&snapshot("battery", battery())).unwrap();
    let station = &mut snapshot["marine"]["tide"]["provenance"]["station"];
    station["id"] = "8518919".into();
    station["kind"] = "tide_subordinate".into();
    station["reference_id"] = "8518750".into();
    let snapshot_json = snapshot.to_string();
    let trip_json = trip(&[(&battery(), None)], "2027-01-08", "2027-01-09");
    let plan = plan_trip(
        trip_json.clone(),
        vec![data("battery", Some(snapshot_json.clone()), None)],
        Vec::new(),
        default_settings_json(),
        Keys::default(),
        false,
        now(),
    )
    .expect("plan");
    let urls: Vec<String> = plan.requests().into_iter().map(|r| r.request.url).collect();
    assert_eq!(urls.len(), 3, "{urls:?}");
    assert!(
        urls[0].ends_with("/stations/8518919/tidepredoffsets.json"),
        "{urls:?}"
    );
    assert!(
        urls[1].ends_with("/stations/8518750/datums.json?units=metric"),
        "{urls:?}"
    );
    assert!(
        urls[2].ends_with("/stations/8518750/harcon.json?units=metric"),
        "{urls:?}"
    );

    let bodies = [
        raw("coops/coops-mdapi-8518919-tidepredoffsets.json"),
        raw("coops/coops-mdapi-8518750-datums-metric.json"),
        raw("coops/coops-mdapi-8518750-harcon-metric.json"),
    ];
    let responses = plan
        .requests()
        .into_iter()
        .zip(bodies)
        .map(|(r, body)| TripResponse {
            id: r.id,
            response: response(200, body),
        })
        .collect();
    let results = plan.results(responses);
    assert_eq!(results.cacheable.len(), 3);
    assert_eq!(results.errors, Vec::new());
    // Two months out: the tides are computed from the reference and the offsets.
    let v = view(
        &trip_json,
        vec![data("battery", Some(snapshot_json), None)],
        results.cacheable,
    );
    for day in 0..2 {
        let tides = cell(&v, day, "tides");
        assert_eq!(tides["basis"], "certain", "{tides}");
    }
}

#[test]
fn an_answer_that_is_not_the_constants_is_not_kept() {
    let snapshot_json = snapshot("battery", battery());
    let trip_json = trip(&[(&battery(), None)], "2026-11-08", "2026-11-09");
    let plan = plan_trip(
        trip_json,
        vec![data("battery", Some(snapshot_json), None)],
        Vec::new(),
        default_settings_json(),
        Keys::default(),
        false,
        now(),
    )
    .expect("plan");
    let responses: Vec<TripResponse> = plan
        .requests()
        .into_iter()
        .map(|r| TripResponse {
            id: r.id,
            response: response(200, b"<html>Sign in</html>".to_vec()),
        })
        .collect();
    let missing = plan.requests().len();
    let results = plan.results(responses);
    assert_eq!(results.cacheable, Vec::new());
    assert_eq!(results.errors.len(), missing);
    // And no response at all is a failure too.
    assert_eq!(plan.results(Vec::new()).errors.len(), missing);
}

// ---------------------------------------------------------------------------------------------
// History: what the dates looked like in past years
// ---------------------------------------------------------------------------------------------

#[test]
fn a_croton_trip_beyond_the_forecast_shows_typical_flow_from_the_normals() {
    let snapshot_json = snapshot("croton", croton());
    // A year out: no forecast reaches it, and the recorded normals cover 11-05 to 11-10.
    let trip_json = trip(&[(&croton(), Some("US-NY"))], "2027-11-08", "2027-11-10");
    let places = vec![data("croton", Some(snapshot_json.clone()), None)];
    let plan = plan_trip(
        trip_json.clone(),
        places.clone(),
        Vec::new(),
        default_settings_json(),
        Keys::default(),
        true,
        now(),
    )
    .expect("plan");
    let requests = plan.requests();
    let normals: Vec<&str> = requests
        .iter()
        .map(|r| r.request.url.as_str())
        .filter(|u| u.contains("observationNormals"))
        .collect();
    assert!(
        normals.iter().any(|u| u.contains("parameter_code=00060")
            && u.contains("start_date=11-08")
            && u.contains("end_date=11-10")),
        "{normals:?}"
    );
    assert!(
        requests
            .iter()
            .filter(|r| r.source == "usgs")
            .all(|r| r.metered)
    );

    let results = plan.results(answer(&plan, "croton"));
    let history = results
        .histories
        .iter()
        .find(|h| h.key == "croton")
        .expect("history");
    let v = view(
        &trip_json,
        vec![data(
            "croton",
            Some(snapshot_json),
            Some(history.history_json.clone()),
        )],
        results.cacheable,
    );
    for day in 0..3 {
        let flow = cell(&v, day, "flow");
        assert_eq!(flow["basis"], "typical", "{flow}");
        assert!(
            flow["basis_label"].as_str().unwrap().contains("typical"),
            "{flow}"
        );
        assert_eq!(cell(&v, day, "verdict")["basis"], "unavailable");
    }

    // The history is kept: the next plan does not ask for the flow normals again.
    let again = plan_trip(
        trip_json,
        vec![data(
            "croton",
            places[0].snapshot_json.clone(),
            Some(history.history_json.clone()),
        )],
        Vec::new(),
        default_settings_json(),
        Keys::default(),
        true,
        now(),
    )
    .expect("plan");
    assert!(
        again
            .requests()
            .iter()
            .all(|r| !r.request.url.contains("parameter_code=00060")),
        "{:?}",
        again.requests()
    );
}

#[test]
fn metered_history_is_planned_only_when_allowed() {
    let snapshot_json = snapshot("croton", croton());
    let trip_json = trip(&[(&croton(), None)], "2027-11-08", "2027-11-10");
    let plan = plan_trip(
        trip_json,
        vec![data("croton", Some(snapshot_json), None)],
        Vec::new(),
        default_settings_json(),
        Keys::default(),
        false,
        now(),
    )
    .expect("plan");
    assert!(
        plan.requests().iter().all(|r| !r.metered),
        "{:?}",
        plan.requests()
    );
}

#[test]
fn sea_surface_temperature_history_asks_each_past_year_at_a_rounded_point() {
    let snapshot_json = snapshot("battery", battery());
    let trip_json = trip(&[(&battery(), None)], "2027-11-08", "2027-11-10");
    let plan = plan_trip(
        trip_json,
        vec![data("battery", Some(snapshot_json), None)],
        Vec::new(),
        default_settings_json(),
        Keys::default(),
        true,
        now(),
    )
    .expect("plan");
    let sst: Vec<String> = plan
        .requests()
        .into_iter()
        .filter(|r| r.source == "open-meteo-marine")
        .map(|r| r.request.url)
        .collect();
    // 2026's dates are not over yet at the test's time.
    assert_eq!(sst.len(), 3, "{sst:?}");
    for (url, year) in sst.iter().zip(2023..) {
        assert!(url.contains(&format!("start_date={year}-11-08")), "{url}");
        assert!(url.contains(&format!("end_date={year}-11-10")), "{url}");
        assert!(url.contains("sea_surface_temperature"), "{url}");
        for (name, value) in url
            .split_once('?')
            .unwrap()
            .1
            .split('&')
            .filter_map(|p| p.split_once('='))
        {
            if name == "latitude" || name == "longitude" {
                let decimals = value.split_once('.').map_or(0, |(_, d)| d.len());
                assert!(decimals <= 2, "{name}={value} in {url}");
            }
        }
    }
}

#[test]
fn a_view_without_snapshots_still_has_the_sky_and_says_what_is_unavailable() {
    let trip_json = trip(&[(&battery(), None)], "2027-11-08", "2027-11-09");
    let v = view(&trip_json, vec![], Vec::new());
    assert_eq!(v["days"].as_array().unwrap().len(), 2);
    assert_eq!(cell(&v, 0, "sun")["basis"], "certain");
    assert_eq!(cell(&v, 0, "verdict")["basis"], "unavailable");
    assert_eq!(cell(&v, 0, "flow")["basis"], "unavailable");
    let err = trip_view(
        trip_json,
        vec![data("battery", Some("{".into()), None)],
        Vec::new(),
        NOT_SHARED,
        default_settings_json(),
        now(),
    );
    assert!(err.is_err());
}
