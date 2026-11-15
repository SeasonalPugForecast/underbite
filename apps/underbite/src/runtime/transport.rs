//! HTTP transport over ureq, and replay of recorded responses.
//!
//! One [`Transport`] serves both Underbite's own [`Request`] and whether's [`HttpRequest`]
//! (the weather and storm sessions). Network I/O lives only here: the sessions build requests
//! and parse bytes. Every URL that reaches a message or a log goes through [`redact_url`].

#![allow(dead_code)] // The fetch driver is the only caller; it is wired in separately.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use underbite_core::time::UnixTime;
use underbite_providers::request::{
    CacheHint, Request, Response, is_secret_header, redact_url, secret_values,
};
use whether_core::time::parse_http_date;
use whether_providers::{HttpRequest, Method};

/// Largest response body accepted, after decompression.
pub(crate) const MAX_BODY: u64 = 8 * 1024 * 1024;

/// The User-Agent: `underbite/<version>`.
pub(crate) fn user_agent() -> String {
    format!("underbite/{}", env!("CARGO_PKG_VERSION"))
}

/// One HTTP call, built from either kind of request.
#[derive(Clone, PartialEq)]
pub(crate) struct Call {
    /// Full URL; may carry a key, so show it only through [`redact_url`].
    pub(crate) url: String,
    /// Extra headers.
    pub(crate) headers: Vec<(&'static str, String)>,
    /// GET, or POST with a JSON body.
    pub(crate) method: Method,
    /// The JSON body of a POST.
    pub(crate) body: Option<String>,
    /// How the response may be cached.
    pub(crate) cache: CacheHint,
}

impl Call {
    /// A GET of `url` with default caching.
    pub(crate) fn get(url: impl Into<String>) -> Self {
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
    pub(crate) fn with_cache(mut self, cache: CacheHint) -> Self {
        self.cache.fresh = cache.fresh;
        self.cache.no_store |= cache.no_store;
        self.cache.max_age = cache.max_age;
        self
    }

    /// Whether the response may be written to disk: never when the call carries a key.
    pub(crate) fn may_store_on_disk(&self) -> bool {
        !self.cache.no_store
    }

    /// The URL with secrets masked, for messages and logs.
    pub(crate) fn redacted_url(&self) -> String {
        redact_url(&self.url)
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
pub(crate) struct Fetched {
    /// Status code.
    pub(crate) status: u16,
    /// Body bytes, decompressed.
    pub(crate) body: Vec<u8>,
    /// The `Content-Type` header.
    pub(crate) content_type: Option<String>,
    /// The `Date` header.
    pub(crate) date: Option<UnixTime>,
    /// The `Expires` header.
    pub(crate) expires: Option<UnixTime>,
    /// `max-age` of `Cache-Control`, seconds.
    pub(crate) cache_max_age: Option<u32>,
    /// The `Last-Modified` header.
    pub(crate) last_modified: Option<String>,
    /// The `ETag` header.
    pub(crate) etag: Option<String>,
    /// `Retry-After`, seconds (a date is converted against the `Date` header).
    pub(crate) retry_after: Option<u32>,
    /// The `X-RateLimit-Remaining` header.
    pub(crate) rate_limit_remaining: Option<u32>,
}

impl Fetched {
    /// A response with just a status and body.
    pub(crate) fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            body: body.into(),
            ..Self::default()
        }
    }

    /// A borrowed view for the sources.
    pub(crate) fn as_response(&self) -> Response<'_> {
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TransportError {
    /// The request or connection timed out.
    Timeout(String),
    /// The body was larger than the cap.
    TooLarge(u64),
    /// DNS, TLS, connection or protocol failure.
    Failed(String),
    /// Replay has no route for this (redacted) URL.
    NoRoute(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout(which) => write!(f, "timed out: {which}"),
            Self::TooLarge(cap) => write!(f, "response larger than {cap} bytes"),
            Self::Failed(message) => f.write_str(message),
            Self::NoRoute(url) => write!(f, "no replay route for {url}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Performs HTTP calls. Implemented by [`UreqTransport`] and [`Replay`].
pub(crate) trait Transport: Send + Sync {
    /// Sends `call` and returns any status with its body.
    fn send(&self, call: &Call) -> Result<Fetched, TransportError>;
}

/// HTTPS with ureq (rustls, gzip).
#[derive(Debug)]
pub(crate) struct UreqTransport {
    agent: ureq::Agent,
    max_body: u64,
}

impl UreqTransport {
    /// A transport with the standard timeouts (30 s total, 10 s to connect) and body cap.
    pub(crate) fn new(user_agent: &str) -> Self {
        Self::with_limits(
            user_agent,
            Duration::from_secs(30),
            Duration::from_secs(10),
            MAX_BODY,
        )
    }

    /// A transport with custom limits (tests use small ones).
    pub(crate) fn with_limits(
        user_agent: &str,
        total: Duration,
        connect: Duration,
        max_body: u64,
    ) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(total))
            .timeout_connect(Some(connect))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            max_body,
        }
    }

    fn read_body(
        &self,
        resp: &mut ureq::http::Response<ureq::Body>,
    ) -> Result<Vec<u8>, ureq::Error> {
        // The limit on the body config counts wire bytes, before gzip is undone; the second
        // cap, on the reader, counts decoded bytes, so a small gzip body cannot inflate past it.
        let reader = resp.body_mut().with_config().limit(self.max_body).reader();
        let mut body = Vec::new();
        reader
            .take(self.max_body + 1)
            .read_to_end(&mut body)
            .map_err(ureq::Error::from)?;
        if body.len() as u64 > self.max_body {
            return Err(ureq::Error::BodyExceedsLimit(self.max_body));
        }
        Ok(body)
    }
}

/// `message` with the URL and every secret query value of it masked.
fn scrub(message: &str, url: &str) -> String {
    let mut out = message.replace(url, &redact_url(url));
    for secret in secret_values(url) {
        out = out.replace(secret, "***");
    }
    out
}

fn map_error(e: &ureq::Error, url: &str, cap: u64) -> TransportError {
    match e {
        ureq::Error::Timeout(which) => TransportError::Timeout(format!("{which:?}")),
        ureq::Error::BodyExceedsLimit(_) => TransportError::TooLarge(cap),
        other => TransportError::Failed(scrub(&other.to_string(), url)),
    }
}

fn header(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
}

/// `max-age=N` from a `Cache-Control` value.
fn max_age(value: &str) -> Option<u32> {
    value.split(',').find_map(|directive| {
        let (name, number) = directive.trim().split_once('=')?;
        if name.trim().eq_ignore_ascii_case("max-age") {
            number.trim().trim_matches('"').parse().ok()
        } else {
            None
        }
    })
}

/// `Retry-After` as seconds: a number, or a date relative to the response's `Date`.
fn retry_after(value: &str, date: Option<UnixTime>) -> Option<u32> {
    if let Ok(seconds) = value.parse::<u32>() {
        return Some(seconds);
    }
    let until = parse_http_date(value)?;
    let seconds = until.0 - date?.0;
    u32::try_from(seconds.max(0)).ok()
}

impl Transport for UreqTransport {
    fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        let sent = match call.method {
            Method::Get => {
                let mut request = self.agent.get(&call.url);
                for (name, value) in &call.headers {
                    request = request.header(*name, value);
                }
                request.call()
            }
            Method::Post => {
                let mut request = self.agent.post(&call.url).content_type("application/json");
                for (name, value) in &call.headers {
                    request = request.header(*name, value);
                }
                request.send(call.body.as_deref().unwrap_or(""))
            }
        };
        let mut resp = sent.map_err(|e| map_error(&e, &call.url, self.max_body))?;
        let date = header(&resp, "date").and_then(|v| parse_http_date(&v));
        let fetched = Fetched {
            status: resp.status().as_u16(),
            body: Vec::new(),
            content_type: header(&resp, "content-type"),
            date,
            expires: header(&resp, "expires").and_then(|v| parse_http_date(&v)),
            cache_max_age: header(&resp, "cache-control").and_then(|v| max_age(&v)),
            last_modified: header(&resp, "last-modified"),
            etag: header(&resp, "etag"),
            retry_after: header(&resp, "retry-after").and_then(|v| retry_after(&v, date)),
            rate_limit_remaining: header(&resp, "x-ratelimit-remaining")
                .and_then(|v| v.parse().ok()),
        };
        let body = self
            .read_body(&mut resp)
            .map_err(|e| map_error(&e, &call.url, self.max_body))?;
        Ok(Fetched { body, ..fetched })
    }
}

/// One entry of a routes file: a URL substring and the recorded response it answers with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Route {
    /// Substring of the URL (host, path and a distinguishing parameter).
    pub(crate) matches: String,
    /// The recorded body, relative to the routes file.
    pub(crate) file: PathBuf,
    /// The status to answer with.
    pub(crate) status: u16,
    /// The `Content-Type` to answer with.
    pub(crate) content_type: String,
}

/// Why a routes file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoutesError(String);

impl fmt::Display for RoutesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RoutesError {}

/// A transport that answers from `<dir>/routes.toml` (first matching route wins) and records
/// the redacted URLs it was asked for. An unmatched URL is an error, so a missing route fails
/// loudly.
#[derive(Debug)]
pub(crate) struct Replay {
    base: PathBuf,
    routes: Vec<Route>,
    seen: Mutex<Vec<String>>,
}

impl Replay {
    /// Loads `<dir>/routes.toml`.
    pub(crate) fn load(dir: &Path) -> Result<Self, RoutesError> {
        let path = dir.join("routes.toml");
        let text = fs::read_to_string(&path)
            .map_err(|e| RoutesError(format!("cannot read {}: {e}", path.display())))?;
        let routes =
            parse_routes(&text).map_err(|e| RoutesError(format!("{}: {}", path.display(), e.0)))?;
        Ok(Self::new(dir, routes))
    }

