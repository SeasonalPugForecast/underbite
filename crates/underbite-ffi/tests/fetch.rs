//! `fetch_spot` and `discover_stations` as the app calls them, with a host answering from the
//! recorded replay scenarios the command line uses (`tests/fixtures/replay/<scenario>`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use underbite_core::model::{SpotSnapshot, WaterType};
use underbite_core::time::parse_rfc3339;
use underbite_ffi::fetch::{
    FetchError, FetchHost, FetchOptions, HostError, HostRequest, HostResponse, Keys, clear_station,
    discover_stations, exclude_station, fetch_spot, forget_spot, pin_station, stored_stations,
};
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::view::{SpotInfo, station_rows};
use underbite_providers::request::redact_url;

/// The time the replay scenarios were recorded for.
fn now() -> i64 {
    parse_rfc3339("2026-11-08T14:30:00Z").expect("time").0
}

fn battery() -> SpotInfo {
    SpotInfo {
        id: "battery".into(),
        name: "The Battery".into(),
        latitude: 40.7033,
        longitude: -74.0170,
        water_type: "tidal_river_estuary".into(),
        time_zone: "America/New_York".into(),
    }
}

fn croton() -> SpotInfo {
    SpotInfo {
        id: "croton".into(),
        name: "Croton River below the dam".into(),
        latitude: 41.2252,
        longitude: -73.8588,
        water_type: "river_stream".into(),
        time_zone: "America/New_York".into(),
    }
}

/// The default settings with `edit` applied to their JSON.
fn settings(edit: impl FnOnce(&mut Value)) -> String {
    let mut json: Value = serde_json::from_str(&default_settings_json()).expect("json");
    edit(&mut json);
    json.to_string()
}

const METERED: FetchOptions = FetchOptions {
    allow_metered: true,
    infer_water_type: false,
};

// ---------------------------------------------------------------------------------------------
// A host that replays the recorded scenarios
// ---------------------------------------------------------------------------------------------

/// One route of a scenario's `routes.toml`: the first route whose `match` the URL contains
/// answers with the recorded `file`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Route {
    #[serde(rename = "match")]
    matches: String,
    file: PathBuf,
    status: Option<u16>,
    content_type: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Routes {
    route: Vec<Route>,
}

/// Answers from a replay scenario and panics with the redacted URL on a request no route
/// answers, so a test never passes without a source. Keeps every request, and the values and
/// ledger in memory. URLs containing `failing` get a host error instead.
struct Replay {
    base: PathBuf,
    routes: Vec<Route>,
    failing: Option<&'static str>,
    seen: Mutex<Vec<HostRequest>>,
    values: Mutex<BTreeMap<(String, String), String>>,
}

impl Replay {
    fn new(scenario: &str) -> Arc<Self> {
        Self::failing(scenario, None)
    }

    fn failing(scenario: &str, failing: Option<&'static str>) -> Arc<Self> {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/replay")
            .join(scenario);
        let text = std::fs::read_to_string(base.join("routes.toml")).expect("routes");
        let routes: Routes = toml::from_str(&text).expect("routes file");
        Arc::new(Self {
            base,
            routes: routes.route,
            failing,
            seen: Mutex::new(Vec::new()),
            values: Mutex::new(BTreeMap::new()),
        })
    }

    fn seen(&self) -> Vec<HostRequest> {
        self.seen.lock().unwrap().clone()
    }

    fn urls(&self) -> Vec<String> {
        self.seen().into_iter().map(|r| r.url).collect()
    }

    fn value(&self, namespace: &str, key: &str) -> Option<String> {
        let values = self.values.lock().unwrap();
        values.get(&(namespace.into(), key.into())).cloned()
    }

    fn values(&self) -> Vec<String> {
        self.values.lock().unwrap().values().cloned().collect()
    }

    fn host(self: &Arc<Self>) -> Arc<dyn FetchHost> {
        Arc::clone(self) as Arc<dyn FetchHost>
    }
}

