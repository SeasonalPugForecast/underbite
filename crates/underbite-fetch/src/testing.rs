//! Test support: a host in memory that answers from the recorded replay scenarios, and the
//! places and time the tests use.

use std::collections::BTreeMap;
use std::future::ready;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use serde::de::DeserializeOwned;
use underbite_core::model::{Spot, WaterType};
use underbite_core::time::{UnixTime, parse_rfc3339};
use underbite_providers::source::FetchContext;

use crate::host::{Call, FetchHost, Fetched, TransportError};
use crate::store::{self, Stored};

pub(crate) fn now() -> UnixTime {
    parse_rfc3339("2026-11-08T14:30:00Z").expect("time")
}

pub(crate) fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

pub(crate) fn ctx() -> FetchContext {
    FetchContext::new(now(), "underbite/test")
}

pub(crate) fn battery() -> Spot {
    Spot {
        id: "battery".into(),
        name: "The Battery".into(),
        latitude: 40.7033,
        longitude: -74.0170,
        water_type: WaterType::TidalRiverEstuary,
        water_body: None,
        time_zone: "America/New_York".into(),
    }
}

pub(crate) fn croton() -> Spot {
    Spot {
        id: "croton".into(),
        name: "Croton River below the dam".into(),
        latitude: 41.2252,
        longitude: -73.8588,
        water_type: WaterType::RiverStream,
        water_body: None,
        time_zone: "America/New_York".into(),
    }
}

/// Fails unless every coordinate `url` sends has at most 2 decimals. A 3DHP query sends an
/// envelope or a point around the rounded spot: its centre is checked.
pub(crate) fn assert_rounded(url: &str) {
    let on_grid = |v: f64| ((v * 100.0).round() - v * 100.0).abs() < 1e-6;
    if url.contains("3DHP_all") {
        let geometry = url
            .split_once('?')
            .and_then(|(_, q)| q.split('&').find_map(|p| p.strip_prefix("geometry=")))
            .expect("geometry");
        let n: Vec<f64> = geometry
            .split(',')
            .map(|v| v.parse().expect("number"))
            .collect();
        let centre = match n[..] {
            [x0, y0, x1, y1] => [f64::midpoint(x0, x1), f64::midpoint(y0, y1)],
            [x, y] => [x, y],
            _ => panic!("{url}"),
        };
        assert!(centre.into_iter().all(on_grid), "{url}");
        return;
    }
    for token in url.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')) {
        if let Some((_, fraction)) = token.split_once('.') {
            let digits = fraction.trim_end_matches('0');
            assert!(
                digits.len() <= 2 || !digits.bytes().all(|b| b.is_ascii_digit()),
                "{url}: {token}"
            );
        }
    }
}

#[test]
fn rounding_checks_catch_precise_coordinates() {
    assert_rounded("https://api.example/x?latitude=40.7&longitude=-74.02");
    assert!(
        std::panic::catch_unwind(|| assert_rounded("https://a.example/points/40.7033,-74.017"))
            .is_err()
    );
}

/// One route of a replay scenario's `routes.toml`: the first route whose `match` the URL
/// contains answers with the recorded `file`.
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

/// A host in memory. It answers exact URLs given with [`MemoryHost::answering`], then the
/// routes of a replay scenario; any other URL fails with its redacted URL and is listed in
/// [`MemoryHost::missed`], so a test cannot pass quietly without a source. Values are kept in
/// memory unless the host is [`MemoryHost::forgetful`]; the ledger is saved as a value. It has
/// no response cache: every call it answers is recorded in [`MemoryHost::seen`].
#[derive(Default)]
pub(crate) struct MemoryHost {
    pages: BTreeMap<String, String>,
    base: PathBuf,
    routes: Vec<Route>,
    forgetful: bool,
    failing_usage: bool,
    values: Mutex<BTreeMap<(String, String), String>>,
    seen: Mutex<Vec<String>>,
    missed: Mutex<Vec<String>>,
}

impl MemoryHost {
    /// A host answering from `tests/fixtures/replay/<scenario>`.
    pub(crate) fn replay(scenario: &str) -> Self {
        let base = fixtures().join("replay").join(scenario);
        let text = std::fs::read_to_string(base.join("routes.toml")).expect("routes");
        let routes: Routes = toml::from_str(&text).expect("routes file");
        Self {
            base,
            routes: routes.route,
            ..Self::default()
        }
    }

