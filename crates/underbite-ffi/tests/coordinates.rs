//! Every coordinate that leaves the phone is rounded: a fetch and a discovery of spots saved
//! with six decimals are run against hosts that keep every request, and each URL and body is
//! checked. The configured precision is at most 2 decimals; the user may choose coarser.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use underbite_core::time::parse_rfc3339;
use underbite_ffi::fetch::{
    FetchHost, FetchOptions, HostError, HostRequest, HostResponse, Keys, discover_stations,
    fetch_spot,
};
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::view::SpotInfo;
use underbite_providers::request::redact_url;

fn now() -> i64 {
    parse_rfc3339("2026-11-08T14:30:00Z").expect("time").0
}

/// The Battery to six decimals: the saved spot keeps all of them.
fn battery() -> SpotInfo {
    SpotInfo {
        id: "battery".into(),
        name: "The Battery".into(),
        latitude: 40.703_312,
        longitude: -74.016_987,
        water_type: "tidal_river_estuary".into(),
        time_zone: "America/New_York".into(),
    }
}

/// The Croton River below the dam, to six decimals.
fn croton() -> SpotInfo {
    SpotInfo {
        id: "croton".into(),
        name: "Croton River below the dam".into(),
        latitude: 41.225_213,
        longitude: -73.858_786,
        water_type: "river_stream".into(),
        time_zone: "America/New_York".into(),
    }
}

/// The digits of the exact coordinates that must never be sent.
const EXACT: [&str; 4] = ["40.703", "74.016", "41.225", "73.858"];

#[derive(serde::Deserialize)]
struct Route {
    #[serde(rename = "match")]
    matches: String,
    file: PathBuf,
    status: Option<u16>,
    content_type: Option<String>,
}

#[derive(serde::Deserialize)]
struct Routes {
    route: Vec<Route>,
}

/// Answers from a replay scenario, or with 404 to everything without one; keeps every request.
/// A replay panics on a request no route answers, with its redacted URL.
#[derive(Default)]
struct Recording {
    base: PathBuf,
    routes: Option<Vec<Route>>,
    seen: Mutex<Vec<HostRequest>>,
    values: Mutex<BTreeMap<(String, String), String>>,
}

impl Recording {
    fn replay(scenario: &str) -> Arc<Self> {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/replay")
            .join(scenario);
        let text = std::fs::read_to_string(base.join("routes.toml")).expect("routes");
        let routes: Routes = toml::from_str(&text).expect("routes file");
        Arc::new(Self {
            base,
            routes: Some(routes.route),
            ..Self::default()
        })
    }

    fn not_found() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn seen(&self) -> Vec<HostRequest> {
        self.seen.lock().unwrap().clone()
    }
}

fn answer(status: u16, body: Vec<u8>, content_type: &str) -> HostResponse {
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
        from_cache: false,
    }
}

