//! The host a fetch runs through: the calls it sends, the responses it gets back, the values it
//! keeps between fetches, and the call ledger.
//!
//! The host does all the I/O: the network, a response cache, a store for values. A fetch only
//! decides what to ask; [`Driver`] adds what belongs to one fetch (the call ledger and the
//! responses kept while the water type is inferred).

use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use underbite_core::time::UnixTime;
use underbite_providers::request::{
    CacheHint, Request, Response, is_secret_header, redact_url, secret_values,
};
use underbite_providers::session::usage::{ResponseLimits, SharedUsage, UsageLedger};
use underbite_providers::source::{Source, SourceError};
use whether_providers::{HttpRequest, Method};

use crate::store::{self, Stored};

/// One HTTP call, built from either kind of request: Underbite's own [`Request`] and whether's
/// [`HttpRequest`] (the weather and storm sessions).
#[derive(Clone, PartialEq)]
pub struct Call {
    /// Full URL; may carry a key, so show it only through [`redact_url`].
    pub url: String,
    /// Extra headers.
    pub headers: Vec<(&'static str, String)>,
    /// GET, or POST with a JSON body.
    pub method: Method,
    /// The JSON body of a POST.
    pub body: Option<String>,
    /// How the response may be cached.
    pub cache: CacheHint,
}

impl Call {
    /// A GET of `url` with default caching.
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(
            url.into(),
            Vec::new(),
            Method::Get,
            None,
            CacheHint::default(),
        )
    }

    fn new(
        url: String,
        headers: Vec<(&'static str, String)>,
        method: Method,
        body: Option<String>,
        mut cache: CacheHint,
    ) -> Self {
        let keyed = !secret_values(&url).is_empty()
            || headers.iter().any(|(name, _)| is_secret_header(name));
        cache.no_store |= keyed;
        Self {
            url,
            headers,
            method,
            body,
            cache,
        }
    }

    /// The same call with other cache hints (the storm plan's).
    #[must_use]
    pub fn with_cache(mut self, cache: CacheHint) -> Self {
        self.cache.fresh = cache.fresh;
        self.cache.no_store |= cache.no_store;
        self.cache.max_age = cache.max_age;
        self
    }

    /// Whether the response may be written to disk: never when the call carries a key.
    pub fn may_store_on_disk(&self) -> bool {
        !self.cache.no_store
    }

    /// The URL with secrets masked, for messages and logs.
    pub fn redacted_url(&self) -> String {
        redact_url(&self.url)
    }

    /// Whether `other` asks for the same thing: URL, method, body and headers.
    fn same_request(&self, other: &Self) -> bool {
        self.url == other.url
            && self.method == other.method
            && self.body == other.body
            && self.headers == other.headers
    }
}

impl From<&Request> for Call {
    fn from(req: &Request) -> Self {
        let mut call = Self::new(
            req.url.clone(),
            req.headers.clone(),
            Method::Get,
            None,
            req.cache,
        );
        call.cache.no_store |= !req.may_store_on_disk();
        call
    }
}

impl From<&HttpRequest> for Call {
    fn from(req: &HttpRequest) -> Self {
        Self::new(
            req.url.clone(),
            req.headers.clone(),
            req.method,
            req.body.clone(),
            CacheHint::default(),
        )
    }
}

impl fmt::Debug for Call {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call")
            .field("url", &self.redacted_url())
            .field("method", &self.method)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

/// A response that owns its bytes and the headers the sessions use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetched {
    /// Status code.
    pub status: u16,
    /// Body bytes, decompressed.
    pub body: Vec<u8>,
    /// The `Content-Type` header.
    pub content_type: Option<String>,
    /// The `Date` header.
    pub date: Option<UnixTime>,
    /// The `Expires` header.
    pub expires: Option<UnixTime>,
    /// `max-age` of `Cache-Control`, seconds.
    pub cache_max_age: Option<u32>,
    /// The `Last-Modified` header.
    pub last_modified: Option<String>,
    /// The `ETag` header.
    pub etag: Option<String>,
    /// `Retry-After`, seconds (a date is converted against the `Date` header).
    pub retry_after: Option<u32>,
    /// The `X-RateLimit-Remaining` header.
    pub rate_limit_remaining: Option<u32>,
}

impl Fetched {
    /// A response with just a status and body.
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            ..Self::default()
        }
    }

    /// A borrowed view for the sources.
    pub fn as_response(&self) -> Response<'_> {
        Response {
            status: self.status,
            body: &self.body,
            content_type: self.content_type.as_deref(),
            date: self.date,
            expires: self.expires,
            cache_max_age: self.cache_max_age,
            last_modified: self.last_modified.as_deref(),
            etag: self.etag.as_deref(),
            retry_after: self.retry_after,
            rate_limit_remaining: self.rate_limit_remaining,
        }
    }
}

