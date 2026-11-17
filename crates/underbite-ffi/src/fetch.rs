//! Fetching a spot through the host the app provides.
//!
//! The fetch is `underbite-fetch`'s, the one the command line runs: discovery, the water and
//! marine session, whether's weather session, the normals, the storm watch and the sky. This
//! module only adapts the app's [`FetchHost`] (`URLSession`, a small store of values kept
//! between fetches, the call ledger) to it and hands the results back as JSON.
//!
//! Coordinates leave the phone only as the fetch rounds them: to the configured
//! `sources.coordinate_precision`, at most 2 decimals (about 1 km). The exact spot stays on the
//! device. Keys arrive with each call in [`Keys`]; they are never stored, serialized or shown by
//! `Debug`, and a request that carries one is marked so the app never writes its response to a
//! disk cache.

use std::collections::BTreeMap;
use std::fmt;
use std::future::ready;
use std::sync::Arc;

use underbite_config::schema::Config;
use underbite_core::discover::rank::Ranking;
use underbite_core::model::{SourceError, Spot, WaterType};
use underbite_core::time::UnixTime;
use underbite_core::view::strings::Strings;
use underbite_fetch::store::{self, Choice, parse_station_key};
use underbite_fetch::{self as library, Call, Fetched, TransportError};
use underbite_providers::request::{is_secret_header, redact_url, secret_values};
use underbite_providers::source::Secrets;
use whether_core::time::parse_http_date;
use whether_providers::Method;

use crate::settings::{SettingsError, load};
use crate::view::SpotInfo;

/// The largest response body a fetch accepts, decompressed: 8 MB, as the command line's
/// (the CO-OPS current station list is 3.8 MB). The app stops reading past it.
pub const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

/// [`MAX_BODY_BYTES`], for the app's `URLSession` host.
#[uniffi::export]
pub fn max_body_bytes() -> u64 {
    MAX_BODY_BYTES
}

// ---------------------------------------------------------------------------------------------
// What the app sends and receives
// ---------------------------------------------------------------------------------------------

/// One request header.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct Header {
    /// Name.
    pub name: String,
    /// Value; the key itself for a secret header such as `X-Api-Key`.
    pub value: String,
}

/// Masks the value of a secret header.
impl fmt::Debug for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = if is_secret_header(&self.name) {
            "***"
        } else {
            self.value.as_str()
        };
        f.debug_tuple("Header")
            .field(&self.name)
            .field(&value)
            .finish()
    }
}

/// The HTTP method of a [`HostRequest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum HostMethod {
    /// GET, without a body.
    Get,
    /// POST with a JSON body (`Content-Type: application/json`).
    Post,
}

/// A request for the app to perform. Never log its URL or headers: show it only through
/// `Debug`, which redacts both.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostRequest {
    /// The full URL.
    pub url: String,
    /// Extra headers.
    pub headers: Vec<Header>,
    /// GET or POST.
    pub method: HostMethod,
    /// The JSON body of a POST.
    pub body: Option<String>,
    /// Fetch a fresh copy, past any memory or disk cache (time-critical files).
    pub fresh: bool,
    /// Never write the response to a disk cache: always set when the request carries a key.
    pub no_store: bool,
    /// How long the response may be reused when the server sends no cache headers, seconds.
    pub max_age_s: Option<u32>,
}

impl fmt::Debug for HostRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostRequest")
            .field("url", &redact_url(&self.url))
            .field("headers", &self.headers)
            .field("method", &self.method)
            .field("body_len", &self.body.as_ref().map(String::len))
            .field("fresh", &self.fresh)
            .field("no_store", &self.no_store)
            .field("max_age_s", &self.max_age_s)
            .finish()
    }
}