    /// A replay over `routes`, with fixture files relative to `base`.
    pub(crate) fn new(base: &Path, routes: Vec<Route>) -> Self {
        Self {
            base: base.to_path_buf(),
            routes,
            seen: Mutex::new(Vec::new()),
        }
    }

    /// Every URL requested so far (redacted), in order.
    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Transport for Replay {
    fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        let redacted = call.redacted_url();
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(redacted.clone());
        }
        let route = self
            .routes
            .iter()
            .find(|route| call.url.contains(&route.matches))
            .ok_or_else(|| TransportError::NoRoute(redacted.clone()))?;
        let path = self.base.join(&route.file);
        let body = fs::read(&path).map_err(|e| {
            TransportError::Failed(format!(
                "replay file {} for {redacted}: {e}",
                path.display()
            ))
        })?;
        let mut fetched = Fetched::new(route.status, body);
        fetched.content_type = Some(route.content_type.clone());
        Ok(fetched)
    }
}

/// The content type a route gets when it names none, from its file's extension.
fn guess_content_type(file: &Path) -> &'static str {
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

#[derive(Default)]
struct Draft {
    matches: Option<String>,
    file: Option<String>,
    status: Option<u16>,
    content_type: Option<String>,
    line: usize,
}

impl Draft {
    fn finish(self) -> Result<Route, RoutesError> {
        let missing =
            |what: &str| RoutesError(format!("route at line {}: missing {what}", self.line));
        let file = PathBuf::from(self.file.clone().ok_or_else(|| missing("file"))?);
        Ok(Route {
            matches: self.matches.clone().ok_or_else(|| missing("match"))?,
            content_type: self
                .content_type
                .clone()
                .unwrap_or_else(|| guess_content_type(&file).to_string()),
            file,
            status: self.status.unwrap_or(200),
        })
    }
}