/// Why a call got no response. Messages never contain secrets.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// The request or connection timed out.
    #[error("timed out: {0}")]
    Timeout(String),
    /// The body was larger than the cap.
    #[error("response larger than {0} bytes")]
    TooLarge(u64),
    /// DNS, TLS, connection or protocol failure.
    #[error("{0}")]
    Failed(String),
    /// Replay has no route for this (redacted) URL.
    #[error("no replay route for {0}")]
    NoRoute(String),
}

/// What a fetch needs from the platform: HTTP, a response cache, a store for values kept
/// between fetches, and somewhere durable for the call ledger.
///
/// A fetch calls the host one request at a time and in a fixed order. Stored values are opaque
/// strings (JSON the fetch writes and reads back); namespaces and keys are letters, digits,
/// `-`, `_` and `.`, and never contain coordinates or keys (see [`crate::store`]).
pub trait FetchHost: Sync {
    /// Sends `call` and returns any status with its body. The host may answer from its cache
    /// as the call's hints allow: never when [`CacheHint::fresh`] asks for a fresh copy, and a
    /// response to a call that may not be stored on disk ([`Call::may_store_on_disk`]) is never
    /// written there.
    fn send(&self, call: &Call) -> impl Future<Output = Result<Fetched, TransportError>> + Send;

    /// The response the host's cache would answer `call` with, without sending anything. A
    /// metered call the cache answers is not counted against its budget. Hosts without a cache
    /// they can look into answer `None`, and every metered call is counted.
    fn cached(&self, call: &Call) -> impl Future<Output = Option<Fetched>> + Send {
        let _ = call;
        std::future::ready(None)
    }

    /// The value last stored under `namespace` and `key`, if any.
    fn load(&self, namespace: &str, key: &str) -> impl Future<Output = Option<String>> + Send;

    /// Stores `value` under `namespace` and `key`. A value that cannot be stored is only
    /// fetched again next time.
    fn store(&self, namespace: &str, key: &str, value: String) -> impl Future<Output = ()> + Send;

    /// Stores the call ledger durably, to be loaded as [`store::USAGE`] / [`store::LEDGER_KEY`]
    /// next time. A metered request goes out only after this succeeds; the error says why
    /// not.
    fn save_usage(&self, ledger: String) -> impl Future<Output = Result<(), String>> + Send;
}

/// Sends one request outside a fetch, through the host and its cache: no call ledger, no kept
/// responses. What the tide table and fixture recording use.
pub async fn send<H: FetchHost>(host: &H, call: &Call) -> Result<Fetched, TransportError> {
    host.send(call).await
}

/// One fetch's way out: the host, the call ledger the sessions share and the responses kept
/// while [`Driver::keeping`] runs. Requests go out one at a time.
pub(crate) struct Driver<'h, H> {
    host: &'h H,
    pub(crate) now: UnixTime,
    pub(crate) usage: SharedUsage,
    pub(crate) allow_metered: bool,
    /// Whether responses are kept for the rest of the fetch (see [`Driver::keeping`]).
    keep_responses: AtomicBool,
    /// The responses kept, answered again without the host or the ledger.
    kept: Mutex<Vec<(Call, Fetched)>>,
}

impl<'h, H: FetchHost> Driver<'h, H> {
    /// A driver at `now`, with the ledger the host stored (an empty one when it has none).
    pub(crate) async fn new(host: &'h H, now: UnixTime, allow_metered: bool) -> Self {
        let ledger = load::<UsageLedger, H>(host, store::USAGE, store::LEDGER_KEY)
            .await
            .map(|stored| stored.value)
            .unwrap_or_default();
        Self {
            host,
            now,
            usage: SharedUsage::new(ledger),
            allow_metered,
            keep_responses: AtomicBool::new(false),
            kept: Mutex::new(Vec::new()),
        }
    }