/// The response to a [`HostRequest`]: any status, with its body and the headers the fetch
/// reads. Header values are passed as received, except the two dates.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct HostResponse {
    /// Status code.
    pub status: u16,
    /// The body, decompressed; at most [`MAX_BODY_BYTES`].
    pub body: Vec<u8>,
    /// The `Content-Type` header (some tile services answer errors with status 200 and a
    /// text or XML body).
    pub content_type: Option<String>,
    /// The `Date` header, as Unix seconds.
    pub date: Option<i64>,
    /// The `Expires` header, as Unix seconds.
    pub expires: Option<i64>,
    /// The `Last-Modified` header.
    pub last_modified: Option<String>,
    /// The `ETag` header.
    pub etag: Option<String>,
    /// The `Retry-After` header: seconds or an HTTP date.
    pub retry_after: Option<String>,
    /// The `X-RateLimit-Remaining` header.
    pub rate_limit_remaining: Option<String>,
}

impl fmt::Debug for HostResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostResponse")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .field("content_type", &self.content_type)
            .field("date", &self.date)
            .finish_non_exhaustive()
    }
}

/// Why the app could not do what the fetch asked.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum HostError {
    /// The request or the save failed (no network, TLS, disk full).
    #[error("{message}")]
    Failed {
        /// What went wrong.
        message: String,
    },
    /// The request timed out.
    #[error("timed out: {message}")]
    TimedOut {
        /// Which limit was reached.
        message: String,
    },
    /// The body was larger than [`MAX_BODY_BYTES`].
    #[error("response too large")]
    TooLarge,
}

impl From<uniffi::UnexpectedUniFFICallbackError> for HostError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed { message: e.reason }
    }
}

/// What the app provides to a fetch: HTTP, a store for the values a fetch keeps between
/// fetches, and somewhere durable for the call ledger.
///
/// The fetch calls the host one request at a time. Stored values are opaque JSON text under a
/// namespace and a key (letters, digits, `-`, `_` and `.`; never coordinates or keys): station
/// lists, rankings, pins and exclusions, observed coverage, the last snapshot per spot. The
/// app may drop any of them (they are fetched again) except the call ledger.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait FetchHost: Send + Sync {
    /// Performs the request (`URLSession`): a GET, or a POST with its JSON body. Any status
    /// is a response; an error is for no response at all. Honour the request's cache flags:
    /// no cached answer when `fresh`, nothing written to disk when `no_store`.
    async fn send(&self, request: HostRequest) -> Result<HostResponse, HostError>;

    /// The value last stored under `namespace` and `key`, or the ledger last saved through
    /// [`FetchHost::save_usage`] for `"usage"` and `"ledger"`.
    fn load(&self, namespace: String, key: String) -> Option<String>;

    /// Stores `value` under `namespace` and `key`. A value that cannot be stored is only
    /// fetched again next time. An empty value is a forgotten one (see [`forget_spot`]): the
    /// host may delete what it holds under the key instead.
    fn store(&self, namespace: String, key: String, value: String);

    /// Stores the call ledger (JSON) durably, to be returned by [`FetchHost::load`] for
    /// `"usage"` and `"ledger"`. Called before every metered request, which is sent only if
    /// this succeeds.
    fn save_usage(&self, ledger_json: String) -> Result<(), HostError>;
}

/// The app's [`FetchHost`] as the fetch library's.
struct Bridge(Arc<dyn FetchHost>);

impl library::FetchHost for Bridge {
    async fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        match self.0.send(host_request(call)).await {
            Ok(response) => fetched(response),
            Err(e) => Err(transport_error(e, &call.url)),
        }
    }

    // The store and the ledger are plain calls on the app's side: each is done when it returns.

    fn load(&self, namespace: &str, key: &str) -> impl Future<Output = Option<String>> + Send {
        ready(self.0.load(namespace.to_owned(), key.to_owned()))
    }

    fn store(&self, namespace: &str, key: &str, value: String) -> impl Future<Output = ()> + Send {
        self.0.store(namespace.to_owned(), key.to_owned(), value);
        ready(())
    }

    fn save_usage(&self, ledger: String) -> impl Future<Output = Result<(), String>> + Send {
        ready(self.0.save_usage(ledger).map_err(|e| e.to_string()))
    }
}

