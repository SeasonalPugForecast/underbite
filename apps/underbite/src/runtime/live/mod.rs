//! Live checks of every data source, run by hand.
//!
//! Each test sends a few real requests through the CLI's own transport (the same HTTP client,
//! rounding and User-Agent the app uses) and checks that the answer still parses with today's
//! parser: the shape and the fields the cards read. A transport failure, an HTTP error or a
//! parse failure fails the test with a message that names the URL, through `redact_url`.
//!
//! They are `#[ignore]` and named `live_*`, so `cargo xtask ci` never runs them. Run the whole
//! set once, one test at a time (some sources rate-limit quickly):
//!
//! ```text
//! cargo xtask remote run -- cargo test --workspace -- --ignored live_ --test-threads=1
//! ```
//!
//! No key is used: USGS runs anonymously and Copernicus is asked for its public capabilities
//! only. Test places are The Battery (CO-OPS 8518750), NDBC 44025, the lower Hudson and the
//! Croton River USGS site; the Canadian, reservoir, bloom and species checks use the stations
//! and areas the recorded fixtures use.

mod canada;
mod maps;
mod nearby;
mod noaa;
mod reservoirs;
mod usgs;
mod weather;

use std::collections::BTreeSet;
use std::fmt::Display;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use underbite_config::schema::Config;
use underbite_core::datum::VerticalDatum;
use underbite_core::model::{Measurement, Requirements};
use underbite_core::provenance::{SourceId, StationKind, StationRef};
use underbite_core::time::UnixTime;
use underbite_fetch::{Call, Fetched};
use underbite_providers::request::{
    Coordinates, Place, Request, Response, Target, Window, redact_url,
};
use underbite_providers::source::{FetchContext, Fragment, Fragments, Source, SourceError};

use super::transport::{Transport, UreqTransport, user_agent};

/// The Battery, New York Harbor, rounded as the app rounds a spot.
pub(super) const BATTERY: (f64, f64) = (40.7033, -74.0170);
/// The Croton River spot the replays use.
pub(super) const CROTON: (f64, f64) = (41.2250, -73.8589);
/// The lower Hudson at Piermont, the middle of the lower Hudson box.
pub(super) const LOWER_HUDSON: (f64, f64) = (41.04, -73.90);

/// The pause between two requests to one host.
pub(super) const GAP: Duration = Duration::from_secs(2);

/// The current time.
pub(super) fn now() -> UnixTime {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    UnixTime(i64::try_from(secs).unwrap_or(0))
}

/// The CLI's transport with the app's User-Agent (`underbite/<version>`).
pub(super) fn transport() -> UreqTransport {
    UreqTransport::new(&user_agent())
}

/// The context of every check: the configured default precision and the app's User-Agent.
pub(super) fn ctx() -> FetchContext {
    FetchContext::new(now(), user_agent())
        .with_coordinate_precision(Config::default().sources.coordinate_precision)
}

/// Sleeps `secs` seconds, before a request to a service that asks for care.
pub(super) fn sleep_s(secs: u64) {
    std::thread::sleep(Duration::from_secs(secs));
}

/// A short, single-line start of a body for a failure message.
fn snippet(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(&body[..body.len().min(160)]);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fails the test naming `url` (redacted) and `what`.
#[track_caller]
pub(super) fn fail(url: &str, what: impl Display) -> ! {
    panic!("live check failed: {what}\n  url: {}", redact_url(url));
}

/// Sends `call` and returns the answer of any status; a transport failure fails the test.
#[track_caller]
pub(super) fn send(transport: &UreqTransport, call: &Call) -> Fetched {
    match transport.send(call) {
        Ok(fetched) => fetched,
        Err(e) => fail(&call.url, format!("no response ({e})")),
    }
}

/// GETs `url` with `transport`; anything but a 2xx fails the test with the status and the start
/// of the body.
#[track_caller]
pub(super) fn get(transport: &UreqTransport, url: &str) -> Fetched {
    get_call(transport, &Call::get(url))
}

/// Like [`get`] for a prepared call.
#[track_caller]
pub(super) fn get_call(transport: &UreqTransport, call: &Call) -> Fetched {
    let fetched = send(transport, call);
    if !(200..300).contains(&fetched.status) {
        fail(
            &call.url,
            format!("HTTP {} ({})", fetched.status, snippet(&fetched.body)),
        );
    }
    fetched
}

/// Unwraps `result`, failing with `what` and the error text and the URL.
#[track_caller]
pub(super) fn parsed<T, E: Display>(url: &str, what: &str, result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(e) => fail(url, format!("{what} no longer parses: {e}")),
    }
}

/// Fails unless `ok`, with `what` and the URL.
#[track_caller]
pub(super) fn expect(url: &str, ok: bool, what: impl Display) {
    if !ok {
        fail(url, what);
    }
}