/// The content type of a recorded file without one in its route.
fn content_type(file: &Path) -> &'static str {
    match file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
    {
        "json" => "application/json",
        "xml" => "text/xml",
        "html" => "text/html",
        _ => "text/plain",
    }
}

#[async_trait::async_trait]
impl FetchHost for Replay {
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
        let url = request.url.clone();
        self.seen.lock().unwrap().push(request);
        if self.failing.is_some_and(|part| url.contains(part)) {
            return Err(HostError::Failed {
                message: "The Internet connection appears to be offline.".into(),
            });
        }
        let Some(route) = self.routes.iter().find(|r| url.contains(&r.matches)) else {
            panic!("no replay route for {}", redact_url(&url));
        };
        let path = self.base.join(&route.file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Ok(HostResponse {
            status: route.status.unwrap_or(200),
            body,
            content_type: Some(
                route
                    .content_type
                    .clone()
                    .unwrap_or_else(|| content_type(&route.file).into()),
            ),
            date: None,
            expires: None,
            last_modified: None,
            etag: None,
            retry_after: None,
            rate_limit_remaining: None,
        })
    }

    fn load(&self, namespace: String, key: String) -> Option<String> {
        self.value(&namespace, &key)
    }

    fn store(&self, namespace: String, key: String, value: String) {
        self.values.lock().unwrap().insert((namespace, key), value);
    }

    fn save_usage(&self, ledger_json: String) -> Result<(), HostError> {
        self.store("usage".into(), "ledger".into(), ledger_json);
        Ok(())
    }
}

fn fetched(
    spot: SpotInfo,
    settings: String,
    keys: Keys,
    options: FetchOptions,
    host: &Arc<Replay>,
) -> (SpotSnapshot, underbite_ffi::fetch::SpotFetchOutcome) {
    let out = pollster::block_on(fetch_spot(
        spot,
        settings,
        keys,
        options,
        now(),
        Vec::new(),
        host.host(),
    ))
    .expect("fetched");
    let snapshot = serde_json::from_str(&out.snapshot_json).expect("snapshot");
    (snapshot, out)
}