/// The call as the app sees it.
pub(crate) fn host_request(call: &Call) -> HostRequest {
    HostRequest {
        url: call.url.clone(),
        headers: call
            .headers
            .iter()
            .map(|(name, value)| Header {
                name: (*name).to_owned(),
                value: value.clone(),
            })
            .collect(),
        method: match call.method {
            Method::Get => HostMethod::Get,
            Method::Post => HostMethod::Post,
        },
        body: call.body.clone(),
        fresh: call.cache.fresh,
        no_store: !call.may_store_on_disk(),
        max_age_s: call.cache.max_age,
    }
}

/// The app's response as the fetch reads it; a body over [`MAX_BODY_BYTES`] is refused.
pub(crate) fn fetched(response: HostResponse) -> Result<Fetched, TransportError> {
    if response.body.len() as u64 > MAX_BODY_BYTES {
        return Err(TransportError::TooLarge(MAX_BODY_BYTES));
    }
    let date = response.date.map(UnixTime);
    let header =
        |value: Option<String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    Ok(Fetched {
        status: response.status,
        content_type: header(response.content_type),
        date,
        expires: response.expires.map(UnixTime),
        cache_max_age: None,
        last_modified: header(response.last_modified),
        etag: header(response.etag),
        retry_after: header(response.retry_after).and_then(|v| retry_after(&v, date)),
        rate_limit_remaining: header(response.rate_limit_remaining).and_then(|v| v.parse().ok()),
        body: response.body,
    })
}

/// `Retry-After` as seconds: a number, or a date relative to the response's `Date`.
fn retry_after(value: &str, date: Option<UnixTime>) -> Option<u32> {
    if let Ok(seconds) = value.parse::<u32>() {
        return Some(seconds);
    }
    let until = parse_http_date(value)?;
    u32::try_from((until.0 - date?.0).max(0)).ok()
}

/// The app's error for the call to `url`, with the URL and any secret in it masked (a
/// platform message may quote the URL).
fn transport_error(e: HostError, url: &str) -> TransportError {
    match e {
        HostError::Failed { message } => TransportError::Failed(scrub(&message, url)),
        HostError::TimedOut { message } => TransportError::Timeout(scrub(&message, url)),
        HostError::TooLarge => TransportError::TooLarge(MAX_BODY_BYTES),
    }
}

/// `message` with `url` redacted and every secret value of it masked.
fn scrub(message: &str, url: &str) -> String {
    let mut out = message.replace(url, &redact_url(url));
    for secret in secret_values(url) {
        out = out.replace(secret, "***");
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Keys, the User-Agent and the version
// ---------------------------------------------------------------------------------------------

/// The keys the app keeps in the keychain, passed with each call and never stored, serialized
/// or shown by `Debug`. An empty key counts as none.
#[derive(Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct Keys {
    /// The USGS Water Data API key (`api.data.gov`): sent only as the `X-Api-Key` header.
    pub usgs: Option<String>,
    /// A Copernicus Marine user name.
    pub copernicus_user: Option<String>,
    /// A Copernicus Marine password.
    pub copernicus_password: Option<String>,
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = |key: &Option<String>| key.as_ref().map(|_| "***");
        f.debug_struct("Keys")
            .field("usgs", &set(&self.usgs))
            .field("copernicus_user", &set(&self.copernicus_user))
            .field("copernicus_password", &set(&self.copernicus_password))
            .finish()
    }
}

impl Keys {
    /// The secrets the sources may send. No source uses the Copernicus account yet.
    fn secrets(&self) -> Secrets {
        Secrets {
            usgs_api_key: self.usgs.clone().filter(|k| !k.trim().is_empty()),
        }
    }
}

/// The core's version, for the About screen.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The User-Agent the data services see: `underbite/<version> (iOS; +<contact>)`, without the
/// `+` part while the project has no contact URL.
#[uniffi::export]
pub fn user_agent() -> String {
    user_agent_with(contact_from(env!("CARGO_PKG_REPOSITORY")).as_deref())
}

/// `url` as a contact, unless it is empty or a `.invalid` placeholder.
fn contact_from(url: &str) -> Option<String> {
    let url = url.trim();
    (!url.is_empty() && !url.contains(".invalid")).then(|| url.to_owned())
}