/// Parses the `[[route]]` tables of a routes file: `match`, `file` and optionally `status`
/// and `content_type`. Only this shape is accepted; anything else is an error.
fn parse_routes(text: &str) -> Result<Vec<Route>, RoutesError> {
    let mut routes = Vec::new();
    let mut draft: Option<Draft> = None;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let bad = |what: &str| RoutesError(format!("line {line_no}: {what}"));
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[route]]" {
            if let Some(done) = draft.take() {
                routes.push(done.finish()?);
            }
            draft = Some(Draft {
                line: line_no,
                ..Draft::default()
            });
            continue;
        }
        let current = draft.as_mut().ok_or_else(|| bad("expected [[route]]"))?;
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| bad("expected key = value"))?;
        let (key, value) = (key.trim(), value.trim());
        if key == "status" {
            current.status = Some(value.parse().map_err(|_| bad("status is not a number"))?);
            continue;
        }
        let text = parse_string(value).ok_or_else(|| bad("expected a quoted string"))?;
        match key {
            "match" => current.matches = Some(text),
            "file" => current.file = Some(text),
            "content_type" => current.content_type = Some(text),
            other => return Err(bad(&format!("unknown key {other}"))),
        }
    }
    if let Some(done) = draft {
        routes.push(done.finish()?);
    }
    Ok(routes)
}