    /// The same host answering `url` exactly with `body`.
    pub(crate) fn answering(mut self, url: impl Into<String>, body: impl Into<String>) -> Self {
        self.pages.insert(url.into(), body.into());
        self
    }

    /// The same host keeping no values (and saving the ledger nowhere).
    pub(crate) fn forgetful(mut self) -> Self {
        self.forgetful = true;
        self
    }

    /// The same host failing to save the ledger.
    pub(crate) fn failing_usage(mut self) -> Self {
        self.failing_usage = true;
        self
    }

    /// Every URL asked for, redacted, in order.
    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("lock").clone()
    }

    /// Every URL asked for that had no answer, redacted, in order.
    pub(crate) fn missed(&self) -> Vec<String> {
        self.missed.lock().expect("lock").clone()
    }

    /// The text stored under `namespace` and `key`.
    pub(crate) fn value(&self, namespace: &str, key: &str) -> Option<String> {
        let values = self.values.lock().expect("lock");
        values.get(&(namespace.into(), key.into())).cloned()
    }

    /// The value stored under `namespace` and `key`.
    pub(crate) fn stored<T: DeserializeOwned>(&self, namespace: &str, key: &str) -> Option<T> {
        let text = self.value(namespace, key)?;
        let stored: Stored<T> = serde_json::from_str(&text).expect("stored value");
        Some(stored.value)
    }

    /// Stores `value` under `namespace` and `key`, as a fetch at [`now`] would.
    pub(crate) fn put<T: Serialize>(&self, namespace: &str, key: &str, value: &T) {
        let text = serde_json::to_string(&Stored {
            saved_at: now().0,
            value,
        })
        .expect("json");
        let mut values = self.values.lock().expect("lock");
        values.insert((namespace.into(), key.into()), text);
    }

    fn answer(&self, call: &Call) -> Option<Fetched> {
        if let Some(body) = self.pages.get(&call.url) {
            return Some(Fetched::new(200, body.clone()));
        }
        let route = self.routes.iter().find(|r| call.url.contains(&r.matches))?;
        let path = self.base.join(&route.file);
        let body = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut fetched = Fetched::new(route.status.unwrap_or(200), body);
        fetched.content_type = Some(
            route
                .content_type
                .clone()
                .unwrap_or_else(|| content_type(&route.file).into()),
        );
        Some(fetched)
    }
}

/// The content type of a recorded file without one in its route, by extension.
fn content_type(file: &Path) -> &'static str {
    let name = file.to_string_lossy();
    let name = name.strip_suffix(".gz").unwrap_or(&name);
    match name.rsplit('.').next().unwrap_or_default() {
        "json" => "application/json",
        "xml" => "text/xml",
        "html" => "text/html",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "tif" => "image/tiff",
        "zip" => "application/zip",
        "txt" | "spec" | "ocean" | "adcp" | "dat" | "das" => "text/plain",
        _ => "application/octet-stream",
    }
}

impl MemoryHost {
    fn exchange(&self, call: &Call) -> Result<Fetched, TransportError> {
        let redacted = call.redacted_url();
        self.seen.lock().expect("lock").push(redacted.clone());
        self.answer(call).ok_or_else(|| {
            self.missed.lock().expect("lock").push(redacted.clone());
            TransportError::NoRoute(redacted)
        })
    }

    fn keep(&self, namespace: &str, key: &str, value: String) {
        if !self.forgetful {
            let mut values = self.values.lock().expect("lock");
            values.insert((namespace.into(), key.into()), value);
        }
    }

    fn keep_usage(&self, ledger: String) -> Result<(), String> {
        if self.failing_usage {
            return Err("the disk is full".into());
        }
        self.keep(store::USAGE, store::LEDGER_KEY, ledger);
        Ok(())
    }
}

impl FetchHost for MemoryHost {
    fn send(&self, call: &Call) -> impl Future<Output = Result<Fetched, TransportError>> + Send {
        ready(self.exchange(call))
    }

    fn load(&self, namespace: &str, key: &str) -> impl Future<Output = Option<String>> + Send {
        ready(self.value(namespace, key))
    }

    fn store(&self, namespace: &str, key: &str, value: String) -> impl Future<Output = ()> + Send {
        self.keep(namespace, key, value);
        ready(())
    }

    fn save_usage(&self, ledger: String) -> impl Future<Output = Result<(), String>> + Send {
        ready(self.keep_usage(ledger))
    }
}