fn user_agent_with(contact: Option<&str>) -> String {
    let version = env!("CARGO_PKG_VERSION");
    match contact {
        Some(contact) => format!("underbite/{version} (iOS; +{contact})"),
        None => format!("underbite/{version} (iOS)"),
    }
}

// ---------------------------------------------------------------------------------------------
// The fetch and discovery
// ---------------------------------------------------------------------------------------------

/// Why a fetch could not start. Sources that fail do not fail a fetch: they are listed in its
/// outcome's errors.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FetchError {
    /// The settings could not be read.
    #[error("settings: {message}")]
    Settings {
        /// Why.
        message: String,
    },
    /// The spot or a result could not be read or written.
    #[error("invalid {what}: {message}")]
    Invalid {
        /// What was being read or written.
        what: String,
        /// The parser's message.
        message: String,
    },
}

impl From<SettingsError> for FetchError {
    fn from(e: SettingsError) -> Self {
        let SettingsError::Invalid { message } = e;
        Self::Settings { message }
    }
}

fn invalid(what: &str, message: &impl ToString) -> FetchError {
    FetchError::Invalid {
        what: what.to_owned(),
        message: message.to_string(),
    }
}

/// How a fetch runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FetchOptions {
    /// Whether metered calls (USGS, Open-Meteo Marine, the storm point) may be spent. A
    /// refresh that was cancelled passes `false` for the fetches still waiting.
    pub allow_metered: bool,
    /// Whether the spot's water type is inferred (from its marine zone, the mapped streams and
    /// waterbodies and the nearest USGS site) before planning; the spot's own type is used when
    /// nothing points anywhere.
    pub infer_water_type: bool,
}

/// A source that failed or was passed over, as the snapshot lists it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SourceIssue {
    /// The source's id (`"coops"`, `"usgs"`, ...).
    pub source: String,
    /// The measurement concerned (`"water_temperature"`, ...), if one.
    pub measurement: Option<String>,
    /// What happened: `rate_limited`, `not_found`, `no_data`, `stale`, `parse`, `transport`
    /// or `other`.
    pub kind: String,
    /// The source's message, without keys or exact coordinates.
    pub message: String,
}

impl From<&SourceError> for SourceIssue {
    fn from(e: &SourceError) -> Self {
        Self {
            source: e.source.as_str().to_owned(),
            measurement: e.measurement.as_ref().map(name),
            kind: name(&e.kind),
            message: e.message.clone(),
        }
    }
}

/// A water type the fetch inferred for the spot.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct WaterTypeSuggestion {
    /// The water type, as `SpotInfo::water_type` writes it.
    pub water_type: String,
    /// Why, in words the core writes in the settings' voice: "in an NWS coastal marine zone",
    /// "on Croton River (stream of order 7)", ...
    pub reasons: Vec<String>,
}

/// A spot's fetch.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SpotFetchOutcome {
    /// The snapshot as JSON: keep it and pass it to `view_model`.
    pub snapshot_json: String,
    /// The sources that failed or were passed over (also in the snapshot).
    pub errors: Vec<SourceIssue>,
    /// The water type inferred, when it was asked for and something pointed to one.
    pub suggestion: Option<WaterTypeSuggestion>,
}

/// The stations around a spot.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DiscoveryOutcome {
    /// Per measurement, the configured chain, the stations ranked with their reasons and the
    /// stations dropped with why, as JSON (the core's rankings by measurement).
    pub stations_json: String,
    /// Station lists or hydrography that failed.
    pub errors: Vec<SourceIssue>,
}

