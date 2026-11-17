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
    PackItemKind, current_table_view, pack_requests, plan_pack, tide_table_view,
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
    seen: Mutex<Vec<String>>,
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

    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl FetchHost for Phone {
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError> {
        self.seen.lock().unwrap().push(request.url.clone());
        if self.offline.load(Ordering::SeqCst) {
            return Err(HostError::Failed {
                message: "The Internet connection appears to be offline.".into(),
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
) -> (SpotSnapshot, underbite_ffi::fetch::SpotFetchOutcome) {
    let out = pollster::block_on(fetch_spot(
        battery(),
        default_settings_json(),
        Keys::default(),
        OPTIONS,
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
    // The constants and datums of the Battery were fetched earlier by the app.
    let cached = with_constants(&first.cacheable);
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

    // What was shown is kept as the last snapshot: a second trip offline still has it.
    let (again, _) = fetch(&phone, later + 3_600, cached);
    let water = again
        .water
        .temperature_c
        .as_ref()
        .expect("still the last temperature");
    assert_eq!(water.samples().last().expect("a sample").time, observed);
    assert!(again.marine.tide.is_some());
}

const BATTERY_MDAPI: &str =
    "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750";

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
}

// ---------------------------------------------------------------------------------------------
// Packs
// ---------------------------------------------------------------------------------------------

fn pack_json(layers: &[&str], min_zoom: u8, max_zoom: u8, dates: &Value) -> String {
    serde_json::json!({
        "id": "harbour",
        "name": "Harbour",
        "bbox": { "west": -74.10, "south": 40.60, "east": -73.95, "north": 40.75 },
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
        let page = pack_requests(Arc::clone(&info.plan), from, 37).expect("page");
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 37);
        from += page.len() as u64;
        for item in page {
            assert!(
                item.request.url.starts_with("https://"),
                "{}",
                item.request.url
            );
            assert!(!item.request.url.contains('{'), "{}", item.request.url);
            assert_ne!(item.host.len(), 0);
            kinds.insert(format!("{:?}", item.kind));
            if item.kind == PackItemKind::Grid {
                assert!(
                    item.request.url.contains("/griddap/"),
                    "{}",
                    item.request.url
                );
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
    let past = pack_requests(Arc::clone(&info.plan), info.item_count + 5, 10).unwrap();
    assert_eq!(past.len(), 0);
    let big = pack_requests(Arc::clone(&info.plan), 0, u32::MAX).unwrap();
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
    }
}

/// The numbers after the first `skip` parts of a slash-separated key.
fn key_numbers(key: &str, skip: usize) -> Vec<f64> {
    key.split('/')
        .skip(skip)
        .map(|n| n.parse().unwrap_or_else(|_| panic!("`{n}` in {key}")))
        .collect()
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
    let page = pack_requests(Arc::clone(&info.plan), 0, 2_000).expect("page");
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
    assert_eq!(asked[0].request.url, grid.request.url);
    let mur = fixture("erddap/jplmursst41-sst-subset-2026-11-07.json");
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
    let page = pack_requests(Arc::clone(&info.plan), 0, 2_000).expect("page");
    let (dated, newest): (Vec<_>, Vec<_>) = page
        .iter()
        .partition(|r| r.key.starts_with(&format!("sst/jplMURSST41/{older}/")));
    assert_eq!(dated.len(), newest.len());
    assert!(newest.iter().all(|r| r.key.contains("/last/")));
    let asked = ask(&store, &dated[0].key, Some(older));
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].key, dated[0].key);
    assert_eq!(asked[0].request.url, dated[0].request.url);
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
    let asked = area(&store).expect("not held yet");
    assert_eq!(asked.key, dem.key);
    assert_eq!(asked.request.url, dem.request.url);
    let tiff = fixture("bathymetry/dem-global-mosaic-export-f32.tif");
    store
        .add_dem(dem.key.clone(), ok("image/tiff", tiff), now())
        .expect("the store takes the pack's key");
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

    let world = pack_json(&["noaa-chart"], 0, 20, &serde_json::json!("latest")).replace(
        "\"west\":-74.1,\"south\":40.6,\"east\":-73.95,\"north\":40.75",
        "\"west\":-120.0,\"south\":20.0,\"east\":-60.0,\"north\":60.0",
    );
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