    /// Runs `f` with every response it receives kept for the rest of the fetch, so the same
    /// requests later (from discovery or a session) are answered without going out again,
    /// with or without a cache on the host. Only for a few lookups: the bodies stay in memory.
    pub(crate) async fn keeping<T>(&self, f: impl Future<Output = T>) -> T {
        self.keep_responses.store(true, Ordering::Relaxed);
        let out = f.await;
        self.keep_responses.store(false, Ordering::Relaxed);
        out
    }

    /// A kept response to the same request as `call`; never for a fresh copy.
    fn kept(&self, call: &Call) -> Option<Fetched> {
        if call.cache.fresh {
            return None;
        }
        let kept = self.kept.lock().ok()?;
        kept.iter()
            .find(|(c, _)| c.same_request(call))
            .map(|(_, f)| f.clone())
    }

    /// Saves the ledger; a metered request goes out only after this succeeds.
    pub(crate) async fn save_usage(&self) -> Result<(), String> {
        let not_saved = |e: &dyn fmt::Display| format!("the call ledger was not saved: {e}");
        let json = stored_json(&self.usage.snapshot(), self.now).map_err(|e| not_saved(&e))?;
        self.host.save_usage(json).await.map_err(|e| not_saved(&e))
    }

    /// Sends `call` through the host, or answers it from the kept responses.
    pub(crate) async fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        if let Some(hit) = self.kept(call) {
            tracing::debug!(url = %call.redacted_url(), "kept");
            return Ok(hit);
        }
        let fetched = self.host.send(call).await?;
        Ok(self.remember(call, fetched))
    }

    /// The response the host's cache holds for `call`, if any; nothing is sent.
    pub(crate) async fn cached_response(&self, call: &Call) -> Option<Fetched> {
        self.host.cached(call).await
    }

    /// Keeps `fetched` while [`Driver::keeping`] runs; returns it.
    fn remember(&self, call: &Call, fetched: Fetched) -> Fetched {
        if self.keep_responses.load(Ordering::Relaxed)
            && !call.cache.fresh
            && let Ok(mut kept) = self.kept.lock()
        {
            kept.push((call.clone(), fetched.clone()));
        }
        fetched
    }

    /// Sends a request of `source` outside a session: a metered one spends from the ledger
    /// (saved first) unless the host's cache answers it.
    pub(crate) async fn get(
        &self,
        source: &dyn Source,
        request: &Request,
    ) -> Result<Fetched, SourceError> {
        let call = Call::from(request);
        if let Some(hit) = self.kept(&call) {
            return Ok(hit);
        }
        if let Some(hit) = self.cached_response(&call).await {
            return Ok(self.remember(&call, hit));
        }
        if request.metered {
            let budget = source.budget().ok_or_else(|| {
                SourceError::Unsupported("a metered request without a budget".into())
            })?;
            if !self.allow_metered {
                return Err(SourceError::RateLimited {
                    retry_after_s: None,
                });
            }
            let now = self.now;
            self.usage
                .with(|ledger| ledger.try_spend(&source.id(), budget, now))
                .map_err(|quota| SourceError::RateLimited {
                    retry_after_s: Some(quota.retry_after_s(now)),
                })?;
            self.save_usage().await.map_err(SourceError::Transport)?;
        }
        let fetched = self
            .send(&call)
            .await
            .map_err(|e| SourceError::Transport(e.to_string()))?;
        if request.metered {
            // A 429 pauses the pool (saved with the ledger at the end of the fetch), so the
            // next request of any spot does not ask again at once.
            let limits = ResponseLimits {
                status: fetched.status,
                retry_after_s: fetched.retry_after,
                remaining: fetched.rate_limit_remaining,
            };
            let now = self.now;
            let id = source.id();
            self.usage
                .with(|ledger| ledger.observe_response(&id, source.budget(), limits, now));
        }
        Ok(fetched)
    }

    /// The value stored under `namespace` and `key` while it is at most `ttl_s` old.
    pub(crate) async fn stored_within<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
        ttl_s: i64,
    ) -> Option<T> {
        let stored = load::<T, H>(self.host, namespace, key).await?;
        let age = self.now.0 - stored.saved_at;
        (0..=ttl_s).contains(&age).then_some(stored.value)
    }

    /// The value stored under `namespace` and `key`, however old.
    pub(crate) async fn stored<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
    ) -> Option<T> {
        load::<T, H>(self.host, namespace, key)
            .await
            .map(|s| s.value)
    }

    /// Stores `value` under `namespace` and `key`, stamped with the fetch's time.
    pub(crate) async fn keep<T: Serialize>(&self, namespace: &str, key: &str, value: &T) {
        match stored_json(value, self.now) {
            Ok(json) => self.host.store(namespace, key, json).await,
            Err(e) => tracing::warn!(namespace, error = %e, "not cached"),
        }
    }
}