fn station_of<T>(series: &underbite_core::series::Series<T>) -> String {
    let p = series.provenance();
    let id = p.station.as_ref().map_or("", |s| s.id.as_str());
    format!("{} {id}", p.source)
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn the_battery_through_the_app_host() {
    let host = Replay::new("battery");
    let (s, out) = fetched(
        battery(),
        default_settings_json(),
        Keys::default(),
        METERED,
        &host,
    );

    let tide = s.marine.tide.as_ref().expect("tide");
    assert!(tide.curve.is_some() && !tide.events.is_empty());
    assert_eq!(
        tide.provenance.station.as_ref().map(|st| st.id.as_str()),
        Some("8518750")
    );
    assert!(s.water.temperature_c.is_some(), "water temperature");
    assert!(s.marine.waves.is_some() && s.marine.waves_forecast.is_some());
    let forecast = s.marine.marine_forecast.as_ref().expect("marine forecast");
    assert_eq!(forecast.id, "ANZ338");
    assert!(forecast.periods.len() > 2, "{:?}", forecast.periods);
    assert!(s.weather.is_some(), "weather");
    assert_eq!(s.sky.days.len(), 7);
    assert_eq!(s.fetched_at.0, now());
    assert_eq!(out.suggestion, None);

    // The outcome lists the snapshot's errors (the CO-OPS stations' stale or missing values).
    let errors: Vec<(String, Option<String>, String)> = out
        .errors
        .iter()
        .map(|e| (e.source.clone(), e.measurement.clone(), e.kind.clone()))
        .collect();
    let in_snapshot: Vec<(String, Option<String>, String)> = s
        .errors
        .iter()
        .map(|e| {
            let name = |v: Value| v.as_str().map(str::to_owned);
            (
                e.source.to_string(),
                e.measurement
                    .and_then(|m| name(serde_json::to_value(m).unwrap())),
                name(serde_json::to_value(e.kind).unwrap()).unwrap(),
            )
        })
        .collect();
    assert_eq!(errors, in_snapshot);
    assert!(
        errors.contains(&(
            "coops".into(),
            Some("water_temperature".into()),
            "stale".into()
        )),
        "{errors:?}"
    );

    // The snapshot, the station lists and the ledger are kept through the host.
    let spot_values: Vec<String> = host
        .values
        .lock()
        .unwrap()
        .keys()
        .map(|(namespace, _)| namespace.clone())
        .collect();
    for namespace in ["spot", "stations", "discovery", "usage"] {
        assert!(
            spot_values.iter().any(|n| n == namespace),
            "{namespace}: {spot_values:?}"
        );
    }
    let ledger: Value =
        serde_json::from_str(&host.value("usage", "ledger").expect("ledger")).expect("json");
    assert_eq!(ledger["saved_at"], now());
}

#[test]
fn a_host_error_shows_in_errors_without_failing_the_fetch() {
    let host = Replay::failing("battery", Some("marine-api.open-meteo.com"));
    let (s, out) = fetched(
        battery(),
        default_settings_json(),
        Keys::default(),
        METERED,
        &host,
    );
    let failed: Vec<_> = out
        .errors
        .iter()
        .filter(|e| e.source == "open-meteo-marine")
        .collect();
    assert!(!failed.is_empty(), "{:#?}", out.errors);
    assert!(failed.iter().all(|e| e.kind == "transport"), "{failed:#?}");
    assert!(
        failed
            .iter()
            .all(|e| e.message.contains("offline") && !e.message.contains("latitude=")),
        "{failed:#?}"
    );
    // Everything else still arrived.
    assert!(s.marine.tide.is_some() && s.marine.marine_forecast.is_some());
    assert!(s.water.temperature_c.is_some() && s.marine.waves.is_some());
    assert_eq!(s.marine.waves_forecast, None);
}

#[test]
fn the_usgs_key_goes_only_in_its_header() {
    const KEY: &str = "TEST-USGS-KEY-123";
    let keys = Keys {
        usgs: Some(KEY.into()),
        ..Keys::default()
    };
    let host = Replay::new("croton");
    let settings = settings(|s| s["sources"]["flow"]["max_distance_km"] = 17.0.into());
    let (s, out) = fetched(croton(), settings, keys, METERED, &host);
    let discharge = s.water.discharge_m3s.as_ref().expect("discharge");
    assert_eq!(station_of(discharge), "usgs USGS-01375000");

    let requests = host.seen();
    let usgs: Vec<&HostRequest> = requests
        .iter()
        .filter(|r| r.url.contains("api.waterdata.usgs.gov"))
        .collect();
    assert!(!usgs.is_empty(), "no USGS request");
    for r in &usgs {
        let keyed: Vec<_> = r.headers.iter().filter(|h| h.value == KEY).collect();
        assert_eq!(keyed.len(), 1, "{r:?}");
        assert_eq!(keyed[0].name, "X-Api-Key");
        assert!(
            r.no_store,
            "a keyed response is never written to disk: {r:?}"
        );
    }
    for r in &requests {
        assert!(!r.url.contains(KEY), "{}", redact_url(&r.url));
        assert!(!r.body.as_deref().unwrap_or_default().contains(KEY));
        if !r.url.contains("api.waterdata.usgs.gov") {
            assert!(r.headers.iter().all(|h| h.value != KEY), "{r:?}");
        }
        assert!(!format!("{r:?}").contains(KEY));
    }
    // Nothing the fetch keeps or returns holds the key.
    assert!(!out.snapshot_json.contains(KEY));
    assert!(host.values().iter().all(|v| !v.contains(KEY)));
    assert!(!format!("{out:?}").contains(KEY));
}

#[test]
fn the_battery_saved_as_a_river_is_inferred_tidal() {
    let host = Replay::new("battery");
    let spot = SpotInfo {
        water_type: "river_stream".into(),
        ..battery()
    };
    let options = FetchOptions {
        infer_water_type: true,
        ..METERED
    };
    let (s, out) = fetched(
        spot,
        default_settings_json(),
        Keys::default(),
        options,
        &host,
    );
    let suggestion = out.suggestion.expect("suggestion");
    assert_eq!(suggestion.water_type, "tidal_river_estuary");
    assert_eq!(
        suggestion.reasons.first().map(String::as_str),
        Some("in an NWS coastal marine zone"),
        "{:?}",
        suggestion.reasons
    );
    assert_eq!(s.spot.water_type, WaterType::TidalRiverEstuary);
    assert!(s.marine.tide.is_some());
}

#[test]
fn battery_discovery_ranks_the_battery_and_keeps_the_lists() {
    let host = Replay::new("battery");
    let discover = || {
        pollster::block_on(discover_stations(
            battery(),
            default_settings_json(),
            Keys::default(),
            true,
            now(),
            host.host(),
        ))
        .expect("discovered")
    };
    let found = discover();
    assert_eq!(found.errors, []);
    let stations: Value = serde_json::from_str(&found.stations_json).expect("json");
    assert_eq!(stations["tide"]["stations"][0]["station"]["id"], "8518750");
    assert!(
        stations["tidal_current"]["stations"]
            .as_array()
            .is_some_and(|s| !s.is_empty())
    );

    // The lists come from the host's store the second time.
    let sent = host.urls().len();
    let again = discover();
    assert_eq!(again.stations_json, found.stations_json);
    let lists = [
        "stations.json",
        "activestations.xml",
        "monitoring-locations",
    ];
    let later = &host.urls()[sent..];
    assert!(
        later.iter().all(|u| lists.iter().all(|l| !u.contains(l))),
        "{later:#?}"
    );
}

#[test]
fn unreadable_settings_or_spots_fail_the_call() {
    let host = Replay::new("battery");
    let run = |spot: SpotInfo, settings: &str| {
        pollster::block_on(fetch_spot(
            spot,
            settings.into(),
            Keys::default(),
            METERED,
            now(),
            Vec::new(),
            host.host(),
        ))
    };
    assert!(matches!(
        run(battery(), "{not json"),
        Err(FetchError::Settings { .. })
    ));
    let puddle = SpotInfo {
        water_type: "puddle".into(),
        ..battery()
    };
    assert!(matches!(
        run(puddle, &default_settings_json()),
        Err(FetchError::Invalid { .. })
    ));
    assert_eq!(host.urls(), Vec::<String>::new());
}

#[test]
fn the_reasons_for_a_water_type_come_as_text_in_either_voice() {
    for voice in ["plain", "luna"] {
        let host = Replay::new("battery");
        let spot = SpotInfo {
            water_type: "river_stream".into(),
            ..battery()
        };
        let options = FetchOptions {
            infer_water_type: true,
            ..METERED
        };
        let settings = settings(|s| s["presentation"]["voice"] = voice.into());
        let (_, out) = fetched(spot, settings, Keys::default(), options, &host);
        let reasons = out.suggestion.expect("suggestion").reasons;
        assert!(reasons.len() >= 2, "{voice}: {reasons:?}");
        assert!(
            reasons.iter().all(|r| !r.contains('{') && !r.contains('_')),
            "{voice}: {reasons:?}"
        );
        assert_eq!(reasons[0], "in an NWS coastal marine zone", "{voice}");
    }
}

/// The station the tide came from after a fetch of the Battery.
fn tide_station(host: &Arc<Replay>) -> String {
    let (s, _) = fetched(
        battery(),
        default_settings_json(),
        Keys::default(),
        METERED,
        host,
    );
    let tide = s.marine.tide.as_ref().expect("tide");
    tide.provenance
        .station
        .as_ref()
        .map(|st| st.id.clone())
        .expect("station")
}

#[test]
fn a_pin_or_exclusion_changes_the_next_fetches_station() {
    let host = Replay::new("battery");
    assert_eq!(tide_station(&host), "8518750");

    // Another tide station the ranking offers.
    let found = pollster::block_on(discover_stations(
        battery(),
        default_settings_json(),
        Keys::default(),
        true,
        now(),
        host.host(),
    ))
    .expect("discovered");
    let stations: Value = serde_json::from_str(&found.stations_json).expect("json");
    let other = stations["tide"]["stations"]
        .as_array()
        .expect("stations")
        .iter()
        .map(|s| s["station"]["id"].as_str().expect("id").to_owned())
        .find(|id| id != "8518750")
        .expect("a second tide station");
    let key = format!("coops:{}", other.to_ascii_lowercase());

    pin_station("battery".into(), key.clone(), now(), host.host()).expect("pinned");
    assert_eq!(tide_station(&host), other);

    // Another spot is not affected, and a pin is lifted by clearing it.
    clear_station("battery".into(), key, now(), host.host()).expect("cleared");
    assert_eq!(tide_station(&host), "8518750");

    // Excluding the nearest station moves the tide elsewhere; the host holds only keys and ids.
    exclude_station("battery".into(), "coops:8518750".into(), now(), host.host())
        .expect("excluded");
    assert_ne!(tide_station(&host), "8518750");
    let choices = host
        .value("choices", &underbite_fetch::store::spot_key("battery"))
        .expect("choices");
    assert!(
        choices.contains("8518750") && !choices.contains("40.70"),
        "{choices}"
    );
    assert!(
        host.value("choices", &underbite_fetch::store::spot_key("croton"))
            .is_none()
    );
}

#[test]
fn a_station_key_that_is_not_one_fails_the_call() {
    let host = Replay::new("battery");
    for bad in ["coops", "coops:", ":8518750", "coops:8518750:x"] {
        assert!(
            matches!(
                pin_station("battery".into(), bad.into(), now(), host.host()),
                Err(FetchError::Invalid { .. })
            ),
            "{bad}"
        );
    }
    assert_eq!(host.values(), Vec::<String>::new());
}

// ---------------------------------------------------------------------------------------------
// The station rows
// ---------------------------------------------------------------------------------------------

fn discovered(spot: SpotInfo, host: &Arc<Replay>) -> String {
    pollster::block_on(discover_stations(
        spot,
        default_settings_json(),
        Keys::default(),
        true,
        now(),
        host.host(),
    ))
    .expect("discovered")
    .stations_json
}

#[test]
fn the_battery_rows_have_distance_relation_and_reasons_in_words() {
    let host = Replay::new("battery");
    let groups =
        station_rows(discovered(battery(), &host), default_settings_json(), now()).expect("rows");
    assert_eq!(groups[0].measurement, "tide");
    assert_eq!(groups[0].title, "Tide");
    let first = &groups[0].rows[0];
    assert_eq!(first.key, "coops:8518750");
    assert_eq!(first.source_name, "NOAA CO-OPS");
    assert!(first.distance.ends_with(" mi"), "{}", first.distance);
    assert!(first.relation.is_some());
    assert!(
        first.detail.starts_with("NOAA CO-OPS 8518750 · "),
        "{}",
        first.detail
    );
    assert_ne!(first.reasons, Vec::<String>::new());
    assert!(first.used && !first.pinned && !first.excluded);

    // Metric settings give kilometres.
    let metric = settings(|v| v["units"]["distance"] = Value::from("km"));
    let km = station_rows(discovered(battery(), &host), metric, now()).expect("rows");
    assert!(km[0].rows[0].distance.ends_with(" km"));

    // Every key the rows give is one the pin calls take.
    for row in groups.iter().flat_map(|g| g.rows.iter().chain(&g.left_out)) {
        pin_station("other".into(), row.key.clone(), now(), host.host()).expect(&row.key);
    }
}

#[test]
fn an_excluded_station_stays_listed_with_its_reason() {
    let host = Replay::new("battery");
    exclude_station("battery".into(), "coops:8518750".into(), now(), host.host())
        .expect("excluded");
    let groups =
        station_rows(discovered(battery(), &host), default_settings_json(), now()).expect("rows");
    let tide = groups
        .iter()
        .find(|g| g.measurement == "tide")
        .expect("tide");
    let row = tide
        .rows
        .iter()
        .find(|r| r.key == "coops:8518750")
        .expect("listed");
    assert!(row.excluded && !row.used);
    assert_eq!(row.reasons, ["Excluded by you"]);
    assert_ne!(tide.rows[0].key, "coops:8518750");
}

#[test]
fn croton_rows_read_usgs_sites_and_say_why_others_were_left_out() {
    let host = Replay::new("croton");
    let groups =
        station_rows(discovered(croton(), &host), default_settings_json(), now()).expect("rows");
    let flow = groups
        .iter()
        .find(|g| g.measurement == "discharge")
        .expect("river flow");
    assert_eq!(flow.title, "River flow");
    let site = &flow.rows[0];
    assert!(site.key.starts_with("usgs:USGS-"), "{}", site.key);
    assert_eq!(site.source_name, "USGS");
    assert!(site.relation.is_some());
    for left in groups.iter().flat_map(|g| &g.left_out) {
        assert_eq!(left.reasons.len(), 1, "{left:?}");
        assert!(!left.used);
    }
}

#[test]
fn a_fetch_leaves_its_stations_to_read_without_a_request() {
    let host = Replay::new("battery");
    assert_eq!(stored_stations("battery".into(), host.host()), None);
    fetched(
        battery(),
        default_settings_json(),
        Keys::default(),
        METERED,
        &host,
    );
    let sent = host.urls().len();
    let stored = stored_stations("battery".into(), host.host()).expect("stored");
    assert_eq!(host.urls().len(), sent, "read without a request");
    // The same ranking a lookup gives, and rows read from it.
    assert_eq!(stored, discovered(battery(), &host));
    let groups = station_rows(stored, default_settings_json(), now()).expect("rows");
    assert_eq!(groups[0].rows[0].key, "coops:8518750");
    // Another spot has none, and an unreadable value is none.
    assert_eq!(stored_stations("croton".into(), host.host()), None);
    host.store(
        "discovery".into(),
        underbite_fetch::store::spot_key("battery"),
        "not json".into(),
    );
    assert_eq!(stored_stations("battery".into(), host.host()), None);
}

#[test]
fn unreadable_stations_fail_the_rows() {
    assert!(station_rows("[]".into(), default_settings_json(), now()).is_err());
    assert_eq!(
        station_rows("{}".into(), default_settings_json(), now()).expect("rows"),
        []
    );
}

#[test]
fn a_forgotten_spot_leaves_no_snapshot_rankings_or_choices_on_the_host() {
    let host = Replay::new("battery");
    fetched(
        battery(),
        default_settings_json(),
        Keys::default(),
        METERED,
        &host,
    );
    exclude_station("battery".into(), "coops:8518750".into(), now(), host.host())
        .expect("excluded");
    // The spot's exact longitude (a station nearby shares its latitude).
    let exact = |values: &[String]| {
        values
            .iter()
            .any(|v| v.contains("-74.017,") || v.contains("-74.017}"))
    };
    assert!(
        exact(&host.values()),
        "the snapshot holds the spot's position"
    );
    assert!(stored_stations("battery".into(), host.host()).is_some());
    let lists = host
        .values
        .lock()
        .unwrap()
        .keys()
        .filter(|(namespace, _)| namespace == "stations")
        .count();

    forget_spot("battery".into(), host.host());
    assert!(!exact(&host.values()), "no value holds the position");
    let key = underbite_fetch::store::spot_key("battery");
    for namespace in ["spot", "discovery", "choices"] {
        assert_eq!(
            host.value(namespace, &key).as_deref(),
            Some(""),
            "{namespace}"
        );
    }
    assert_eq!(stored_stations("battery".into(), host.host()), None);
    // The area's station lists and the ledger stay.
    let kept = host
        .values
        .lock()
        .unwrap()
        .keys()
        .filter(|(namespace, _)| namespace == "stations")
        .count();
    assert_eq!(kept, lists);
    assert!(host.value("usage", "ledger").is_some_and(|l| !l.is_empty()));

    // The exclusion went with it: a fetch of the same id takes the nearest station again.
    assert_eq!(tide_station(&host), "8518750");
}
