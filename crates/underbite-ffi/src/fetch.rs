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
use std::sync::{Arc, Mutex, PoisonError};

use underbite_config::schema::Config;
use underbite_core::datum::{DatumTable, VerticalDatum};
use underbite_core::discover::rank::Ranking;
use underbite_core::model::{
    Measurement, SourceError, SourceErrorKind, Spot, SpotSnapshot, WaterType,
};
use underbite_core::offline::cache::{
    CachedMetadata, MetadataKind, cache_key, merge_last, serve_offline,
};
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef, ValueKind};
use underbite_core::time::UnixTime;
use underbite_core::view::format::Formatter;
use underbite_core::view::strings::{Key, Strings};
use underbite_fetch::store::{self, Choice, parse_station_key};
use underbite_fetch::{self as library, Call, Fetched, TransportError};
use underbite_providers::coops::mdapi::{self, StationList};
use underbite_providers::ndbc;
use underbite_providers::request::{
    Place, Request, Target, Window, is_secret_header, redact_url, secret_values,
};
use underbite_providers::source::{Secrets, SourceError as ProviderError};
use whether_core::time::parse_http_date;
use whether_providers::Method;

use crate::offline::note_time;
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

/// What a fetch saw of the network and of the cached metadata.
#[derive(Debug, Default)]
struct Traffic {
    /// Metadata responses worth keeping, one per key.
    cacheable: Vec<CachedResponse>,
    /// Requests that got an answer: a 2xx response that is not a web page and, for metadata,
    /// one its provider reads.
    answered: usize,
    /// Requests that got none: no response, another status, or a page in its place (a captive
    /// portal's sign-in page, whatever status it comes with).
    unanswered: usize,
    /// What each cached copy that answered a request was.
    served: Vec<MetadataKind>,
}

/// The app's [`FetchHost`] as the fetch library's. Long-lived station metadata that arrives and
/// its provider reads is noted for the app to keep (`Traffic::cacheable`); when such a request
/// gets no answer (no response, another status than 2xx, a page in its place or a body its
/// provider does not read), the copy the app kept answers it instead, if it is not too old.
struct Bridge {
    host: Arc<dyn FetchHost>,
    cached: BTreeMap<String, CachedMetadata>,
    now: UnixTime,
    traffic: Mutex<Traffic>,
}

impl Bridge {
    fn new(host: Arc<dyn FetchHost>, cached: Vec<CachedResponse>, now: UnixTime) -> Self {
        Self {
            host,
            cached: cached
                .into_iter()
                .map(|c| {
                    let entry = CachedMetadata {
                        key: c.key.clone(),
                        body: c.body,
                        fetched_at: UnixTime(c.fetched_at),
                    };
                    (c.key, entry)
                })
                .collect(),
            now,
            traffic: Mutex::new(Traffic::default()),
        }
    }

    fn traffic(&self) -> std::sync::MutexGuard<'_, Traffic> {
        self.traffic.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The cached copy that answers `url` when the network cannot.
    fn offline_answer(&self, url: &str) -> Option<Fetched> {
        let served = serve_offline(url, self.cached.get(&cache_key(url)), self.now).ok()?;
        self.traffic().served.push(served.kind);
        Some(Fetched {
            status: 200,
            body: served.body.to_vec(),
            content_type: Some(
                match served.kind {
                    MetadataKind::NdbcStations => "application/xml",
                    _ => "application/json",
                }
                .to_owned(),
            ),
            ..Fetched::default()
        })
    }

    /// Whether `response` answers the request for `url`, counted either way; metadata its
    /// provider reads is noted to keep.
    fn note(&self, url: &str, response: &Fetched) -> bool {
        let kind = MetadataKind::of_url(url);
        let answered = (200..300).contains(&response.status)
            && !is_page(response)
            && kind.is_none_or(|kind| metadata_reads(kind, url, &response.body));
        let mut traffic = self.traffic();
        if !answered {
            traffic.unanswered += 1;
            return false;
        }
        traffic.answered += 1;
        let Some(entry) = CachedMetadata::new(url, response.body.clone(), self.now) else {
            return true;
        };
        if !traffic.cacheable.iter().any(|c| c.key == entry.key) {
            traffic.cacheable.push(CachedResponse {
                key: entry.key,
                body: entry.body,
                fetched_at: entry.fetched_at.0,
            });
        }
        true
    }
}