/// A station of `source` as the fetch library would hold it.
pub(super) fn station(
    source: SourceId,
    id: &str,
    name: &str,
    kind: StationKind,
    at: (f64, f64),
    bin: Option<u16>,
) -> StationRef {
    StationRef {
        source,
        id: id.into(),
        name: name.into(),
        kind,
        latitude: at.0,
        longitude: at.1,
        bin,
        reference_id: None,
    }
}

/// What the checks ask for: two days back, two ahead.
pub(super) fn requirements(measurements: &[Measurement]) -> Requirements {
    Requirements {
        measurements: measurements.iter().copied().collect::<BTreeSet<_>>(),
        past_hours: 48,
        future_days: 2,
    }
}

/// A target at `station` for `measurements`.
pub(super) fn station_target(
    station: StationRef,
    measurements: &[Measurement],
    datum: Option<VerticalDatum>,
) -> Target {
    let now = now();
    Target {
        place: Place::Station {
            station,
            distance_m: Some(1_000.0),
            relation: underbite_core::provenance::Relation::SameWaterBody,
        },
        measurements: measurements.iter().copied().collect(),
        window: Window::around(now, &requirements(measurements)),
        datum,
    }
}

/// A target at a point rounded as the app rounds it.
pub(super) fn point_target(at: (f64, f64), measurements: &[Measurement]) -> Target {
    let ctx = ctx();
    Target {
        place: Place::Point(ctx.round(Coordinates {
            latitude: at.0,
            longitude: at.1,
        })),
        measurements: measurements.iter().copied().collect(),
        window: Window::around(ctx.now, &requirements(measurements)),
        datum: None,
    }
}

/// What [`run`] found.
#[derive(Debug, Default)]
pub(super) struct Run {
    /// Every fragment the responses parsed into.
    pub(super) fragments: Vec<Fragment>,
    /// Redacted URLs of the requests sent.
    pub(super) urls: Vec<String>,
    /// Requests the source answered with no data or only old data: not a shape change, shown
    /// for the record.
    pub(super) quiet: Vec<String>,
}

impl Run {
    /// Whether some fragment is for `measurement`.
    pub(super) fn has(&self, measurement: Measurement) -> bool {
        self.fragments.iter().any(|f| f.measurement == measurement)
    }

    /// Fails unless every measurement of `wanted` has a fragment.
    #[track_caller]
    pub(super) fn expect_all(&self, source: &str, wanted: &[Measurement]) {
        let missing: Vec<_> = wanted.iter().filter(|m| !self.has(**m)).collect();
        assert!(
            missing.is_empty(),
            "live check failed: {source} gave no {missing:?}\n  asked: {:?}\n  no data: {:?}\n  got: {:?}",
            self.urls,
            self.quiet,
            self.fragments
                .iter()
                .map(|f| f.measurement)
                .collect::<Vec<_>>(),
        );
    }
}

/// Plans `target` with `source`, sends up to `max` requests (the plan, then the follow-ups),
/// and parses each answer with the source. A response that is not data (a 404 for a missing
/// product, no values, an old value) is recorded in [`Run::quiet`]; any other failure fails the
/// test naming the request's URL.
#[track_caller]
pub(super) fn run(
    source: &dyn Source,
    target: &Target,
    requirements: &Requirements,
    max: usize,
) -> Run {
    let transport = transport();
    let ctx = ctx();
    let planned = match source.plan(target, requirements, &ctx) {
        Ok(planned) => planned,
        Err(e) => panic!(
            "live check failed: {} cannot plan the request: {e}",
            source.id()
        ),
    };
    let mut queue: std::collections::VecDeque<Request> = planned.into();
    let mut out = Run::default();
    let mut sent = 0;
    while let Some(request) = queue.pop_front() {
        if sent >= max {
            break;
        }
        if sent > 0 {
            std::thread::sleep(GAP);
        }
        sent += 1;
        let call = Call::from(&request);
        let fetched = send(&transport, &call);
        let response: Response<'_> = fetched.as_response();
        out.urls.push(call.redacted_url());
        let mut fragments = Fragments::new();
        match source.parse(&request, &response, &ctx, &mut fragments) {
            Ok(()) => out.fragments.extend(fragments.into_parts().0),
            Err(e @ (SourceError::NoData(_) | SourceError::Stale { .. })) => {
                out.quiet.push(format!("{}: {e}", call.redacted_url()));
            }
            Err(e) => fail(
                &request.url,
                format!(
                    "{} answered {} but its parser said: {e}",
                    source.id(),
                    fetched.status
                ),
            ),
        }
        match source.follow_up(&request, &response, &ctx) {
            Ok(next) => queue.extend(next),
            Err(SourceError::NoData(_) | SourceError::Stale { .. }) => {}
            Err(e) => fail(&request.url, format!("{} follow-up: {e}", source.id())),
        }
    }
    out
}