/// A serde enum's name (`"water_temperature"`).
fn name(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// The spot as the core holds it.
fn spot(info: &SpotInfo) -> Result<Spot, FetchError> {
    let water_type: WaterType = serde_json::from_value(serde_json::Value::String(
        info.water_type.clone(),
    ))
    .map_err(|_| {
        invalid(
            "water type",
            &format!("`{}` is not a water type", info.water_type),
        )
    })?;
    Ok(Spot {
        id: info.id.clone(),
        name: info.name.clone(),
        latitude: info.latitude,
        longitude: info.longitude,
        water_type,
        water_body: None,
        time_zone: info.time_zone.clone(),
    })
}

/// The library's options for a fetch.
fn options(keys: &Keys, allow_metered: bool, infer_water_type: bool) -> library::FetchOptions {
    library::FetchOptions {
        allow_metered,
        secrets: keys.secrets(),
        user_agent: user_agent(),
        infer_water_type,
    }
}

/// Fetches everything the cards of `spot` need at Unix time `now`, through `host`: discovery,
/// the water and marine session, the weather, the normals, the storm watch and the sky, as the
/// command line does. The station lists, rankings and the snapshot are stored through the host
/// and the call ledger saved. A source that fails is listed in the outcome's errors; the fetch
/// fails only on unreadable settings or spot.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub async fn fetch_spot(
    spot: SpotInfo,
    settings_json: String,
    keys: Keys,
    options: FetchOptions,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<SpotFetchOutcome, FetchError> {
    let (config, _) = load(&settings_json)?;
    let spot = self::spot(&spot)?;
    let options = self::options(&keys, options.allow_metered, options.infer_water_type);
    let bridge = Bridge(host);
    // Boxed: the whole fetch is a large future.
    let out = Box::pin(library::fetch_spot(
        &bridge,
        &spot,
        &config,
        UnixTime(now),
        &options,
    ))
    .await;
    let strings = Strings::new(config.presentation.voice);
    let suggestion = out.inferred.as_ref().map(|s| WaterTypeSuggestion {
        water_type: name(&s.water_type),
        reasons: s
            .reasons
            .iter()
            .map(|r| strings.water_type_reason(r))
            .collect(),
    });
    Ok(SpotFetchOutcome {
        snapshot_json: serde_json::to_string(&out.snapshot).map_err(|e| invalid("snapshot", &e))?,
        errors: out.snapshot.errors.iter().map(SourceIssue::from).collect(),
        suggestion,
    })
}

/// The stations around `spot` that can serve its cards under the settings at Unix time `now`,
/// ranked per measurement, through `host`: the station lists (stored through the host), the
/// hydrography, the spot's pins and exclusions and the coverage checks, as a fetch plans them.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub async fn discover_stations(
    spot: SpotInfo,
    settings_json: String,
    keys: Keys,
    allow_metered: bool,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<DiscoveryOutcome, FetchError> {
    let (config, _): (Config, _) = load(&settings_json)?;
    let spot = self::spot(&spot)?;
    let options = self::options(&keys, allow_metered, false);
    let requirements = library::requirements(&config, spot.water_type);
    let bridge = Bridge(host);
    let found = Box::pin(library::stations_near(
        &bridge,
        &spot,
        &config,
        &requirements,
        UnixTime(now),
        &options,
    ))
    .await;
    Ok(DiscoveryOutcome {
        stations_json: serde_json::to_string(&found.ranking.measurements)
            .map_err(|e| invalid("stations", &e))?,
        errors: found.errors.iter().map(SourceIssue::from).collect(),
    })
}

/// The rankings a spot's discovery stores through the host: per spot id, the ranking of
/// [`library::discover`]. Only the rankings are read.
#[derive(serde::Deserialize)]
struct StoredRankings {
    spots: BTreeMap<String, Ranking>,
}

/// The stations the last fetch or lookup of the spot `spot_id` ranked, read from what it
/// stored through `host`, in the shape of [`discover_stations`]'s `stations_json`; `None` when
/// nothing readable is stored. No request is made: a list can show them at once.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn stored_stations(spot_id: String, host: Arc<dyn FetchHost>) -> Option<String> {
    let text = host.load(store::DISCOVERY.to_owned(), store::spot_key(&spot_id))?;
    let stored: store::Stored<StoredRankings> = serde_json::from_str(&text).ok()?;
    let ranking = stored.value.spots.get(&spot_id)?;
    serde_json::to_string(&ranking.measurements).ok()
}

/// Always use the station `key` (`"coops:8518750"`, `"coops:NYH1927:13"` for one current bin,
/// `"ndbc:44025"`, `"usgs:01375000"`; the id is read in the case its source writes it) for
/// the spot `spot_id` from the next fetch on. Kept through `host` with the spot's other
/// choices; an exclusion of the same station is lifted. `now` is Unix seconds.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn pin_station(
    spot_id: String,
    key: String,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<(), FetchError> {
    choose(&spot_id, &key, Choice::Pin, now, host.as_ref())
}