/// `value` stamped with `now`, as the host stores it.
fn stored_json<T: Serialize>(value: &T, now: UnixTime) -> serde_json::Result<String> {
    serde_json::to_string(&Stored {
        saved_at: now.0,
        value,
    })
}

/// The value the host stored under `namespace` and `key`; one that does not read back is
/// passed over (and replaced when the fetch stores it again).
async fn load<T: DeserializeOwned, H: FetchHost>(
    host: &H,
    namespace: &str,
    key: &str,
) -> Option<Stored<T>> {
    let text = host.load(namespace, key).await?;
    serde_json::from_str(&text)
        .inspect_err(|e| tracing::warn!(namespace, error = %e, "stored value passed over"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MemoryHost, now};
    use underbite_core::provenance::SourceId;
    use underbite_providers::request::{
        Coordinates, Place, Resource, RoundedPoint, Target, Window,
    };

    fn target() -> Target {
        Target {
            place: Place::Point(RoundedPoint::new(
                Coordinates {
                    latitude: 40.7,
                    longitude: -74.02,
                },
                2,
            )),
            measurements: std::collections::BTreeSet::new(),
            window: Window {
                from: UnixTime(0),
                to: UnixTime(1),
            },
            datum: None,
        }
    }

    #[test]
    fn keyed_calls_are_never_stored() {
        let keyed = Request::new(
            "https://api.example.test/a?site=1&api_key=K".into(),
            Resource::CoopsStations,
            target(),
        );
        assert!(!Call::from(&keyed).may_store_on_disk());
        assert!(!format!("{:?}", Call::from(&keyed)).contains("api_key=K"));
        let plain = Request::new(
            "https://api.example.test/a".into(),
            Resource::CoopsStations,
            target(),
        );
        assert!(Call::from(&plain).may_store_on_disk());
        let with_header = plain.header("X-Api-Key", "K");
        assert!(!Call::from(&with_header).may_store_on_disk());
    }

    #[test]
    fn values_are_stamped_read_back_and_expire() {
        let host = MemoryHost::default();
        pollster::block_on(async {
            let driver = Driver::new(&host, now(), true).await;
            driver
                .keep("stations", "list", &vec!["a".to_string()])
                .await;
            assert_eq!(
                host.value("stations", "list").as_deref(),
                Some(format!(r#"{{"saved_at":{},"value":["a"]}}"#, now().0).as_str())
            );
            let fresh: Option<Vec<String>> = driver.stored_within("stations", "list", 60).await;
            assert_eq!(fresh, Some(vec!["a".to_string()]));
            let later = Driver::new(&host, UnixTime(now().0 + 61), true).await;
            let stale: Option<Vec<String>> = later.stored_within("stations", "list", 60).await;
            assert_eq!(stale, None);
            let any: Option<Vec<String>> = later.stored("stations", "list").await;
            assert!(any.is_some());
            let wrong: Option<u32> = later.stored("stations", "list").await;
            assert_eq!(wrong, None);
        });
    }

    #[test]
    fn the_ledger_is_loaded_and_saved_through_the_host() {
        use underbite_providers::session::usage::OPEN_METEO;
        use underbite_providers::source::Period;

        let host = MemoryHost::default();
        pollster::block_on(async {
            let driver = Driver::new(&host, now(), true).await;
            driver
                .usage
                .with(|l| l.count(&OPEN_METEO, Period::Day, now()));
            driver.save_usage().await.expect("saved");
            let again = Driver::new(&host, now(), true).await;
            assert_eq!(again.usage.snapshot().used(&OPEN_METEO, now()), 1);
        });
        let failing = MemoryHost::default().failing_usage();
        let err = pollster::block_on(async {
            Driver::new(&failing, now(), true).await.save_usage().await
        })
        .expect_err("not saved");
        assert!(err.starts_with("the call ledger was not saved: "), "{err}");
    }

    /// A USGS request that costs a call.
    fn usgs_request(url: &str) -> Request {
        Request::new(url.into(), Resource::UsgsMonitoringLocations, target()).metered()
    }

    #[test]
    fn a_429_without_retry_after_pauses_the_pool_and_the_next_request_is_not_sent() {
        use underbite_providers::usgs::Usgs;

        let host = MemoryHost::default()
            .answering_part("waterdata.test/a", Fetched::new(429, "slow down"))
            .answering("https://waterdata.test/b", "{}");
        pollster::block_on(async {
            let driver = Driver::new(&host, now(), true).await;
            let usgs = Usgs::new();
            let first = driver
                .get(&usgs, &usgs_request("https://waterdata.test/a"))
                .await
                .expect("a response");
            assert_eq!(first.status, 429);
            let until = UnixTime(now().0 + 600);
            assert_eq!(
                driver.usage.snapshot().paused_until(&SourceId::USGS, now()),
                Some(until)
            );
            driver.save_usage().await.expect("saved");
            // A later fetch loads the pause with the ledger.
            let later = Driver::new(&host, UnixTime(now().0 + 60), true).await;
            let refused = later
                .get(&usgs, &usgs_request("https://waterdata.test/b"))
                .await
                .expect_err("paused");
            assert!(
                matches!(
                    refused,
                    SourceError::RateLimited {
                        retry_after_s: Some(540)
                    }
                ),
                "{refused:?}"
            );
            let after = Driver::new(&host, UnixTime(until.0 + 1), true).await;
            after
                .get(&usgs, &usgs_request("https://waterdata.test/b"))
                .await
                .expect("sent again");
        });
        assert_eq!(host.seen().len(), 2, "{:?}", host.seen());
    }

    #[test]
    fn a_reported_remaining_count_lowers_what_is_left_and_a_retry_after_is_honoured() {
        use underbite_providers::usgs::Usgs;

        let counted = Fetched {
            rate_limit_remaining: Some(7),
            ..Fetched::new(200, "{}")
        };
        let limited = Fetched {
            retry_after: Some(1_800),
            ..Fetched::new(429, "")
        };
        let host = MemoryHost::default()
            .answering_part("waterdata.test/count", counted)
            .answering_part("waterdata.test/limit", limited);
        pollster::block_on(async {
            let driver = Driver::new(&host, now(), true).await;
            let usgs = Usgs::new();
            driver
                .get(&usgs, &usgs_request("https://waterdata.test/count"))
                .await
                .expect("a response");
            let left = driver.usage.snapshot().remaining(
                &SourceId::USGS,
                usgs.budget().expect("budget"),
                now(),
            );
            assert_eq!(left, 7);
            driver
                .get(&usgs, &usgs_request("https://waterdata.test/limit"))
                .await
                .expect("a response");
            assert_eq!(
                driver.usage.snapshot().paused_until(&SourceId::USGS, now()),
                Some(UnixTime(now().0 + 1_800))
            );
        });
    }

    #[test]
    fn kept_responses_answer_again_only_while_keeping_and_never_fresh_calls() {
        let host = MemoryHost::default().answering("https://a.example/x", "x");
        pollster::block_on(async {
            let driver = Driver::new(&host, now(), true).await;
            let call = Call::get("https://a.example/x");
            driver
                .keeping(async { driver.send(&call).await.expect("sent") })
                .await;
            driver.send(&call).await.expect("kept");
            assert_eq!(host.seen().len(), 1);
            let fresh = call.clone().with_cache(CacheHint {
                fresh: true,
                ..CacheHint::default()
            });
            driver.send(&fresh).await.expect("sent");
            assert_eq!(host.seen().len(), 2);
        });
    }
}