/// Whether a response is a web page: what a captive portal sends in place of an answer, and
/// never what a source the fetch reads answers with.
fn is_page(response: &Fetched) -> bool {
    let html_type = response
        .content_type
        .as_deref()
        .is_some_and(|t| t.to_ascii_lowercase().contains("html"));
    let start = response.body.trim_ascii_start();
    let head = start[..start.len().min(16)].to_ascii_lowercase();
    html_type || head.starts_with(b"<!doctype html") || head.starts_with(b"<html")
}

/// The station id in a CO-OPS metadata URL (`.../stations/{id}/harcon.json`).
fn station_of(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?;
    let (_, rest) = path.split_once("/stations/")?;
    rest.split('/').next().filter(|id| !id.is_empty())
}

/// The value of a query parameter of `url`.
fn query_value<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    let query = url.split('#').next()?.split_once('?')?.1;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

/// Whether the provider of a metadata response reads `body`: a copy is kept, and counts as an
/// answer, only then.
fn metadata_reads(kind: MetadataKind, url: &str, body: &[u8]) -> bool {
    read_metadata(kind, url, body).is_ok()
}

/// Reads a metadata response to `url` as its provider does, keeping nothing: `Ok` when the
/// provider can use `body`, else why not.
pub(crate) fn read_metadata(
    kind: MetadataKind,
    url: &str,
    body: &[u8],
) -> Result<(), ProviderError> {
    let station = station_of(url).unwrap_or_default();
    let unread = |what: &str| ProviderError::Parse(what.to_owned());
    match kind {
        // A current station's constants are asked for by bin.
        MetadataKind::Harmonics if url.contains("bin=") => {
            let bin = query_value(url, "bin")
                .and_then(|bin| bin.parse().ok())
                .ok_or_else(|| unread("no bin"))?;
            mdapi::parse_current_harmonics(station, Some(bin), body).map(|_| ())
        }
        MetadataKind::Harmonics => {
            // Only the constants are read here; `Z0` comes from the datums.
            let msl = DatumTable::new(station, BTreeMap::from([("MSL".to_owned(), 0.0)]));
            mdapi::parse_tide_harmonics(station, body, &msl, VerticalDatum::Msl).map(|_| ())
        }
        MetadataKind::Datums => mdapi::parse_datums(station, body).map(|_| ()),
        MetadataKind::TideOffsets => mdapi::parse_tide_offsets(body).map(|_| ()),
        MetadataKind::CurrentOffsets => mdapi::parse_current_offsets(None, body).map(|_| ()),
        MetadataKind::Bins => mdapi::parse_bins(body).map(|_| ()),
        MetadataKind::CoopsStations => {
            let list = [
                StationList::TidePredictions,
                StationList::CurrentPredictions,
                StationList::WaterLevels,
                StationList::Currents,
            ]
            .into_iter()
            .find(|list| query_value(url, "type") == Some(list.type_name()))
            .ok_or_else(|| unread("an unknown station list"))?;
            mdapi::parse_station_list(list, body).map(|_| ())
        }
        MetadataKind::NdbcStations => {
            let text = std::str::from_utf8(body).map_err(|e| unread(&e.to_string()))?;
            if ndbc::stations::parse(text)?.is_empty() {
                return Err(ProviderError::NoData(Some("no stations".into())));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The station metadata tides and currents are computed from
// ---------------------------------------------------------------------------------------------

/// How old a kept copy of a station's harmonic constants, datums or offsets may be before a
/// fetch with signal asks for it again, seconds: a week. Older copies still answer without
/// signal, up to their kind's own limit.
pub const METADATA_REFRESH_S: i64 = 7 * 86_400;

/// A target naming only `station`, for the metadata requests: no coordinates leave with them.
fn station_target(station: StationRef, measurement: Measurement) -> Target {
    Target {
        place: Place::Station {
            station,
            distance_m: None,
            relation: Relation::Nearby,
        },
        measurements: std::iter::once(measurement).collect(),
        window: Window {
            from: UnixTime(0),
            to: UnixTime(0),
        },
        datum: None,
    }
}

/// The reference station `id` of a subordinate station, as a station of `kind`.
fn reference_station(
    station: &StationRef,
    id: &str,
    kind: StationKind,
    bin: Option<u16>,
) -> StationRef {
    StationRef {
        id: id.to_owned(),
        name: id.to_owned(),
        kind,
        bin,
        reference_id: None,
        ..station.clone()
    }
}

/// The datums and constants requests of the harmonic tide station `station`.
fn tide_reference(station: StationRef) -> [(MetadataKind, Result<Request, ProviderError>); 2] {
    let none = DatumTable::new(&station.id, BTreeMap::new());
    let target = station_target(station, Measurement::Tide);
    [
        (MetadataKind::Datums, mdapi::datums_request(target.clone())),
        (
            MetadataKind::Harmonics,
            mdapi::tide_harcon_request(target, &none),
        ),
    ]
}

/// The metadata requests the computed tides or currents of the CO-OPS `station` rest on, in
/// the order the computation asks for them, each with its kind: a harmonic tide station's
/// datums and constants; a subordinate tide station's offsets and its reference's datums and
/// constants; a harmonic current station's constants for its bin; a subordinate current
/// station's offsets and its reference's constants for the reference bin.
///
/// The reference is read from the subordinate station's `offsets` body when it is given, as the
/// computation reads it; without it, a tide station's `reference_id` names it and a current
/// station's reference stays unknown (only the offsets are listed). Any other station has none.
pub(crate) fn station_metadata(
    station: &StationRef,
    offsets: Option<&[u8]>,
) -> Vec<(MetadataKind, Request)> {
    if station.source != SourceId::COOPS {
        return Vec::new();
    }
    let current = |s: StationRef| station_target(s, Measurement::TidalCurrent);
    let mut wanted: Vec<(MetadataKind, Result<Request, ProviderError>)> = Vec::new();
    match station.kind {
        StationKind::TideHarmonic => wanted.extend(tide_reference(station.clone())),
        StationKind::TideSubordinate => {
            wanted.push((
                MetadataKind::TideOffsets,
                mdapi::tide_offsets_request(station_target(station.clone(), Measurement::Tide)),
            ));
            let reference = offsets
                .and_then(|body| mdapi::parse_tide_offsets(body).ok().flatten())
                .map(|o| o.reference_id)
                .or_else(|| station.reference_id.clone());
            if let Some(id) = reference {
                let s = reference_station(station, &id, StationKind::TideHarmonic, None);
                wanted.extend(tide_reference(s));
            }
        }
        StationKind::CurrentHarmonic => wanted.push((
            MetadataKind::Harmonics,
            mdapi::current_harcon_request(current(station.clone())),
        )),
        StationKind::CurrentSubordinate => {
            wanted.push((
                MetadataKind::CurrentOffsets,
                mdapi::current_offsets_request(current(station.clone())),
            ));
            if let Some(o) =
                offsets.and_then(|body| mdapi::parse_current_offsets(station.bin, body).ok())
            {
                let s = reference_station(
                    station,
                    &o.reference_id,
                    StationKind::CurrentHarmonic,
                    Some(o.reference_bin),
                );
                wanted.push((
                    MetadataKind::Harmonics,
                    mdapi::current_harcon_request(current(s)),
                ));
            }
        }
        _ => {}
    }
    wanted
        .into_iter()
        .filter_map(|(kind, request)| Some((kind, request.ok()?)))
        .collect()
}

/// The CO-OPS stations of a snapshot's tide and tidal current.
fn computed_stations(snapshot: &SpotSnapshot) -> Vec<&StationRef> {
    let tide = snapshot.marine.tide.as_ref().map(|t| &t.provenance);
    let current = snapshot
        .marine
        .tidal_current
        .as_ref()
        .map(|c| &c.provenance);
    [tide, current]
        .into_iter()
        .flatten()
        .filter_map(|p| p.station.as_ref())
        .filter(|s| s.source == SourceId::COOPS)
        .collect()
}

fn is_offsets(kind: MetadataKind) -> bool {
    matches!(
        kind,
        MetadataKind::TideOffsets | MetadataKind::CurrentOffsets
    )
}

impl Bridge {
    /// Asks for what computes the tide and current of `snapshot` without signal (its stations'
    /// constants, datums and offsets) unless the app kept a copy less than
    /// [`METADATA_REFRESH_S`] old or this fetch already got one; `refresh` asks again in any
    /// case, past any cache. What arrives and its provider reads is noted to keep. The requests
    /// name the stations only.
    async fn keep_station_metadata(&self, snapshot: &SpotSnapshot, refresh: bool) {
        for station in computed_stations(snapshot) {
            // A subordinate station's offsets name its reference: they come first.
            let mut offsets = None;
            for (_, request) in station_metadata(station, None)
                .iter()
                .filter(|(kind, _)| is_offsets(*kind))
            {
                offsets = self.metadata_body(request, refresh).await;
            }
            for (kind, request) in station_metadata(station, offsets.as_deref()) {
                if !is_offsets(kind) {
                    self.metadata_body(&request, refresh).await;
                }
            }
        }
    }

    /// The body of a metadata `request`: what this fetch already received, a kept copy fresh
    /// enough (unless `refresh`), else the network's answer, or the kept copy of any age it
    /// may be served at when the network does not answer.
    async fn metadata_body(&self, request: &Request, refresh: bool) -> Option<Vec<u8>> {
        let key = cache_key(&request.url);
        let received = self
            .traffic()
            .cacheable
            .iter()
            .find(|c| c.key == key)
            .map(|c| c.body.clone());
        if received.is_some() {
            return received;
        }
        if !refresh
            && let Some(kept) = self.cached.get(&key)
            && self.now.0 - kept.fetched_at.0 <= METADATA_REFRESH_S
        {
            return Some(kept.body.clone());
        }
        let mut call = Call::from(request);
        call.cache.fresh |= refresh;
        let fetched = library::FetchHost::send(self, &call).await.ok()?;
        (200..300).contains(&fetched.status).then_some(fetched.body)
    }
}

impl library::FetchHost for Bridge {
    async fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        let fetched = match self.host.send(host_request(call)).await {
            Ok(response) => fetched(response),
            Err(e) => Err(transport_error(e, &call.url)),
        };
        match fetched {
            Ok(fetched) if self.note(&call.url, &fetched) => Ok(fetched),
            // Not an answer: a kept copy answers metadata, anything else fails as it came.
            Ok(fetched) => Ok(self.offline_answer(&call.url).unwrap_or(fetched)),
            Err(e) => {
                self.traffic().unanswered += 1;
                self.offline_answer(&call.url).ok_or(e)
            }
        }
    }

    // The store and the ledger are plain calls on the app's side: each is done when it returns.

    fn load(&self, namespace: &str, key: &str) -> impl Future<Output = Option<String>> + Send {
        ready(self.host.load(namespace.to_owned(), key.to_owned()))
    }

    fn store(&self, namespace: &str, key: &str, value: String) -> impl Future<Output = ()> + Send {
        self.host.store(namespace.to_owned(), key.to_owned(), value);
        ready(())
    }

    fn save_usage(&self, ledger: String) -> impl Future<Output = Result<(), String>> + Send {
        ready(self.host.save_usage(ledger).map_err(|e| e.to_string()))
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
    /// Whether the station metadata the spot's tides and currents are computed from without
    /// signal is asked for again even when the kept copy is recent: a pull to refresh.
    #[uniffi(default = false)]
    pub refresh_metadata: bool,
}

/// A response to long-lived station metadata (harmonic constants, datums, offsets, bins, station
/// lists) the app keeps for a spot, so a fetch without signal can still compute tides. The key
/// is the request's URL without anything that looks like a credential.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct CachedResponse {
    /// The request's cache key.
    pub key: String,
    /// The response body.
    pub body: Vec<u8>,
    /// When it was fetched, Unix seconds.
    pub fetched_at: i64,
}

impl fmt::Debug for CachedResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedResponse")
            .field("key", &self.key)
            .field("body_len", &self.body.len())
            .field("fetched_at", &self.fetched_at)
            .finish()
    }
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
    /// Whether nothing answered: requests were made and none got an answer (no response, another
    /// status than 2xx, or a web page such as a captive portal's sign-in in place of data). The
    /// snapshot is then the offline one, and the spot has no new values to show.
    #[uniffi(default = false)]
    pub unanswered: bool,
    /// Station metadata responses to keep for the next fetch without signal, passed back as
    /// `cached_metadata`: one per request, each replacing the copy under its key.
    #[uniffi(default = [])]
    pub cacheable: Vec<CachedResponse>,
}

/// The stations around a spot.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DiscoveryOutcome {
    /// Per measurement, the configured chain, the stations ranked with their reasons and the
    /// stations dropped with why, as JSON (the core's rankings by measurement).
    pub stations_json: String,
    /// Station lists or hydrography that failed.
    pub errors: Vec<SourceIssue>,
    /// Whether nothing answered: requests were made and none got an answer (no response, another
    /// status than 2xx, or a web page such as a captive portal's sign-in in place of data). The
    /// stations are then empty, and the lookup says nothing about the spot.
    #[uniffi(default = false)]
    pub unanswered: bool,
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
///
/// `cached_metadata` is what the app kept from earlier fetches' `cacheable` lists. Without
/// signal (no request got a response) the station metadata in it answers the requests for
/// harmonic constants, so tides are computed on the device, and the values the fetch lacks come
/// from the spot's last snapshot with their own times; the outcome's errors then say "offline:
/// computed from harmonic constants, other values from <time>". Anything else fails as usual.
///
/// With signal, the fetch makes sure the app keeps what computes the snapshot's tide and current
/// later: the harmonic constants, datums and offsets of their CO-OPS stations (and of a
/// subordinate station's reference) are asked for when `cached_metadata` lacks them or holds a
/// copy more than [`METADATA_REFRESH_S`] old, or always with `options.refresh_metadata`, and
/// returned in `cacheable`. These requests name the stations only, never a position.
#[uniffi::export(default(cached_metadata = []))]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub async fn fetch_spot(
    spot: SpotInfo,
    settings_json: String,
    keys: Keys,
    options: FetchOptions,
    now: i64,
    cached_metadata: Vec<CachedResponse>,
    host: Arc<dyn FetchHost>,
) -> Result<SpotFetchOutcome, FetchError> {
    let (config, _) = load(&settings_json)?;
    let info = spot;
    let spot = self::spot(&info)?;
    let refresh_metadata = options.refresh_metadata;
    let options = self::options(&keys, options.allow_metered, options.infer_water_type);
    let bridge = Bridge::new(host, cached_metadata, UnixTime(now));
    let previous = last_snapshot(&bridge, &spot.id);
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
    // With signal, what computes the tide and current without it is kept up to date.
    let online = bridge.traffic().answered > 0;
    if online {
        Box::pin(bridge.keep_station_metadata(&out.snapshot, refresh_metadata)).await;
    }
    let traffic = std::mem::take(&mut *bridge.traffic());
    let mut snapshot = out.snapshot;
    let unanswered = traffic.answered == 0 && traffic.unanswered > 0;
    if unanswered {
        let zone = if snapshot.spot.time_zone.is_empty() {
            info.time_zone.as_str()
        } else {
            snapshot.spot.time_zone.as_str()
        };
        let f = crate::map::formatter(&config, zone);
        let computed =
            traffic.served.contains(&MetadataKind::Harmonics) && computed_here(&snapshot);
        snapshot = offline(&bridge, strings, &f, previous.as_ref(), snapshot, computed);
    }
    Ok(SpotFetchOutcome {
        snapshot_json: serde_json::to_string(&snapshot).map_err(|e| invalid("snapshot", &e))?,
        errors: snapshot.errors.iter().map(SourceIssue::from).collect(),
        suggestion,
        unanswered,
        cacheable: traffic.cacheable,
    })
}