/// Never use the station `key` for the spot `spot_id` from the next fetch on (see
/// [`pin_station`]); a pin of the same station is lifted.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn exclude_station(
    spot_id: String,
    key: String,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<(), FetchError> {
    choose(&spot_id, &key, Choice::Exclude, now, host.as_ref())
}

/// Forgets the pin or exclusion of the station `key` for the spot `spot_id` (see
/// [`pin_station`]).
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn clear_station(
    spot_id: String,
    key: String,
    now: i64,
    host: Arc<dyn FetchHost>,
) -> Result<(), FetchError> {
    choose(&spot_id, &key, Choice::Clear, now, host.as_ref())
}

/// Forgets everything fetches and lookups of the spot `spot_id` kept through `host`: its last
/// snapshot (which holds its exact position), its station rankings and its pins and
/// exclusions. Each is stored empty, which the next fetch reads as nothing stored. The station
/// lists of the area, the observed coverage and the call ledger stay. Call it when the spot
/// is deleted, and again when a fetch or lookup of it that was running then ends.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn forget_spot(spot_id: String, host: Arc<dyn FetchHost>) {
    for (namespace, key) in store::spot_entries(&spot_id) {
        host.store(namespace.to_owned(), key, store::FORGOTTEN.to_owned());
    }
}

