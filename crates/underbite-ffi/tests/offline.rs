//! Working offline as the app does: a fetch without signal that computes tides from the
//! station metadata it kept, pack plans read a page at a time, and tide and current tables.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use underbite_core::model::SpotSnapshot;
use underbite_core::provenance::{SourceId, ValueKind};
use underbite_core::time::parse_rfc3339;
use underbite_ffi::fetch::{
    CachedResponse, FetchHost, FetchOptions, HostError, HostRequest, HostResponse, Keys,
    SpotFetchOutcome, fetch_spot,
};
use underbite_ffi::map::GridStore;
use underbite_ffi::offline::{
    PackItemKind, PackRequest, current_table_view, pack_requests, plan_pack, storage_size_text,
    storage_view, tide_table_view,
};
use underbite_ffi::settings::default_settings_json;
use underbite_ffi::view::SpotInfo;

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

const OPTIONS: FetchOptions = FetchOptions {
    allow_metered: true,
    infer_water_type: false,
    refresh_metadata: false,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn raw(name: &str) -> String {
    let path = fixtures().join("raw/coops").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// A host that replays the Battery scenario until the signal is lost
// ---------------------------------------------------------------------------------------------

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

struct Phone {
    base: PathBuf,
    routes: Vec<Route>,
    /// Every request fails while this is set.
    offline: AtomicBool,
    /// Every request gets this status and a sign-in page while set, as behind a captive portal.
    portal: Mutex<Option<u16>>,
    /// When set, only requests whose URL contains this text meet the portal.
    portal_only: Mutex<Option<String>>,
    /// While set, only requests whose URL contains this text are answered, from the platform's
    /// cache; every other request fails at the transport.
    cache_only: Mutex<Option<String>>,
    /// Requests whose URL contains one of these texts get a 503.
    broken: Mutex<Vec<String>>,
    seen: Mutex<Vec<HostRequest>>,
    values: Mutex<BTreeMap<(String, String), String>>,
}

impl Phone {
    fn new() -> Arc<Self> {
        let base = fixtures().join("replay/battery");
        let text = std::fs::read_to_string(base.join("routes.toml")).expect("routes");
        let routes: Routes = toml::from_str(&text).expect("routes file");
        Arc::new(Self {
            base,
            routes: routes.route,
            offline: AtomicBool::new(false),
            portal: Mutex::new(None),
            portal_only: Mutex::new(None),
            cache_only: Mutex::new(None),
            broken: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            values: Mutex::new(BTreeMap::new()),
        })
    }

    fn host(self: &Arc<Self>) -> Arc<dyn FetchHost> {
        Arc::clone(self) as Arc<dyn FetchHost>
    }

    fn lose_signal(&self) {
        self.offline.store(true, Ordering::SeqCst);
    }

    fn behind_portal(&self, status: u16) {
        *self.portal.lock().unwrap() = Some(status);
    }

    /// Only requests whose URL contains `part` meet the portal; the rest are answered.
    fn behind_portal_for(&self, status: u16, part: &str) {
        *self.portal_only.lock().unwrap() = Some(part.to_owned());
        self.behind_portal(status);
    }

    /// From now on only requests containing `part` are answered, from the cache.
    fn serve_from_cache(&self, part: &str) {
        *self.cache_only.lock().unwrap() = Some(part.to_owned());
    }

    fn break_requests(&self, parts: &[&str]) {
        *self.broken.lock().unwrap() = parts.iter().map(|p| (*p).to_owned()).collect();
    }

    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The requests for constants, datums or offsets sent since the first `from`.
    fn constants_asked(&self, from: usize) -> Vec<HostRequest> {
        self.seen.lock().unwrap()[from..]
            .iter()
            .filter(|r| {
                ["/harcon.json", "/datums.json", "offsets.json"]
                    .iter()
                    .any(|file| r.url.contains(file))
            })
            .cloned()
            .collect()
    }
}

#[async_trait::async_trait]
impl FetchHost for Phone {
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
        self.seen.lock().unwrap().push(request.clone());
        if self.offline.load(Ordering::SeqCst) {
            return Err(HostError::Failed {
                message: "The Internet connection appears to be offline.".into(),
            });
        }
        let cache_only = self.cache_only.lock().unwrap().clone();
        if let Some(part) = &cache_only
            && !request.url.contains(part)
        {
            return Err(HostError::Failed {
                message: "The Internet connection appears to be offline.".into(),
            });
        }
        let from_cache = cache_only.is_some();
        if self
            .broken
            .lock()
            .unwrap()
            .iter()
            .any(|part| request.url.contains(part))
        {
            return Ok(HostResponse {
                status: 503,
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
        let portal = *self.portal.lock().unwrap();
        let only = self.portal_only.lock().unwrap().clone();
        if let Some(status) = portal
            && only.is_none_or(|part| request.url.contains(&part))
        {
            return Ok(HostResponse {
                status,
                body: b"<!DOCTYPE html><html><body>Sign in to the network</body></html>".to_vec(),
                content_type: Some("text/html; charset=utf-8".into()),
                date: None,
                expires: None,
                last_modified: None,
                etag: None,
                retry_after: None,
                rate_limit_remaining: None,
                from_cache: false,
            });
        }
        let url = request.url;
        let route = self
            .routes
            .iter()
            .find(|r| url.contains(&r.matches))
            .unwrap_or_else(|| panic!("no replay route for {}", url.split('?').next().unwrap()));
        let path = self.base.join(&route.file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let kind = match path.extension().and_then(|e| e.to_str()) {
            Some("json") => "application/json",
            Some("xml") => "text/xml",
            _ => "text/plain",
        };
        Ok(HostResponse {
            status: route.status.unwrap_or(200),
            body,
            content_type: Some(route.content_type.clone().unwrap_or_else(|| kind.into())),
            date: None,
            expires: None,
            last_modified: None,
            etag: None,
            retry_after: None,
            rate_limit_remaining: None,
            from_cache,
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

fn fetch(
    phone: &Arc<Phone>,
    at: i64,
    cached: Vec<CachedResponse>,
) -> (SpotSnapshot, SpotFetchOutcome) {
    fetch_with(phone, at, cached, OPTIONS)
}

fn fetch_with(
    phone: &Arc<Phone>,
    at: i64,
    cached: Vec<CachedResponse>,
    options: FetchOptions,
) -> (SpotSnapshot, SpotFetchOutcome) {
    let out = pollster::block_on(fetch_spot(
        battery(),
        default_settings_json(),
        Keys::default(),
        options,
        at,
        cached,
        phone.host(),
    ))
    .expect("fetched");
    let snapshot = serde_json::from_str(&out.snapshot_json).expect("snapshot");
    (snapshot, out)
}

// ---------------------------------------------------------------------------------------------
// A fetch without signal
// ---------------------------------------------------------------------------------------------

#[test]
fn a_fetch_without_signal_computes_the_tide_and_keeps_the_last_values() {
    let phone = Phone::new();
    let (online, first) = fetch(&phone, now(), Vec::new());
    // Only metadata of requests the fetch made is offered, once per key, nothing else.
    let kept = &first.cacheable;
    assert!(
        kept.iter()
            .any(|c| c.key.contains("stations.json?type=tidepredictions"))
    );
    let keys: BTreeSet<&str> = kept.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys.len(), kept.len());
    assert!(
        kept.iter()
            .all(|c| c.key.contains("/mdapi/") || c.key.contains("activestations")),
        "{:?}",
        kept.iter().map(|c| &c.key).collect::<Vec<_>>()
    );
    // The fetch kept what computes the tide and the current without signal: the Battery's
    // constants and datums, and the Hudson River Entrance's constants for its bin.
    for key in [
        format!("{BATTERY_MDAPI}/harcon.json?units=metric"),
        format!("{BATTERY_MDAPI}/datums.json?units=metric"),
        format!("{MDAPI}/stations/NYH1927/harcon.json?bin=13"),
    ] {
        assert!(keys.contains(key.as_str()), "{key} not in {keys:?}");
    }
    let cached = first.cacheable.clone();
    let water = online
        .water
        .temperature_c
        .as_ref()
        .expect("a water temperature");
    let observed = water.samples().last().expect("a sample").time;

    // The signal goes: every request fails, the app hands back what it kept.
    phone.lose_signal();
    let before = phone.requests();
    let later = now() + 6 * 3_600;
    let (s, out) = fetch(&phone, later, cached.clone());
    assert!(phone.requests() > before, "the fetch tried the network");

    // The tide is computed on the device from the kept constants.
    let tide = s.marine.tide.as_ref().expect("a computed tide");
    assert_eq!(tide.provenance.kind, ValueKind::Computed);
    assert_eq!(tide.provenance.source, SourceId::COOPS_HARMONICS);
    assert!(!tide.events.is_empty() && tide.curve.is_some());
    assert!(
        tide.events.iter().any(|e| e.time.0 > later),
        "events reach past the new time"
    );
    assert_current_table(&cached);

    // The other values are the last snapshot's, with their own times.
    let water = s
        .water
        .temperature_c
        .as_ref()
        .expect("the last water temperature");
    assert_eq!(water.samples().last().expect("a sample").time, observed);
    assert_eq!(
        water.provenance(),
        online.water.temperature_c.as_ref().unwrap().provenance()
    );
    assert_eq!(
        s.fetched_at, online.fetched_at,
        "the snapshot was last fetched online"
    );
    assert!(s.weather.is_some());
    assert_eq!(s.stations.len(), online.stations.len());

    // The outcome says so, in words from the core.
    let note = out
        .errors
        .iter()
        .find(|e| e.message.starts_with("offline:"))
        .expect("the offline note");
    assert_eq!(note.source, "offline");
    assert_eq!(note.kind, "stale");
    assert!(
        note.message.starts_with(
            "offline: computed from harmonic constants, other values from Nov 8, 9:30"
        ),
        "{}",
        note.message
    );
    assert!(out.cacheable.is_empty(), "nothing was fetched to keep");

    // What was shown is kept as the last snapshot: a second trip offline still has it, and
    // still names the last time online.
    let (again, out) = fetch(&phone, later + 3_600, cached);
    let water = again
        .water
        .temperature_c
        .as_ref()
        .expect("still the last temperature");
    assert_eq!(water.samples().last().expect("a sample").time, observed);
    assert!(again.marine.tide.is_some());
    let note = offline_note(&out);
    assert!(
        note.starts_with(
            "offline: computed from harmonic constants, other values from Nov 8, 9:30"
        ),
        "{note}"
    );
}

const MDAPI: &str = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi";
const BATTERY_MDAPI: &str =
    "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750";

/// The current station's constants in `kept` give its table without signal.
fn assert_current_table(kept: &[CachedResponse]) {
    let current = kept
        .iter()
        .find(|c| c.key.ends_with("/NYH1927/harcon.json?bin=13"))
        .expect("the current constants");
    let table = current_table_view(
        "NYH1927".into(),
        Some(13),
        String::from_utf8(current.body.clone()).expect("text"),
        None,
        default_settings_json(),
        "UTC".into(),
        "2026-11-08".into(),
        1,
    )
    .expect("a current table");
    assert_ne!(table.rows.len(), 0);
}

/// `kept` with the Battery's `harcon` and `datums` bodies, as the app stored them a day
/// earlier.
fn with_kept(kept: &[CachedResponse], harcon: Option<Vec<u8>>) -> Vec<CachedResponse> {
    let mut cached = kept.to_vec();
    let datums = raw("coops-mdapi-8518750-datums-metric.json").into_bytes();
    for (file, body) in [("harcon", harcon), ("datums", Some(datums))] {
        let Some(body) = body else {
            continue;
        };
        let url = format!("{BATTERY_MDAPI}/{file}.json?units=metric");
        cached.retain(|c| c.key != url);
        cached.push(CachedResponse {
            key: url,
            body,
            fetched_at: now() - 86_400,
        });
    }
    cached
}

/// `kept` with the Battery's constants and datums.
fn with_constants(kept: &[CachedResponse]) -> Vec<CachedResponse> {
    let harcon = raw("coops-mdapi-8518750-harcon-metric.json").into_bytes();
    with_kept(kept, Some(harcon))
}

/// The outcome's offline note.
fn offline_note(out: &SpotFetchOutcome) -> String {
    out.errors
        .iter()
        .find(|e| e.message.starts_with("offline:"))
        .unwrap_or_else(|| panic!("no offline note in {:?}", out.errors))
        .message
        .clone()
}

#[test]
fn fetches_without_signal_keep_the_time_of_the_last_one_online() {
    let phone = Phone::new();
    let (online, first) = fetch(&phone, now(), Vec::new());
    let cached = with_constants(&first.cacheable);
    phone.lose_signal();
    for hours in [6, 7, 8] {
        let (s, out) = fetch(&phone, now() + hours * 3_600, cached.clone());
        assert_eq!(s.fetched_at, online.fetched_at, "{hours} h later");
        let note = offline_note(&out);
        assert!(
            note.starts_with(
                "offline: computed from harmonic constants, other values from Nov 8, 9:30"
            ),
            "{hours} h later: {note}"
        );
    }
}

#[test]
fn kept_metadata_that_computes_nothing_is_not_called_a_computed_tide() {
    let phone = Phone::new();
    let (online, first) = fetch(&phone, now(), Vec::new());
    // The app lost the constants the online fetch kept.
    let kept: Vec<CachedResponse> = first
        .cacheable
        .iter()
        .filter(|c| !c.key.contains("harcon"))
        .cloned()
        .collect();
    assert!(kept.len() < first.cacheable.len(), "constants were kept");
    phone.lose_signal();
    // Datums alone, then datums with constants the predictor cannot use: copies answer, but
    // no tide is computed from them.
    for harcon in [None, Some(br#"{"HarmonicConstituents":[]}"#.to_vec())] {
        let cached = with_kept(&kept, harcon.clone());
        let (s, out) = fetch(&phone, now() + 3_600, cached);
        assert_eq!(s.marine.tide, online.marine.tide, "{harcon:?}");
        let note = offline_note(&out);
        assert!(
            note.starts_with("offline: values from Nov 8, 9:30"),
            "{harcon:?}: {note}"
        );
    }
}

#[test]
fn a_captive_portal_neither_hides_the_fallback_nor_replaces_kept_copies() {
    for status in [200, 302, 511] {
        let phone = Phone::new();
        let (online, first) = fetch(&phone, now(), Vec::new());
        let cached = with_constants(&first.cacheable);
        phone.behind_portal(status);
        let (s, out) = fetch(&phone, now() + 3_600, cached);
        assert!(
            out.cacheable.is_empty(),
            "{status}: offered {:?}",
            out.cacheable.iter().map(|c| &c.key).collect::<Vec<_>>()
        );
        let tide = s.marine.tide.as_ref().expect("a tide");
        assert_eq!(tide.provenance.kind, ValueKind::Computed, "{status}");
        assert_eq!(s.fetched_at, online.fetched_at, "{status}");
        let note = offline_note(&out);
        assert!(
            note.starts_with("offline: computed from harmonic constants"),
            "{status}: {note}"
        );
    }
}

#[test]
fn without_the_kept_metadata_no_tide_is_computed_and_the_last_one_stays() {
    let phone = Phone::new();
    let (online, _) = fetch(&phone, now(), Vec::new());
    let last_tide = online.marine.tide.clone().expect("a tide");
    phone.lose_signal();
    let (s, out) = fetch(&phone, now() + 3_600, Vec::new());
    let tide = s.marine.tide.as_ref().expect("the last tide");
    assert_eq!(tide, &last_tide, "carried with its own provenance");
    assert_ne!(tide.provenance.kind, ValueKind::Computed);
    let note = out
        .errors
        .iter()
        .find(|e| e.message.starts_with("offline:"))
        .expect("the offline note");
    assert!(
        note.message.starts_with("offline: values from Nov 8"),
        "{}",
        note.message
    );
}

#[test]
fn a_fetch_that_got_answers_is_not_offline() {
    let phone = Phone::new();
    let (s, out) = fetch(&phone, now(), Vec::new());
    assert!(s.errors.iter().all(|e| !e.message.starts_with("offline:")));
    assert_ne!(out.cacheable.len(), 0);
    assert!(!out.unanswered);
}

#[test]
fn a_portal_answering_every_request_is_unanswered_whatever_the_status() {
    for status in [200, 302, 511] {
        let phone = Phone::new();
        phone.behind_portal(status);
        let (_, out) = fetch(&phone, now(), Vec::new());
        assert!(out.unanswered, "{status}");
        assert!(out.cacheable.is_empty(), "{status}");
    }
}

#[test]
fn no_signal_is_unanswered() {
    let phone = Phone::new();
    phone.lose_signal();
    let (_, out) = fetch(&phone, now(), Vec::new());
    assert!(out.unanswered);
}

#[test]
fn a_partial_answer_is_not_unanswered() {
    let phone = Phone::new();
    phone.behind_portal_for(200, "tidesandcurrents");
    let (_, out) = fetch(&phone, now(), Vec::new());
    assert!(
        phone.requests() > 0
            && phone
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.url.contains("tidesandcurrents"))
    );
    assert!(!out.unanswered);
}

#[test]
fn answers_from_the_platforms_cache_are_not_answers() {
    // Signal lost: the weather service's point lookup comes from memory, everything else fails.
    let phone = Phone::new();
    phone.serve_from_cache("api.weather.gov/points");
    let (_, out) = fetch(&phone, now(), Vec::new());
    assert!(
        phone
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.url.contains("api.weather.gov/points")),
        "the cached request was made"
    );
    assert!(out.unanswered);
    assert!(out.cacheable.is_empty());
    assert!(out.metadata_stations.is_empty());

    // The same fetch with the network answering is an answer.
    let phone = Phone::new();
    let (_, out) = fetch(&phone, now(), Vec::new());
    assert!(!out.unanswered);
}

#[test]
fn the_fetch_names_the_stations_the_spot_needs_even_when_a_prediction_fails() {
    let phone = Phone::new();
    let (_, first) = fetch(&phone, now(), Vec::new());
    assert_eq!(first.metadata_stations, ["8518750", "NYH1927"]);

    // The current predictions fail: the current's constants are still wanted.
    let phone = Phone::new();
    phone.break_requests(&["interval=max_slack", "interval=6&"]);
    let refresh = FetchOptions {
        refresh_metadata: true,
        ..OPTIONS
    };
    let (_, out) = fetch_with(&phone, now(), Vec::new(), refresh);
    assert!(!out.unanswered);
    assert_eq!(out.metadata_stations, ["8518750", "NYH1927"]);

    // Without signal nothing is known: the app releases nothing.
    phone.break_requests(&[]);
    phone.lose_signal();
    let (_, out) = fetch(&phone, now() + 3_600, first.cacheable);
    assert!(out.unanswered);
    assert!(out.metadata_stations.is_empty());
}

fn discover(phone: &Arc<Phone>) -> underbite_ffi::fetch::DiscoveryOutcome {
    pollster::block_on(underbite_ffi::fetch::discover_stations(
        battery(),
        default_settings_json(),
        Keys::default(),
        true,
        now(),
        phone.host(),
    ))
    .expect("discovered")
}

#[test]
fn a_portal_answering_every_discovery_request_is_unanswered() {
    for status in [200, 302, 511] {
        let phone = Phone::new();
        phone.behind_portal(status);
        let out = discover(&phone);
        assert!(phone.requests() > 0, "{status}");
        assert!(out.unanswered, "{status}");
    }
}

#[test]
fn a_partly_answered_discovery_is_not_unanswered() {
    let phone = Phone::new();
    phone.behind_portal_for(200, "activestations");
    let out = discover(&phone);
    assert!(!out.unanswered);
    let phone = Phone::new();
    assert!(!discover(&phone).unanswered);
}

/// The keys of the constants, datums and offsets in `kept`.
fn constant_keys(kept: &[CachedResponse]) -> BTreeSet<String> {
    kept.iter()
        .filter(|c| c.key.contains("/harcon.json") || c.key.contains("/datums.json"))
        .map(|c| c.key.clone())
        .collect()
}

#[test]
fn kept_constants_are_asked_again_only_when_a_week_old_or_on_a_refresh() {
    let phone = Phone::new();
    let (_, first) = fetch(&phone, now(), Vec::new());
    let wanted = constant_keys(&first.cacheable);
    assert_eq!(wanted.len(), 3, "{wanted:?}");
    // They name the stations only: no position leaves with them.
    for request in phone.constants_asked(0) {
        let url = &request.url;
        assert!(url.starts_with(&format!("{MDAPI}/stations/")), "{url}");
        let query = url.split_once('?').map_or("", |(_, q)| q);
        assert!(query == "units=metric" || query == "bin=13", "{url}");
        assert!(!request.fresh, "{url}");
    }

    // A day later the copies are recent: the fetch does not ask for them again.
    let day_later = now() + 86_400;
    let before = phone.requests();
    let (_, out) = fetch(&phone, day_later, first.cacheable.clone());
    assert_eq!(phone.constants_asked(before), Vec::new());
    assert_eq!(constant_keys(&out.cacheable), BTreeSet::new());

    // A pull to refresh asks for them again, past any cache, and keeps the new copies.
    let refresh = FetchOptions {
        refresh_metadata: true,
        ..OPTIONS
    };
    let before = phone.requests();
    let (_, out) = fetch_with(&phone, day_later, first.cacheable.clone(), refresh);
    let asked = phone.constants_asked(before);
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert!(asked.iter().all(|r| r.fresh), "{asked:?}");
    assert_eq!(constant_keys(&out.cacheable), wanted);
    assert!(out.cacheable.iter().all(|c| c.fetched_at == day_later));

    // Copies more than a week old are asked for again.
    let old: Vec<CachedResponse> = first
        .cacheable
        .iter()
        .map(|c| CachedResponse {
            fetched_at: now() - 8 * 86_400,
            ..c.clone()
        })
        .collect();
    let before = phone.requests();
    let (_, out) = fetch(&phone, now(), old);
    assert_eq!(phone.constants_asked(before).len(), 3);
    assert_eq!(constant_keys(&out.cacheable), wanted);
}

// ---------------------------------------------------------------------------------------------
// Packs
// ---------------------------------------------------------------------------------------------

/// The harbour around the Battery: west, south, east, north.
const HARBOUR: [f64; 4] = [-74.10, 40.60, -73.95, 40.75];

fn pack_json(layers: &[&str], min_zoom: u8, max_zoom: u8, dates: &Value) -> String {
    pack_json_in(HARBOUR, layers, min_zoom, max_zoom, dates)
}

fn pack_json_in(
    [west, south, east, north]: [f64; 4],
    layers: &[&str],
    min_zoom: u8,
    max_zoom: u8,
    dates: &Value,
) -> String {
    serde_json::json!({
        "id": "harbour",
        "name": "Harbour",
        "bbox": { "west": west, "south": south, "east": east, "north": north },
        "min_zoom": min_zoom,
        "max_zoom": max_zoom,
        "layers": layers,
        "dates": dates,
        "spots": ["battery"],
        "created_at": now(),
    })
    .to_string()
}

fn settings(edit: impl FnOnce(&mut Value)) -> String {
    let mut json: Value = serde_json::from_str(&default_settings_json()).expect("json");
    edit(&mut json);
    json.to_string()
}

#[test]
fn a_plan_reports_counts_sizes_and_words_and_pages_through_its_requests() {
    let json = pack_json(
        &["noaa-chart", "sst", "depth-dem", "radar"],
        10,
        12,
        &serde_json::json!({ "last_days": 2 }),
    );
    let info = plan_pack(
        json,
        default_settings_json(),
        HashMap::default(),
        HashMap::default(),
        now(),
    )
    .expect("planned");
    assert!(info.tile_count > 0 && info.grid_count > 0 && info.dem_count > 0);
    assert_eq!(
        info.item_count,
        info.tile_count + info.grid_count + info.dem_count
    );
    assert_eq!(info.plan.item_count(), info.item_count);
    assert!(info.est_bytes > 0 && info.est_seconds > 0);
    assert!(
        info.size_text.ends_with("MB") || info.size_text.ends_with("KB"),
        "{}",
        info.size_text
    );
    assert_eq!(
        info.summary_text,
        format!("{}, about {}", info.size_text, info.time_text)
    );
    assert!(!info.over_budget && info.over_budget_text.is_none());
    assert_eq!(info.skipped.len(), 1);
    assert_eq!(info.skipped[0].layer, "radar");
    assert_eq!(info.skipped[0].reason, "Not kept offline");

    // Pages of 37 return every item once, in order, until one comes back empty.
    let mut keys = Vec::new();
    let mut kinds = BTreeSet::new();
    let mut from = 0u64;
    loop {
        let page = pack_requests(Arc::clone(&info.plan), from, 37);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 37);
        from += page.len() as u64;
        for item in page {
            assert_eq!(item.error, None, "{}", item.key);
            let url = &item.request.as_ref().expect("a request").url;
            assert!(url.starts_with("https://"), "{url}");
            assert!(!url.contains('{'), "{url}");
            assert_ne!(item.host.len(), 0);
            kinds.insert(format!("{:?}", item.kind));
            if item.kind == PackItemKind::Grid {
                assert!(url.contains("/griddap/"), "{url}");
            }
            keys.push(item.key);
        }
    }
    assert_eq!(keys.len() as u64, info.item_count);
    assert_eq!(
        keys.iter().collect::<BTreeSet<_>>().len(),
        keys.len(),
        "each once"
    );
    assert_eq!(
        kinds,
        BTreeSet::from(["Dem".into(), "Grid".into(), "Tile".into()])
    );
    // A page past the end is empty, and one page is never the whole plan.
    let past = pack_requests(Arc::clone(&info.plan), info.item_count + 5, 10);
    assert_eq!(past.len(), 0);
    let big = pack_requests(Arc::clone(&info.plan), 0, u32::MAX);
    assert!(big.len() <= 2_000);
}

fn fixture(path: &str) -> Vec<u8> {
    let path = fixtures().join("raw").join(path);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ok(content_type: &str, body: Vec<u8>) -> HostResponse {
    HostResponse {
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
    }
}

/// The numbers after the first `skip` parts of a slash-separated key.
fn key_numbers(key: &str, skip: usize) -> Vec<f64> {
    key.split('/')
        .skip(skip)
        .map(|n| n.parse().unwrap_or_else(|_| panic!("`{n}` in {key}")))
        .collect()
}

fn url_of(item: &PackRequest) -> &str {
    &item.request.as_ref().expect("a request").url
}

#[test]
fn pack_items_are_the_map_stores_keys_and_requests() {
    let json = pack_json(&["sst", "depth-dem"], 12, 12, &serde_json::json!("latest"));
    let info = plan_pack(
        json,
        default_settings_json(),
        HashMap::default(),
        HashMap::default(),
        now(),
    )
    .expect("planned");
    let page = pack_requests(Arc::clone(&info.plan), 0, 2_000);
    let grid = page
        .iter()
        .find(|r| r.kind == PackItemKind::Grid)
        .expect("a grid item");
    let dem = page
        .iter()
        .find(|r| r.kind == PackItemKind::Dem)
        .expect("a DEM item");

    // A view inside the block asks the map's store for the same key and request.
    let store = GridStore::new();
    let ask = |store: &GridStore, key: &str, time: Option<i64>| {
        let [south, west, _stride] = key_numbers(key, 3)[..] else {
            panic!("{key}");
        };
        store
            .grid_requests(
                "sst".into(),
                None,
                time,
                west + 0.001,
                south + 0.001,
                west + 0.002,
                south + 0.002,
                12.0,
                now(),
            )
            .expect("requests")
    };
    let asked = ask(&store, &grid.key, None);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].key, grid.key);
    assert_eq!(asked[0].request.url, url_of(grid));
    let mur = fixture("erddap/jplmursst41-sst-mask-subset-2026-11-17.json");
    store
        .add_grid(grid.key.clone(), ok("application/json", mur.clone()), now())
        .expect("the store takes the pack's key");
    assert_eq!(ask(&store, &grid.key, None), [], "the map has the block");

    // An older day is asked at the time the layer's axis lists, as the map's date control
    // asks for it, and the newest as `(last)`.
    let time = |t: &str| parse_rfc3339(t).expect("time").0;
    let older = time("2026-11-06T09:00:00Z");
    let axes = HashMap::from([("sst".to_owned(), vec![older, time("2026-11-07T09:00:00Z")])]);
    let json = pack_json(&["sst"], 12, 12, &serde_json::json!({ "last_days": 2 }));
    let info = plan_pack(
        json,
        default_settings_json(),
        HashMap::default(),
        axes,
        now(),
    )
    .expect("planned");
    let page = pack_requests(Arc::clone(&info.plan), 0, 2_000);
    let (dated, newest): (Vec<_>, Vec<_>) = page
        .iter()
        .partition(|r| r.key.starts_with(&format!("sst/jplMURSST41/{older}/")));
    assert_eq!(dated.len(), newest.len());
    assert!(newest.iter().all(|r| r.key.contains("/last/")));
    let asked = ask(&store, &dated[0].key, Some(older));
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].key, dated[0].key);
    assert_eq!(asked[0].request.url, url_of(dated[0]));
    store
        .add_grid(dated[0].key.clone(), ok("application/json", mur), now())
        .expect("the store takes the pack's key");
    assert_eq!(ask(&store, &dated[0].key, Some(older)), []);

    // The elevation area is the map's area at the pack's deepest zoom.
    let [w, s, e, n, _cell] = key_numbers(&dem.key, 2)[..] else {
        panic!("{}", dem.key);
    };
    let edge = |micro: f64| micro / 1e6;
    let area = |store: &GridStore| {
        store
            .dem_area_request(edge(w), edge(s), edge(e), edge(n), 12.0)
            .expect("an area request")
    };
    // The map asks for a view rounded outward to hundredths of a degree, so its key and URL are
    // the pack's only for an area already on that grid.
    let asked = area(&store).expect("not held yet");
    let tiff = fixture("bathymetry/dem-global-mosaic-export-f32.tif");
    store
        .add_dem(dem.key.clone(), ok("image/tiff", tiff.clone()), now())
        .expect("the store takes the pack's key");
    store
        .add_dem(asked.key.clone(), ok("image/tiff", tiff), now())
        .expect("the store takes the map's key");
    assert_eq!(area(&store), None, "the map has the area");
}

#[test]
fn a_plan_over_the_offline_limit_says_so_and_a_huge_one_is_refused() {
    let json = pack_json(&["noaa-chart"], 8, 14, &serde_json::json!("latest"));
    let tight = settings(|s| s["map"]["offline_max_mb"] = 1.into());
    let info = plan_pack(
        json.clone(),
        tight,
        HashMap::default(),
        HashMap::default(),
        now(),
    )
    .expect("planned");
    assert!(info.over_budget);
    let text = info.over_budget_text.expect("text");
    assert!(
        text.ends_with("more than your 1.0 MB offline limit."),
        "{text}"
    );

    // Measured sizes replace the seeds.
    let measured = [("noaa-chart".to_owned(), 1_000u64)].into_iter().collect();
    let small = plan_pack(
        json,
        default_settings_json(),
        measured,
        HashMap::default(),
        now(),
    )
    .expect("planned");
    assert_eq!(small.est_bytes, small.item_count * 1_000);

    // The same zooms over the harbour fit; over most of North America they do not.
    let latest = serde_json::json!("latest");
    let harbour = pack_json(&["noaa-chart"], 8, 12, &latest);
    plan_pack(
        harbour,
        default_settings_json(),
        HashMap::default(),
        HashMap::default(),
        now(),
    )
    .expect("planned");
    let world = pack_json_in([-120.0, 20.0, -60.0, 60.0], &["noaa-chart"], 8, 12, &latest);
    let err = plan_pack(
        world,
        default_settings_json(),
        HashMap::default(),
        HashMap::default(),
        now(),
    )
    .expect_err("too many items");
    assert!(
        err.to_string().starts_with("This pack needs more than"),
        "{err}"
    );
    let bad = plan_pack(
        "{".into(),
        default_settings_json(),
        HashMap::default(),
        HashMap::default(),
        now(),
    );
    assert!(bad.is_err());
}

// ---------------------------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------------------------

#[test]
fn the_storage_page_words_what_is_kept_against_the_limit() {
    let view = storage_view(412_000_000, 300_000_000, 1_500, default_settings_json());
    assert_eq!(view.total_text, "412 MB of 1.5 GB");
    assert_eq!(view.packs_text, "Packs: 300 MB");
    assert_eq!(view.cache_text, "Map cache: 112 MB");
    assert_eq!(view.limit_text, "Limit: 1.5 GB");

    // A limit of 0 keeps nothing but packs.
    let off = storage_view(300_000_000, 300_000_000, 0, default_settings_json());
    assert_eq!(off.total_text, "300 MB used");
    assert_eq!(off.cache_text, "Map cache: 0 KB");
    assert_eq!(off.limit_text, "No map cache: only packs are kept");

    // Counts taken while the store changes never make a negative cache.
    let odd = storage_view(10_000, 20_000, 500, default_settings_json());
    assert_eq!(odd.cache_text, "Map cache: 0 KB");
    assert_eq!(odd.total_text, "10 KB of 500 MB");

    // Pack rows read the same way; unreadable settings still give words.
    assert_eq!(
        storage_size_text(48_260_000, default_settings_json()),
        "48.3 MB"
    );
    assert_eq!(storage_size_text(1_540_000_000, "{".into()), "1.5 GB");
    assert_eq!(
        storage_view(412_000_000, 0, 1_500, "{".into()).total_text,
        "412 MB of 1.5 GB"
    );
}

// ---------------------------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------------------------

#[test]
fn the_battery_tide_table_lists_a_week_of_highs_and_lows() {
    let view = tide_table_view(
        "8518750".into(),
        raw("coops-mdapi-8518750-harcon-metric.json"),
        raw("coops-mdapi-8518750-datums-metric.json"),
        None,
        default_settings_json(),
        "UTC".into(),
        "2026-11-08".into(),
        7,
    )
    .expect("a table");
    assert_eq!(
        view.rows.len(),
        27,
        "{:?}",
        view.rows.iter().map(|r| &r.date_text).collect::<Vec<_>>()
    );
    assert_eq!(view.datum_text.as_deref(), Some("above MLLW"));
    assert_eq!(view.note, "Computed on device");
    assert_eq!(view.credit, "NOAA CO-OPS harmonics");
    assert_eq!(view.notice, None);
    let first = &view.rows[0];
    assert_eq!(first.date_text, "Nov 8");
    assert!(first.kind_text == "High" || first.kind_text == "Low");
    assert!(
        first.value_text.as_deref().unwrap().ends_with("ft MLLW"),
        "{first:?}"
    );
    // Highs and lows alternate.
    for pair in view.rows.windows(2) {
        assert_ne!(pair[0].kind_text, pair[1].kind_text);
        assert!(pair[0].time < pair[1].time);
    }
}

#[test]
fn a_subordinate_station_has_highs_and_lows_from_its_reference() {
    let view = tide_table_view(
        "8518919".into(),
        raw("coops-mdapi-8518750-harcon-metric.json"),
        raw("coops-mdapi-8518750-datums-metric.json"),
        Some(raw("coops-mdapi-8518919-tidepredoffsets.json")),
        default_settings_json(),
        "America/New_York".into(),
        "2026-11-08".into(),
        1,
    )
    .expect("a table");
    assert!(view.rows.len() >= 3);
    assert!(
        view.rows
            .iter()
            .all(|r| r.kind_text == "High" || r.kind_text == "Low")
    );
}

#[test]
fn the_current_table_names_floods_ebbs_and_slacks_and_the_notice() {
    let view = current_table_view(
        "NYH1927".into(),
        Some(13),
        raw("coops-mdapi-NYH1927-harcon.json"),
        None,
        default_settings_json(),
        "UTC".into(),
        "2026-11-08".into(),
        1,
    )
    .expect("a table");
    let kinds: BTreeSet<&str> = view.rows.iter().map(|r| r.kind_text.as_str()).collect();
    assert!(
        kinds.contains("Max flood") && kinds.contains("Max ebb") && kinds.contains("Slack"),
        "{kinds:?}"
    );
    let slack = view.rows.iter().find(|r| r.kind_text == "Slack").unwrap();
    assert_eq!(slack.value_text, None);
    let flood = view
        .rows
        .iter()
        .find(|r| r.kind_text == "Max flood")
        .unwrap();
    assert!(
        flood.value_text.as_deref().unwrap().ends_with("kn"),
        "{flood:?}"
    );
    assert_eq!(view.datum_text, None);
    assert_eq!(view.notice.as_deref(), Some("Not for navigation."));
}

#[test]
fn tables_refuse_bad_input_in_words() {
    let bad = tide_table_view(
        "8518750".into(),
        "{}".into(),
        "{}".into(),
        None,
        default_settings_json(),
        "UTC".into(),
        "2026-11-08".into(),
        1,
    )
    .expect_err("no constants");
    assert!(bad.to_string().contains("could be computed"), "{bad}");
    let harcon = raw("coops-mdapi-8518750-harcon-metric.json");
    let datums = raw("coops-mdapi-8518750-datums-metric.json");
    for (date, days) in [("2026-11-08", 0), ("2026-11-08", 400), ("someday", 1)] {
        assert!(
            tide_table_view(
                "8518750".into(),
                harcon.clone(),
                datums.clone(),
                None,
                default_settings_json(),
                "UTC".into(),
                date.into(),
                days,
            )
            .is_err(),
            "{date} {days}"
        );
    }
}