#[async_trait::async_trait]
impl FetchHost for Recording {
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
        let url = request.url.clone();
        self.seen.lock().unwrap().push(request);
        let Some(routes) = &self.routes else {
            return Ok(answer(404, b"Not Found".to_vec(), "text/plain"));
        };
        let Some(route) = routes.iter().find(|r| url.contains(&r.matches)) else {
            panic!("no replay route for {}", redact_url(&url));
        };
        let path = self.base.join(&route.file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let kind = route.content_type.as_deref().unwrap_or(
            match path.extension().and_then(|e| e.to_str()) {
                Some("json") => "application/json",
                Some("xml") => "text/xml",
                Some("html") => "text/html",
                _ => "text/plain",
            },
        );
        Ok(answer(route.status.unwrap_or(200), body, kind))
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

/// A fetch and then a discovery of `spot` through `host`; the fetch infers the water type
/// first when `infer` (more lookups around the spot).
fn fetch_and_discover(spot: &SpotInfo, settings: &str, infer: bool, host: &Arc<Recording>) {
    let options = FetchOptions {
        allow_metered: true,
        infer_water_type: infer,
        refresh_metadata: false,
    };
    let dyn_host = || Arc::clone(host) as Arc<dyn FetchHost>;
    pollster::block_on(fetch_spot(
        spot.clone(),
        settings.into(),
        Keys::default(),
        options,
        now(),
        Vec::new(),
        dyn_host(),
    ))
    .expect("fetched");
    // A fresh store, so discovery asks for its lists again.
    host.values.lock().unwrap().clear();
    pollster::block_on(discover_stations(
        spot.clone(),
        settings.into(),
        Keys::default(),
        true,
        now(),
        dyn_host(),
    ))
    .expect("discovered");
}

/// The numbers in `text`, as written.
fn numbers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter(|t| t.parse::<f64>().is_ok())
}

/// Fails unless every coordinate around `spot` in `text` (a URL or a body) has at most
/// `decimals` decimals. A box (`bbox=`, `bbox.xmin=`..., or a 3DHP query's `geometry=`
/// envelope) spans a fixed distance around the rounded spot: its centre is checked.
fn assert_rounded(text: &str, spot: &SpotInfo, decimals: usize) {
    let scale = 10f64.powi(i32::try_from(decimals).unwrap());
    let on_grid = |v: f64| ((v * scale).round() - v * scale).abs() < 1e-6;
    // A number within 2 degrees of the spot's latitude or longitude is a coordinate.
    let near = |v: f64| (v - spot.latitude).abs() < 2.0 || (v - spot.longitude).abs() < 2.0;
    for exact in EXACT {
        assert!(!text.contains(exact), "{exact} in {text}");
    }
    let (path, query) = text.split_once('?').unwrap_or((text, ""));
    let mut rest = vec![path];
    let mut named_box: BTreeMap<&str, f64> = BTreeMap::new();
    let mut boxes: Vec<Vec<f64>> = Vec::new();
    for param in query.split('&') {
        let (name, value) = param.split_once('=').unwrap_or((param, ""));
        if let Some(side) = name.strip_prefix("bbox.") {
            named_box.insert(side, value.parse().expect("number"));
        } else if name == "bbox" || name == "geometry" {
            boxes.push(
                value
                    .split(',')
                    .map(|v| v.parse().expect("number"))
                    .collect(),
            );
        } else {
            rest.push(param);
        }
    }
    if !named_box.is_empty() {
        let side = |s: &str| named_box.get(s).copied().expect("box side");
        boxes.push(vec![side("xmin"), side("ymin"), side("xmax"), side("ymax")]);
    }
    for n in boxes {
        let centre = match n[..] {
            [x0, y0, x1, y1] => [f64::midpoint(x0, x1), f64::midpoint(y0, y1)],
            [x, y] => [x, y],
            _ => panic!("{text}"),
        };
        assert!(centre.into_iter().all(on_grid), "{text}");
    }
    for part in rest {
        for token in numbers(part) {
            let value: f64 = token.parse().expect("number");
            assert!(!near(value) || on_grid(value), "{text}: {token}");
        }
    }
}

fn assert_all_rounded(host: &Recording, spot: &SpotInfo, decimals: usize) {
    let seen = host.seen();
    assert!(seen.len() > 5, "{} requests", seen.len());
    for request in &seen {
        assert_rounded(&request.url, spot, decimals);
        if let Some(body) = &request.body {
            assert_rounded(body, spot, decimals);
        }
    }
}

#[test]
fn rounding_checks_catch_precise_coordinates() {
    let spot = battery();
    assert_rounded(
        "https://a.example/x?lat=40.7&lon=-74.02&tolerance=0.0005",
        &spot,
        2,
    );
    assert_rounded(
        "https://a.example/x?bbox=-74.27,40.45,-73.77,40.95",
        &spot,
        2,
    );
    let precise = std::panic::catch_unwind(|| {
        assert_rounded("https://a.example/points/40.71,-74.019", &battery(), 2);
    });
    assert!(precise.is_err());
    let coarse = std::panic::catch_unwind(|| {
        assert_rounded(
            "https://a.example/x?bbox.xmin=-74.27&bbox.ymin=40.45&bbox.xmax=-73.77&bbox.ymax=40.95",
            &battery(),
            1,
        );
    });
    assert!(coarse.is_err());
}

#[test]
fn the_battery_is_sent_rounded() {
    let host = Recording::replay("battery");
    fetch_and_discover(&battery(), &default_settings_json(), true, &host);
    assert_all_rounded(&host, &battery(), 2);
    // The point lookups went out, and with the rounded spot.
    assert!(
        host.seen()
            .iter()
            .any(|r| r.url.contains("api.weather.gov/points/40.7,-74.02"))
    );
}

#[test]
fn the_croton_is_sent_rounded() {
    let host = Recording::replay("croton");
    fetch_and_discover(&croton(), &default_settings_json(), false, &host);
    assert_all_rounded(&host, &croton(), 2);
}

#[test]
fn a_coarser_precision_is_used_everywhere() {
    let mut settings: Value = serde_json::from_str(&default_settings_json()).expect("json");
    settings["sources"]["coordinate_precision"] = 1.into();
    let settings = settings.to_string();
    for spot in [battery(), croton()] {
        let host = Recording::not_found();
        fetch_and_discover(&spot, &settings, true, &host);
        assert_all_rounded(&host, &spot, 1);
    }
}