/// Applies `choice` to the station `key` in the spot's stored choices.
fn choose(
    spot_id: &str,
    key: &str,
    choice: Choice,
    now: i64,
    host: &dyn FetchHost,
) -> Result<(), FetchError> {
    let key = parse_station_key(key).map_err(|e| invalid("station", &e))?;
    let name = store::spot_key(spot_id);
    let stored = host.load(store::CHOICES.to_owned(), name.clone());
    let value = store::apply_choice(stored.as_deref(), choice, key, UnixTime(now))
        .map_err(|e| invalid("station choices", &e))?;
    host.store(store::CHOICES.to_owned(), name, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use underbite_providers::request::CacheHint;

    fn response(body: Vec<u8>) -> HostResponse {
        HostResponse {
            status: 200,
            body,
            content_type: Some("application/json".into()),
            date: Some(1_794_744_000),
            expires: None,
            last_modified: None,
            etag: Some("\"abc\"".into()),
            retry_after: None,
            rate_limit_remaining: None,
        }
    }

    #[test]
    fn requests_reach_the_app_with_their_cache_flags_and_hide_secrets_in_debug() {
        let call = Call::get("https://api.waterdata.usgs.gov/ogcapi/v0/collections/x?api_key=K1");
        let request = host_request(&call);
        assert_eq!(request.method, HostMethod::Get);
        assert!(request.no_store, "a keyed request is never stored on disk");
        assert!(!format!("{request:?}").contains("K1"), "{request:?}");

        let plain = Call::get("https://tidesandcurrents.noaa.gov/x").with_cache(CacheHint {
            fresh: true,
            no_store: false,
            max_age: Some(60),
        });
        let request = host_request(&plain);
        assert!(request.fresh && !request.no_store);
        assert_eq!(request.max_age_s, Some(60));

        let headers = HostRequest {
            headers: vec![
                Header {
                    name: "X-Api-Key".into(),
                    value: "SECRET-9".into(),
                },
                Header {
                    name: "Accept".into(),
                    value: "application/json".into(),
                },
            ],
            body: Some(r#"{"a":1}"#.into()),
            ..request
        };
        let debug = format!("{headers:?}");
        assert!(!debug.contains("SECRET-9"), "{debug}");
        assert!(debug.contains(r#"Header("X-Api-Key", "***")"#), "{debug}");
        assert!(debug.contains("application/json"), "{debug}");
        assert!(debug.contains("body_len: Some(7)"), "{debug}");
    }

    #[test]
    fn responses_keep_their_headers_and_bodies_are_capped() {
        let mut r = response(b"{}".to_vec());
        r.retry_after = Some(" 120 ".into());
        r.rate_limit_remaining = Some("998".into());
        let f = fetched(r).expect("fetched");
        assert_eq!(f.body, b"{}");
        assert_eq!(f.content_type.as_deref(), Some("application/json"));
        assert_eq!(f.date, Some(UnixTime(1_794_744_000)));
        assert_eq!(f.etag.as_deref(), Some("\"abc\""));
        assert_eq!(
            (f.retry_after, f.rate_limit_remaining),
            (Some(120), Some(998))
        );

        let mut dated = response(Vec::new());
        dated.date = Some(1_791_198_752);
        dated.retry_after = Some("Mon, 05 Oct 2026 11:13:32 GMT".into());
        assert_eq!(fetched(dated).expect("fetched").retry_after, Some(60));

        let at_cap = usize::try_from(MAX_BODY_BYTES).expect("fits");
        assert!(fetched(response(vec![b' '; at_cap])).is_ok());
        assert_eq!(
            fetched(response(vec![b' '; at_cap + 1])),
            Err(TransportError::TooLarge(MAX_BODY_BYTES))
        );
        assert_eq!(max_body_bytes(), 8 * 1024 * 1024);
    }

    #[test]
    fn host_errors_never_quote_a_key() {
        let url = "https://api.example.test/a?site=1&api_key=SECRET-1";
        let e = transport_error(
            HostError::Failed {
                message: format!("could not connect to {url} (SECRET-1)"),
            },
            url,
        );
        let text = e.to_string();
        assert!(!text.contains("SECRET-1"), "{text}");
        assert!(text.starts_with("could not connect to https://api.example.test/a"));
        assert!(matches!(
            transport_error(
                HostError::TimedOut {
                    message: "30 s".into()
                },
                url
            ),
            TransportError::Timeout(_)
        ));
        assert_eq!(
            transport_error(HostError::TooLarge, url),
            TransportError::TooLarge(MAX_BODY_BYTES)
        );
    }

    #[test]
    fn keys_never_reach_debug_and_empty_ones_are_none() {
        let keys = Keys {
            usgs: Some("USGS-SECRET".into()),
            copernicus_user: Some("someone".into()),
            copernicus_password: Some("PASSWORD-SECRET".into()),
        };
        let debug = format!("{keys:?}");
        assert!(
            !debug.contains("SECRET") && !debug.contains("someone"),
            "{debug}"
        );
        let options = options(&keys, true, false);
        assert_eq!(options.secrets.usgs_api_key.as_deref(), Some("USGS-SECRET"));
        assert!(!format!("{options:?}").contains("USGS-SECRET"));
        let blank = Keys {
            usgs: Some("  ".into()),
            ..Keys::default()
        };
        assert_eq!(blank.secrets(), Secrets::default());
    }

    #[test]
    fn the_user_agent_names_the_version_and_platform() {
        let version = core_version();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        let agent = user_agent();
        assert!(
            agent.starts_with(&format!("underbite/{version} (iOS")),
            "{agent}"
        );
        assert!(agent.ends_with(')') && !agent.contains("+)"), "{agent}");
        assert_eq!(
            user_agent_with(Some("https://example.org/underbite")),
            format!("underbite/{version} (iOS; +https://example.org/underbite)")
        );
        assert_eq!(contact_from(""), None);
        assert_eq!(contact_from("https://example.invalid/underbite"), None);
        assert_eq!(
            contact_from(" https://example.org/u "),
            Some("https://example.org/u".into())
        );
    }

    #[test]
    fn spots_read_their_water_type() {
        let info = SpotInfo {
            id: "battery".into(),
            name: "The Battery".into(),
            latitude: 40.7033,
            longitude: -74.017,
            water_type: "tidal_river_estuary".into(),
            time_zone: String::new(),
        };
        assert_eq!(
            spot(&info).expect("spot").water_type,
            WaterType::TidalRiverEstuary
        );
        let bad = SpotInfo {
            water_type: "puddle".into(),
            ..info
        };
        assert!(matches!(spot(&bad), Err(FetchError::Invalid { .. })));
    }
}
