//! `map_stations` as the app calls it, with a host answering from recorded responses
//! (`tests/fixtures/raw`): the Battery, the Croton River and the gauges of the lower Hudson.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use underbite_core::time::parse_rfc3339;
use underbite_ffi::fetch::{FetchHost, HostError, HostRequest, HostResponse, Keys};
use underbite_ffi::map::MapError;
use underbite_ffi::map_stations::{MapStations, StationPin, map_stations, station_pin_roles};
use underbite_ffi::settings::default_settings_json;
use underbite_providers::request::redact_url;

/// When the responses were recorded.
fn now() -> i64 {
    parse_rfc3339("2026-11-08T14:30:00Z").expect("time").0
}

/// The default settings with the station layers `shown` turned on.
fn settings(shown: &[&str]) -> String {
    let mut json: Value = serde_json::from_str(&default_settings_json()).expect("json");
    let layers = json["map"]["layers"].as_array_mut().expect("layers");
    for layer in layers {
        if shown.contains(&layer["id"].as_str().unwrap_or_default()) {
            layer["shown"] = Value::Bool(true);
        }
    }
    json.to_string()
}

const ALL_STATIONS: &[&str] = &[
    "tide-stations",
    "current-stations",
    "usgs-gauges",
    "buoys",
    "nwps-gauges",
];

// ---------------------------------------------------------------------------------------------
// A host that answers from recorded files
// ---------------------------------------------------------------------------------------------

/// The CO-OPS and NDBC lists, the NWPS gauges of the lower Hudson and the USGS latest values of
/// three New York sites, recorded on 2026-11-08.
const COMMON: &[(&str, &str)] = &[
    (
        "stations.json?type=tidepredictions",
        "coops/coops-mdapi-stations-tidepredictions-newyork-trimmed.json",
    ),
    (
        "stations.json?type=waterlevels",
        "coops/coops-mdapi-stations-waterlevels-newyork-trimmed.json",
    ),
    (
        "stations.json?type=currentpredictions",
        "coops/coops-mdapi-stations-currentpredictions-newyork-trimmed.json",
    ),
    (
        "ndbc.noaa.gov/activestations.xml",
        "ndbc/ndbc-activestations.xml",
    ),
    (
        "nwps/v1/gauges?bbox",
        "nwps/nwps-gauges-bbox-lower-hudson.json",
    ),
    (
        "collections/latest-continuous",
        "usgs/usgs-latest-continuous-multi-site.json",
    ),
];

/// USGS sites around Manhattan: the stream sites of a Manhattan box and the series of the
/// Hudson at Pier 25. No estuary, lake or tidal-stream site was recorded there; those searches
/// get the recorded empty answer of the Croton box.
const MANHATTAN_SITES: &[(&str, &str)] = &[
    (
        "site_type_code=ST-TS",
        "usgs/usgs-monitoring-locations-bbox-croton-es.json",
    ),
    (
        "site_type_code=ST&",
        "usgs/usgs-monitoring-locations-bbox-manhattan-st.json",
    ),
    (
        "site_type_code=ES",
        "usgs/usgs-monitoring-locations-bbox-croton-es.json",
    ),
    (
        "site_type_code=LK",
        "usgs/usgs-monitoring-locations-bbox-croton-es.json",
    ),
    (
        "collections/time-series-metadata",
        "usgs/usgs-time-series-metadata-hudson-pier-25-01376520.json",
    ),
];

/// USGS sites around the Croton River. No tidal-stream search was recorded there; it gets the
/// recorded empty estuary answer.
const CROTON_SITES: &[(&str, &str)] = &[
    (
        "site_type_code=ST-TS",
        "usgs/usgs-monitoring-locations-bbox-croton-es.json",
    ),
    (
        "site_type_code=ST&",
        "usgs/usgs-monitoring-locations-bbox-croton-st.json",
    ),
    (
        "site_type_code=ES",
        "usgs/usgs-monitoring-locations-bbox-croton-es.json",
    ),
    (
        "site_type_code=LK",
        "usgs/usgs-monitoring-locations-bbox-croton-lk.json",
    ),
    (
        "collections/time-series-metadata",
        "usgs/usgs-time-series-metadata-bbox-croton.json",
    ),
];