/// A double-quoted string with `\"` and `\\` escapes, then nothing but a comment.
fn parse_string(value: &str) -> Option<String> {
    let mut chars = value.strip_prefix('"')?.chars();
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => break,
            '\\' => out.push(match chars.next()? {
                '"' => '"',
                '\\' => '\\',
                _ => return None,
            }),
            c => out.push(c),
        }
    }
    let rest = chars.as_str().trim();
    (rest.is_empty() || rest.starts_with('#')).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;
    use underbite_providers::request::{
        Coordinates, Place, Resource, RoundedPoint, Target, Window,
    };

    /// Serves one canned response to the next connection; returns the base URL.
    fn serve(response: Vec<u8>) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).expect("read");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let _ = stream.write_all(&response);
            String::from_utf8_lossy(&request).into_owned()
        });
        (url, handle)
    }

    fn http(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {status}\r\n").into_bytes();
        for (name, value) in headers {
            out.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(body);
        out
    }

    fn transport(max_body: u64) -> UreqTransport {
        UreqTransport::with_limits(
            "underbite/test",
            Duration::from_secs(5),
            Duration::from_secs(2),
            max_body,
        )
    }

    #[test]
    fn captures_status_body_and_headers() {
        let reply = http(
            "429 Too Many Requests",
            &[
                ("Content-Type", "application/json; charset=utf-8"),
                ("Date", "Sun, 15 Nov 2026 12:00:00 GMT"),
                ("Expires", "Sun, 15 Nov 2026 12:10:00 GMT"),
                ("Cache-Control", "public, max-age=300"),
                ("Last-Modified", "Sun, 15 Nov 2026 11:55:00 GMT"),
                ("ETag", "\"abc\""),
                ("Retry-After", "Sun, 15 Nov 2026 12:00:42 GMT"),
                ("X-RateLimit-Remaining", "7"),
            ],
            b"{\"error\":1}",
        );
        let (url, server) = serve(reply);
        let got = transport(1024)
            .send(&Call::get(format!("{url}/x?a=1")))
            .expect("sent");
        let seen = server.join().expect("server");
        assert!(
            seen.to_ascii_lowercase()
                .contains("user-agent: underbite/test"),
            "{seen}"
        );
        assert_eq!(got.status, 429);
        assert_eq!(got.body, b"{\"error\":1}");
        assert_eq!(
            got.content_type.as_deref(),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(got.date, Some(UnixTime(1_794_744_000)));
        assert_eq!(got.expires, Some(UnixTime(1_794_744_600)));
        assert_eq!(got.cache_max_age, Some(300));
        assert_eq!(
            got.last_modified.as_deref(),
            Some("Sun, 15 Nov 2026 11:55:00 GMT")
        );
        assert_eq!(got.etag.as_deref(), Some("\"abc\""));
        assert_eq!(got.retry_after, Some(42));
        assert_eq!(got.rate_limit_remaining, Some(7));
        assert_eq!(got.as_response().status, 429);
    }

    #[test]
    fn sends_request_headers() {
        let (url, server) = serve(http("200 OK", &[], b"ok"));
        let call = Call {
            headers: vec![("Accept", "application/geo+json".to_string())],
            ..Call::get(format!("{url}/x"))
        };
        transport(1024).send(&call).expect("sent");
        let seen = server.join().expect("server").to_ascii_lowercase();
        assert!(seen.contains("accept: application/geo+json"), "{seen}");
    }

    #[test]
    fn a_body_over_the_cap_is_an_error() {
        let (url, server) = serve(http("200 OK", &[], &[b'x'; 2000]));
        let err = transport(1000)
            .send(&Call::get(format!("{url}/big")))
            .unwrap_err();
        let _ = server.join();
        assert_eq!(err, TransportError::TooLarge(1000));
    }

    #[test]
    fn a_body_at_the_cap_is_accepted() {
        let (url, server) = serve(http("200 OK", &[], &[b'x'; 1000]));
        let got = transport(1000)
            .send(&Call::get(format!("{url}/big")))
            .expect("sent");
        let _ = server.join();
        assert_eq!(got.body.len(), 1000);
    }

    #[test]
    fn errors_never_carry_keys() {
        // Nothing listens on port 9 of the loopback address: the connection is refused.
        let call = Call::get("http://127.0.0.1:9/x?api_key=SECRET-VALUE&a=1");
        let err = transport(1000).send(&call).unwrap_err();
        let text = err.to_string();
        assert!(!text.contains("SECRET-VALUE"), "{text}");
    }

    #[test]
    fn scrub_masks_urls_and_keys() {
        let url = "https://api.example/x?lat=1&api_key=SECRET";
        let scrubbed = scrub(&format!("failed to fetch {url}: bad SECRET"), url);
        assert!(!scrubbed.contains("SECRET"), "{scrubbed}");
        assert!(scrubbed.contains("api_key=***"), "{scrubbed}");
    }

    #[test]
    fn parses_cache_control_and_retry_after() {
        assert_eq!(max_age("public, Max-Age=60, must-revalidate"), Some(60));
        assert_eq!(max_age("no-store"), None);
        assert_eq!(retry_after("120", None), Some(120));
        assert_eq!(retry_after("Sun, 15 Nov 2026 12:00:42 GMT", None), None);
    }

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

    const ROUTES: &str = r#"
# comment
[[route]]
match = "datagetter?product=water_level"
file = "water.json"

[[route]]
match = "ndbc.noaa.gov/data/realtime2/44025.txt"   # a text file
file = "ndbc.txt"
status = 404
content_type = "text/plain"
"#;

    #[test]
    fn replay_answers_by_substring_and_fails_loudly() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("routes.toml"), ROUTES).expect("write");
        fs::write(dir.path().join("water.json"), "{}").expect("write");
        fs::write(dir.path().join("ndbc.txt"), "gone").expect("write");
        let replay = Replay::load(dir.path()).expect("load");

        let hit = replay
            .send(&Call::get(
                "https://api.example/datagetter?product=water_level&station=1",
            ))
            .expect("route");
        assert_eq!(
            (hit.status, hit.body.as_slice(), hit.content_type.as_deref()),
            (200, &b"{}"[..], Some("application/json"))
        );
        let miss = replay
            .send(&Call::get(
                "https://www.ndbc.noaa.gov/data/realtime2/44025.txt",
            ))
            .expect("route");
        assert_eq!(
            (miss.status, miss.content_type.as_deref()),
            (404, Some("text/plain"))
        );

        let err = replay
            .send(&Call::get("https://other.test/y?api_key=SECRET"))
            .unwrap_err();
        assert!(matches!(err, TransportError::NoRoute(_)));
        assert!(!err.to_string().contains("SECRET"), "{err}");
        assert!(err.to_string().contains("other.test/y"));
        assert_eq!(replay.seen().len(), 3);
        assert!(replay.seen().iter().all(|u| !u.contains("SECRET")));
    }

    #[test]
    fn replay_reports_a_missing_fixture_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let routes =
            parse_routes("[[route]]\nmatch = \"a\"\nfile = \"gone.json\"\n").expect("parse");
        let err = Replay::new(dir.path(), routes)
            .send(&Call::get("https://a.test/"))
            .unwrap_err();
        assert!(matches!(err, TransportError::Failed(_)), "{err}");
    }

    #[test]
    fn the_shipped_replay_directories_load_and_point_at_recorded_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/replay");
        for scenario in ["battery", "croton"] {
            let dir = root.join(scenario);
            let replay = Replay::load(&dir).expect("routes load");
            assert!(!replay.routes.is_empty(), "{scenario}");
            for route in &replay.routes {
                assert!(
                    dir.join(&route.file).is_file(),
                    "{scenario}: {:?}",
                    route.file
                );
            }
        }
        let replay = Replay::load(&root.join("battery")).expect("load");
        let got = replay
            .send(&Call::get(
                "https://www.ndbc.noaa.gov/data/realtime2/44025.txt",
            ))
            .expect("route");
        assert_eq!(got.status, 200);
        assert!(got.body.len() > 100);
    }

    #[test]
    fn routes_files_are_checked() {
        assert!(parse_routes("match = \"a\"").is_err());
        assert!(parse_routes("[[route]]\nmatch = \"a\"").is_err());
        assert!(parse_routes("[[route]]\nmatch = a\nfile = \"b\"").is_err());
        assert!(parse_routes("[[route]]\nmatch = \"a\"\nfile = \"b\"\nfoo = \"1\"").is_err());
        let routes =
            parse_routes("[[route]]\nmatch = \"a\\\"b\"\nfile = \"x.png\"").expect("parse");
        assert_eq!(routes[0].matches, "a\"b");
        assert_eq!(routes[0].content_type, "image/png");
    }
}