/// Whether the fetch computed a tide or a current itself (from harmonic constants).
fn computed_here(snapshot: &SpotSnapshot) -> bool {
    let tide = snapshot.marine.tide.as_ref().map(|t| t.provenance.kind);
    let current = snapshot
        .marine
        .tidal_current
        .as_ref()
        .map(|c| c.provenance.kind);
    [tide, current].contains(&Some(ValueKind::Computed))
}

/// The spot's snapshot as the last fetch stored it.
fn last_snapshot(bridge: &Bridge, spot_id: &str) -> Option<SpotSnapshot> {
    let text = bridge
        .host
        .load(store::SPOT.to_owned(), store::spot_key(spot_id))?;
    let stored: store::Stored<SpotSnapshot> = serde_json::from_str(&text).ok()?;
    Some(stored.value)
}

/// The snapshot of a fetch without signal: the values it lacks come from the last snapshot with
/// their own times, the offline note is added to its errors, and the result is kept in place of
/// the sparse one the fetch stored, so the next fetch without signal still has them. `computed`
/// says the fetch computed a tide or current from kept harmonic constants.
fn offline(
    bridge: &Bridge,
    strings: Strings,
    f: &Formatter,
    previous: Option<&SpotSnapshot>,
    fresh: SpotSnapshot,
    computed: bool,
) -> SpotSnapshot {
    let mut merged = match previous {
        Some(last) => merge_last(last, fresh),
        None => fresh,
    };
    let time = previous.map(|last| note_time(f, last.fetched_at));
    let message = match (computed, time) {
        (true, Some(time)) => strings.fmt(Key::OfflineComputedLast, &[("time", &time)]),
        (true, None) => strings.get(Key::OfflineComputedOnly).to_owned(),
        (false, Some(time)) => strings.fmt(Key::OfflineLastOnly, &[("time", &time)]),
        (false, None) => return merged,
    };
    merged.errors.push(SourceError {
        source: SourceId::from_static("offline"),
        measurement: None,
        kind: SourceErrorKind::Stale,
        message,
    });
    if let Ok(json) = serde_json::to_string(&store::Stored {
        saved_at: bridge.now.0,
        value: &merged,
    }) {
        bridge.host.store(
            store::SPOT.to_owned(),
            store::spot_key(&merged.spot.id),
            json,
        );
    }
    merged
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
    let bridge = Bridge::new(host, Vec::new(), UnixTime(now));
    let found = Box::pin(library::stations_near(
        &bridge,
        &spot,
        &config,
        &requirements,
        UnixTime(now),
        &options,
    ))
    .await;
    let traffic = std::mem::take(&mut *bridge.traffic());
    Ok(DiscoveryOutcome {
        stations_json: serde_json::to_string(&found.ranking.measurements)
            .map_err(|e| invalid("stations", &e))?,
        errors: found.errors.iter().map(SourceIssue::from).collect(),
        unanswered: traffic.answered == 0 && traffic.unanswered > 0,
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

    fn coops(file: &str) -> Vec<u8> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read(root.join("../../tests/fixtures/raw/coops").join(file)).unwrap()
    }

    #[test]
    fn metadata_is_kept_only_when_its_provider_reads_it() {
        let base = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi";
        let page = b"<!DOCTYPE html><html><body>Sign in</body></html>".to_vec();
        let cases = [
            (
                MetadataKind::Harmonics,
                format!("{base}/stations/8518750/harcon.json?units=metric"),
                coops("coops-mdapi-8518750-harcon-metric.json"),
            ),
            (
                MetadataKind::Harmonics,
                format!("{base}/stations/NYH1927/harcon.json?bin=13"),
                coops("coops-mdapi-NYH1927-harcon.json"),
            ),
            (
                MetadataKind::Datums,
                format!("{base}/stations/8518750/datums.json?units=metric"),
                coops("coops-mdapi-8518750-datums-metric.json"),
            ),
            (
                MetadataKind::TideOffsets,
                format!("{base}/stations/8518919/tidepredoffsets.json"),
                coops("coops-mdapi-8518919-tidepredoffsets.json"),
            ),
        ];
        for (kind, url, body) in cases {
            assert!(metadata_reads(kind, &url, &body), "{url}");
            assert!(!metadata_reads(kind, &url, &page), "{url}");
            assert!(!metadata_reads(kind, &url, b"{\"error\":"), "{url}");
        }
        let html = |content_type: &str, body: &[u8]| Fetched {
            status: 200,
            content_type: Some(content_type.into()),
            body: body.to_vec(),
            ..Fetched::default()
        };
        assert!(is_page(&html("text/html; charset=utf-8", b"Sign in")));
        assert!(is_page(&html("application/json", b"  <HTML><body/>")));
        assert!(!is_page(&html("application/json", b"{\"a\":1}")));
        assert!(!is_page(&html(
            "text/xml",
            b"<?xml version=\"1.0\"?><stations/>"
        )));
    }

    fn coops_station(id: &str, kind: StationKind, bin: Option<u16>) -> StationRef {
        StationRef {
            source: SourceId::COOPS,
            id: id.into(),
            name: id.into(),
            kind,
            latitude: 40.7,
            longitude: -74.0,
            bin,
            reference_id: None,
        }
    }

    /// The URLs of `station_metadata`, after the metadata API's base, with their kinds.
    fn metadata_urls(station: &StationRef, offsets: Option<&[u8]>) -> Vec<(MetadataKind, String)> {
        let base = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/";
        station_metadata(station, offsets)
            .into_iter()
            .map(|(kind, r)| {
                assert_eq!(MetadataKind::of_url(&r.url), Some(kind), "{}", r.url);
                (kind, r.url.strip_prefix(base).expect("mdapi").to_owned())
            })
            .collect()
    }

    #[test]
    fn station_metadata_follows_the_computation() {
        use MetadataKind::{CurrentOffsets, Datums, Harmonics, TideOffsets};
        let battery = coops_station("8518750", StationKind::TideHarmonic, None);
        assert_eq!(
            metadata_urls(&battery, None),
            [
                (Datums, "8518750/datums.json?units=metric".to_owned()),
                (Harmonics, "8518750/harcon.json?units=metric".to_owned()),
            ]
        );

        // A subordinate tide station: the offsets, then its reference's datums and constants,
        // the reference read from the offsets when they are there.
        let mut sub = coops_station("8518919", StationKind::TideSubordinate, None);
        assert_eq!(
            metadata_urls(&sub, None),
            [(TideOffsets, "8518919/tidepredoffsets.json".to_owned())]
        );
        sub.reference_id = Some("8518750".into());
        let offsets = coops("coops-mdapi-8518919-tidepredoffsets.json");
        for given in [None, Some(offsets.as_slice())] {
            assert_eq!(
                metadata_urls(&sub, given),
                [
                    (TideOffsets, "8518919/tidepredoffsets.json".to_owned()),
                    (Datums, "8518750/datums.json?units=metric".to_owned()),
                    (Harmonics, "8518750/harcon.json?units=metric".to_owned()),
                ]
            );
        }

        // Current stations by bin; a subordinate one's reference comes from its offsets.
        let entrance = coops_station("NYH1927", StationKind::CurrentHarmonic, Some(13));
        assert_eq!(
            metadata_urls(&entrance, None),
            [(Harmonics, "NYH1927/harcon.json?bin=13".to_owned())]
        );
        let sub = coops_station("ACT3646", StationKind::CurrentSubordinate, Some(1));
        assert_eq!(
            metadata_urls(&sub, None),
            [(
                CurrentOffsets,
                "ACT3646_1/currentpredictionoffsets.json".to_owned()
            )]
        );
        let offsets = coops("coops-mdapi-ACT3646_1-currentpredictionoffsets.json");
        let reference = mdapi::parse_current_offsets(Some(1), &offsets).expect("offsets");
        let urls = metadata_urls(&sub, Some(&offsets));
        assert_eq!(urls.len(), 2, "{urls:?}");
        assert_eq!(
            urls[1],
            (
                Harmonics,
                format!(
                    "{}/harcon.json?bin={}",
                    reference.reference_id, reference.reference_bin
                )
            )
        );

        // Other stations and sources have none.
        let buoy = StationRef {
            source: SourceId::from_static("ndbc"),
            ..battery.clone()
        };
        assert_eq!(metadata_urls(&buoy, None), []);
        let meter = coops_station("n06010", StationKind::PortsCurrentMeter, Some(1));
        assert_eq!(metadata_urls(&meter, None), []);
    }

    #[test]
    fn metadata_is_read_with_its_reason() {
        let url = "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750/datums.json?units=metric";
        assert_eq!(
            read_metadata(
                MetadataKind::Datums,
                url,
                &coops("coops-mdapi-8518750-datums-metric.json")
            ),
            Ok(())
        );
        assert!(matches!(
            read_metadata(MetadataKind::Datums, url, b"<html>"),
            Err(ProviderError::Parse(_))
        ));
        let list =
            "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations.json?type=other";
        assert!(read_metadata(MetadataKind::CoopsStations, list, b"{}").is_err());
    }

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