/// Answers each request with the first route whose pattern its URL contains, and panics with
/// the redacted URL on any other, so no test passes without a source. Keeps every request,
/// the stored values and the ledger in memory.
struct Recorded {
    routes: Vec<(&'static str, &'static str)>,
    seen: Mutex<Vec<HostRequest>>,
    values: Mutex<BTreeMap<(String, String), String>>,
}

impl Recorded {
    fn new(sites: &[(&'static str, &'static str)]) -> Arc<Self> {
        Arc::new(Self {
            routes: sites.iter().chain(COMMON).copied().collect(),
            seen: Mutex::new(Vec::new()),
            values: Mutex::new(BTreeMap::new()),
        })
    }

    fn urls(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.url.clone())
            .collect()
    }

    fn forget_requests(&self) {
        self.seen.lock().unwrap().clear();
    }

    fn host(self: &Arc<Self>) -> Arc<dyn FetchHost> {
        Arc::clone(self) as Arc<dyn FetchHost>
    }
}

fn raw(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/raw")
        .join(file)
}

#[async_trait::async_trait]
impl FetchHost for Recorded {
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
        let url = request.url.clone();
        self.seen.lock().unwrap().push(request);
        let Some((_, file)) = self.routes.iter().find(|(m, _)| url.contains(m)) else {
            panic!("no recorded response for {}", redact_url(&url));
        };
        if let Some(status) = file.strip_prefix("status:") {
            return Ok(HostResponse {
                status: status.parse().expect("a status"),
                body: Vec::new(),
                content_type: None,
                date: None,
                expires: None,
                last_modified: None,
                etag: None,
                retry_after: None,
                rate_limit_remaining: None,
                from_cache: false,
            });
        }
        let path = raw(file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let content_type = if Path::new(file)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
        {
            "text/xml"
        } else {
            "application/json"
        };
        Ok(HostResponse {
            status: 200,
            body,
            content_type: Some(content_type.into()),
            date: None,
            expires: None,
            last_modified: None,
            etag: None,
            retry_after: None,
            rate_limit_remaining: None,
            from_cache: false,
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

fn pins(
    view: [f64; 4],
    zoom: f64,
    shown: &[&str],
    host: &Arc<Recorded>,
) -> Result<MapStations, MapError> {
    pins_with(view, zoom, settings(shown), host)
}

fn pins_with(
    view: [f64; 4],
    zoom: f64,
    settings_json: String,
    host: &Arc<Recorded>,
) -> Result<MapStations, MapError> {
    let [west, south, east, north] = view;
    pollster::block_on(map_stations(
        west,
        south,
        east,
        north,
        zoom,
        settings_json,
        Keys::default(),
        now(),
        host.host(),
    ))
}

/// [`settings`] in the plain voice.
fn plain(shown: &[&str]) -> String {
    let mut json: Value = serde_json::from_str(&settings(shown)).expect("json");
    json["presentation"]["voice"] = Value::String("plain".into());
    json.to_string()
}

fn pin<'a>(found: &'a MapStations, id: &str) -> &'a StationPin {
    found
        .pins
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("no pin {id} among {:?}", ids(found)))
}

fn ids(found: &MapStations) -> Vec<&str> {
    found.pins.iter().map(|p| p.id.as_str()).collect()
}

const BATTERY: [f64; 4] = [-74.10, 40.62, -73.95, 40.78];
const CROTON: [f64; 4] = [-73.95, 41.15, -73.75, 41.30];
/// The Ramapo, Saddle and Hackensack rivers and the Hudson at Piermont.
const LOWER_HUDSON: [f64; 4] = [-74.30, 40.90, -73.80, 41.15];

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn a_battery_view_has_tide_current_buoy_and_gauge_pins() {
    let host = Recorded::new(MANHATTAN_SITES);
    let found = pins(BATTERY, 13.0, ALL_STATIONS, &host).expect("pins");
    assert_eq!(found.errors, []);

    let battery = pin(&found, "coops:8518750");
    assert_eq!(
        (battery.kind.as_str(), battery.layer.as_str()),
        ("tide", "tide-stations")
    );
    assert_eq!(battery.name, "NEW YORK (The Battery)");
    assert_eq!(battery.state_role, "none");
    assert_eq!(battery.label, "Tide station");
    assert!((battery.lat - 40.7006).abs() < 1e-3 && (battery.lon + 74.0142).abs() < 1e-3);

    // One pin per current station, whatever its bins.
    let entrance = pin(&found, "coops:NYH1927");
    assert_eq!(
        (entrance.kind.as_str(), entrance.label.as_str()),
        ("current", "Current station")
    );
    assert_eq!(
        ids(&found)
            .iter()
            .filter(|id| id.contains("NYH1927"))
            .count(),
        1
    );

    let ndbc = pin(&found, "ndbc:batn6");
    assert_eq!((ndbc.kind.as_str(), ndbc.layer.as_str()), ("buoy", "buoys"));
    assert_eq!(ndbc.label, "Coastal station");

    // Pier 25's series ended in April: no value, and no request for one.
    let pier = pin(&found, "usgs:USGS-01376520");
    assert_eq!(pier.layer, "usgs-gauges");
    assert_eq!(pier.state_role, "no_data");
    // The default voice is Luna's.
    assert_eq!(pier.label, "River gauge · Nothing lately");
    // Pier 84 serves nothing the app reads.
    assert!(!ids(&found).contains(&"usgs:USGS-01376515"));
    // No NWPS gauge of the recorded list lies in the view; nothing outside it is pinned.
    assert!(found.pins.iter().all(|p| p.kind != "forecast_gauge"));
    assert!(found.pins.iter().all(|p| {
        (BATTERY[1]..=BATTERY[3]).contains(&p.lat) && (BATTERY[0]..=BATTERY[2]).contains(&p.lon)
    }));
    let roles = station_pin_roles();
    assert!(found.pins.iter().all(|p| roles.contains(&p.state_role)));

    let urls = host.urls();
    assert!(
        !urls.iter().any(|u| u.contains("latest-continuous")),
        "{urls:?}"
    );
    // Box lists go out by cell, centred on whole and half degrees: never the view itself.
    assert!(
        urls.iter()
            .any(|u| u.contains("bbox=-74.25,40.25,-73.75,40.75&")),
        "{urls:?}"
    );
    assert!(
        urls.iter().any(
            |u| u.contains("bbox.xmin=-74.25&bbox.ymin=40.75&bbox.xmax=-73.75&bbox.ymax=41.25")
        )
    );
    assert!(
        !urls
            .iter()
            .any(|u| u.contains("40.62") || u.contains("-74.1,"))
    );

    // The lists are stored: the same view again sends nothing.
    host.forget_requests();
    let again = pins(BATTERY, 13.0, ALL_STATIONS, &host).expect("pins");
    assert_eq!(again, found);
    assert_eq!(host.urls(), Vec::<String>::new());
}

#[test]
fn croton_gauges_take_their_latest_value_from_one_batched_request() {
    let host = Recorded::new(CROTON_SITES);
    let found = pins_with(CROTON, 12.0, plain(&["usgs-gauges"]), &host).expect("pins");
    assert_eq!(found.errors, []);
    let dam = pin(&found, "usgs:USGS-01375000");
    assert_eq!(dam.state_role, "reporting");
    assert_eq!(dam.label, "River gauge · 12.4 cfs, 45 min ago");
    // A site whose series ended in 2009 is not asked about.
    let hunter = pin(&found, "usgs:USGS-0137499350");
    assert_eq!(hunter.state_role, "no_data");
    assert_eq!(hunter.label, "River gauge · No recent value");
    // Only the layer shown is fetched.
    assert!(found.pins.iter().all(|p| p.layer == "usgs-gauges"));

    let urls = host.urls();
    let latest: Vec<&String> = urls
        .iter()
        .filter(|u| u.contains("latest-continuous"))
        .collect();
    assert_eq!(latest.len(), 1, "{urls:?}");
    assert!(
        latest[0].contains("monitoring_location_id=USGS-01375000&"),
        "{}",
        latest[0]
    );
    assert!(
        !urls
            .iter()
            .any(|u| u.contains("tidesandcurrents") || u.contains("ndbc"))
    );
    // Four cells of five site requests each, and one for the values.
    assert_eq!(urls.len(), 21, "{urls:?}");
}

#[test]
fn a_spent_budget_gives_pins_without_state_and_no_request() {
    let host = Recorded::new(CROTON_SITES);
    pins(CROTON, 12.0, &["usgs-gauges"], &host).expect("pins");
    // Spend the hour: the ledger as the fetch saves it, at USGS's hourly limit.
    let hour = now() - now().rem_euclid(3_600);
    host.store(
        "usage".into(),
        "ledger".into(),
        format!(
            r#"{{"saved_at":{},"value":{{"counters":{{"usgs":{{"period":"hour","start":{hour},"used":1000}}}}}}}}"#,
            now()
        ),
    );
    host.forget_requests();

    let found = pins(CROTON, 12.0, &["usgs-gauges"], &host).expect("pins");
    assert_eq!(host.urls(), Vec::<String>::new());
    let dam = pin(&found, "usgs:USGS-01375000");
    assert_eq!(dam.state_role, "unknown");
    assert_eq!(dam.label, "River gauge");
    assert_eq!(pin(&found, "usgs:USGS-0137499350").state_role, "no_data");
    assert!(
        found
            .errors
            .iter()
            .any(|e| e.source == "usgs" && e.kind == "rate_limited"),
        "{:?}",
        found.errors
    );
}

#[test]
fn nwps_gauges_show_their_flood_category() {
    let host = Recorded::new(&[]);
    let found = pins(LOWER_HUDSON, 11.0, &["nwps-gauges"], &host).expect("pins");
    assert_eq!(found.errors, []);
    let piermont = pin(&found, "nwps:PMTN6");
    assert_eq!(
        (piermont.kind.as_str(), piermont.layer.as_str()),
        ("forecast_gauge", "nwps-gauges")
    );
    assert_eq!(piermont.state_role, "normal");
    assert_eq!(
        piermont.label,
        "River forecast gauge · No flooding, 1 h ago"
    );
    let pompton = pin(&found, "nwps:PMPN4");
    assert_eq!(pompton.state_role, "reporting");
    assert_eq!(
        pompton.label,
        "River forecast gauge · No flood stages, 1 h ago"
    );
    let pelham = pin(&found, "nwps:HAMN6");
    assert_eq!(pelham.state_role, "no_data");
    assert_eq!(pelham.label, "River forecast gauge · Out of service");
    // West Point is north of the view.
    assert!(!ids(&found).contains(&"nwps:WPTN6"));

    let urls = host.urls();
    assert_eq!(urls.len(), 2, "{urls:?}");
    assert!(urls.iter().all(|u| u.contains("nwps/v1/gauges?bbox")));
}

#[test]
fn a_failed_site_search_is_not_asked_again_within_the_hour() {
    // The lake search fails for every cell; the other site types answer.
    let sites: Vec<(&str, &str)> = [("site_type_code=LK", "status:503")]
        .iter()
        .chain(CROTON_SITES)
        .copied()
        .collect();
    let host = Recorded::new(&sites);
    let at = |t: i64| {
        pollster::block_on(map_stations(
            CROTON[0],
            CROTON[1],
            CROTON[2],
            CROTON[3],
            12.0,
            settings(&["usgs-gauges"]),
            Keys::default(),
            t,
            host.host(),
        ))
        .expect("pins")
    };
    let site_searches = |host: &Recorded| {
        host.urls()
            .iter()
            .filter(|u| u.contains("site_type_code=") || u.contains("time-series-metadata"))
            .count()
    };
    let first = at(now());
    assert!(
        first.errors.iter().any(|e| e.source == "usgs"),
        "{:?}",
        first.errors
    );
    assert_eq!(site_searches(&host), 20);
    pin(&first, "usgs:USGS-01375000");

    // Within the hour the sites found answer, and no search is spent again.
    host.forget_requests();
    let again = at(now() + 600);
    assert_eq!(site_searches(&host), 0, "{:?}", host.urls());
    pin(&again, "usgs:USGS-01375000");

    // After it the cells are searched again.
    host.forget_requests();
    at(now() + 3_601);
    assert_eq!(site_searches(&host), 20);
}

#[test]
fn zoomed_out_views_fetch_no_box_lists() {
    let host = Recorded::new(CROTON_SITES);
    let found = pins(CROTON, 7.0, &["usgs-gauges", "nwps-gauges"], &host).expect("pins");
    assert_eq!(found.pins, []);
    assert_eq!(host.urls(), Vec::<String>::new());
}

#[test]
fn whole_lists_stored_by_discovery_are_used() {
    // The map keeps its copy beside discovery's, in the same shape.
    let first = Recorded::new(&[]);
    let found = pins(BATTERY, 13.0, &["buoys"], &first).expect("pins");
    let ours = first.load("stations".into(), "map-ndbc-active".into());
    assert!(ours.is_some());
    assert_eq!(first.load("stations".into(), "ndbc-active".into()), None);

    // A list discovery stored answers without a request.
    let second = Recorded::new(&[]);
    second.store("stations".into(), "ndbc-active".into(), ours.expect("list"));
    let again = pins(BATTERY, 13.0, &["buoys"], &second).expect("pins");
    assert_eq!(again, found);
    assert_eq!(second.urls(), Vec::<String>::new());

    // Six weeks on, the offline cache no longer serves it: the list is fetched again.
    let later = now() + 6 * 7 * 86_400 + 1;
    let refetched = pollster::block_on(map_stations(
        BATTERY[0],
        BATTERY[1],
        BATTERY[2],
        BATTERY[3],
        13.0,
        settings(&["buoys"]),
        Keys::default(),
        later,
        second.host(),
    ))
    .expect("pins");
    assert_eq!(refetched.pins, found.pins);
    assert_eq!(second.urls().len(), 1);
}

#[test]
fn a_stored_list_from_any_time_never_overflows_its_age() {
    let first = Recorded::new(&[]);
    let found = pins(BATTERY, 13.0, &["buoys"], &first).expect("pins");
    let ours = first
        .load("stations".into(), "map-ndbc-active".into())
        .expect("list");
    let mut stored: Value = serde_json::from_str(&ours).expect("json");
    stored["saved_at"] = Value::from(i64::MIN);
    let second = Recorded::new(&[]);
    second.store("stations".into(), "ndbc-active".into(), stored.to_string());
    // Past any limit: fetched again.
    let again = pins(BATTERY, 13.0, &["buoys"], &second).expect("pins");
    assert_eq!(again.pins, found.pins);
    assert_eq!(second.urls().len(), 1);
}

#[test]
fn bad_views_and_settings_are_refused() {
    let host = Recorded::new(&[]);
    assert!(matches!(
        pins([-74.0, 41.0, -73.0, 40.0], 10.0, ALL_STATIONS, &host),
        Err(MapError::Invalid { .. })
    ));
    assert!(matches!(
        pins([f64::NAN, 40.0, -73.0, 41.0], 10.0, ALL_STATIONS, &host),
        Err(MapError::Invalid { .. })
    ));
    let bad = pollster::block_on(map_stations(
        -74.0,
        40.0,
        -73.0,
        41.0,
        10.0,
        "{".into(),
        Keys::default(),
        now(),
        host.host(),
    ));
    assert!(matches!(bad, Err(MapError::Invalid { .. })));
    assert_eq!(host.urls(), Vec::<String>::new());
}
