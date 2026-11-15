//! Replays recorded fixtures for a [`Session`]: a route table from URL substrings to files.
//!
//! Fixtures were recorded with dates and parameters that differ from what the sources plan for
//! the fixed `now`, so routes match on host, path and station rather than the full URL.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use underbite_core::datum::VerticalDatum;
use underbite_core::model::{Measurement, Requirements};
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef};
use underbite_core::time::UnixTime;
use underbite_providers::request::{Coordinates, Place, Response, Target, Window};
use underbite_providers::session::usage::{SharedUsage, UsageLedger};
use underbite_providers::session::{Chain, Link, Session, SessionOutcome};
use underbite_providers::source::{FetchContext, Source};

/// 2026-11-08T14:30:00Z, the moment the fixtures were recorded.
pub(crate) const NOW: UnixTime = UnixTime(1_794_148_200);

/// One route: every needle must occur in the URL; the answer is a status and a fixture.
pub(crate) struct Route {
    pub(crate) needles: &'static [&'static str],
    pub(crate) status: u16,
    pub(crate) dir: &'static str,
    pub(crate) file: &'static str,
}

/// A route answering 200.
pub(crate) const fn ok(
    needles: &'static [&'static str],
    dir: &'static str,
    file: &'static str,
) -> Route {
    Route {
        needles,
        status: 200,
        dir,
        file,
    }
}

/// A route answering with `status`.
pub(crate) const fn with_status(
    status: u16,
    needles: &'static [&'static str],
    dir: &'static str,
    file: &'static str,
) -> Route {
    Route {
        needles,
        status,
        dir,
        file,
    }
}

fn fixture(dir: &str, file: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/raw")
        .join(dir)
        .join(file);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

/// The context of every scenario.
pub(crate) fn ctx() -> FetchContext {
    FetchContext::new(NOW, "underbite-tests/0.1 (fixtures)")
}

/// Runs `session` to the end against `routes`. An unrouted URL fails the test. Returns the
/// outcome and the requests served (status and URL), in order.
pub(crate) fn replay(mut session: Session, routes: &[Route]) -> (SessionOutcome, Vec<String>) {
    let mut served = Vec::new();
    while let Some(request) = session.next_request() {
        let url = request.url.clone();
        let route = routes
            .iter()
            .find(|r| r.needles.iter().all(|n| url.contains(n)))
            .unwrap_or_else(|| panic!("no route for {url}"));
        let body = fixture(route.dir, route.file);
        served.push(format!("{} {} <- {}", route.status, url, route.file));
        session.on_response(&Response::new(route.status, &body));
    }
    (session.finish(NOW), served)
}

pub(crate) fn station(
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

fn window() -> Window {
    Window {
        from: UnixTime(NOW.0 - 7 * 86_400),
        to: UnixTime(NOW.0 + 7 * 86_400),
    }
}

/// A link to a station for `measurements`.
pub(crate) fn station_link(
    source: Arc<dyn Source>,
    station: StationRef,
    relation: Relation,
    measurements: &[Measurement],
) -> Link {
    Link {
        source,
        target: Target {
            place: Place::Station {
                station,
                distance_m: Some(1_000.0),
                relation,
            },
            measurements: measurements.iter().copied().collect(),
            window: window(),
            datum: Some(VerticalDatum::Mllw),
        },
    }
}

/// A link to a rounded point for `measurements`.
pub(crate) fn point_link(
    source: Arc<dyn Source>,
    at: (f64, f64),
    measurements: &[Measurement],
) -> Link {
    let point = ctx().round(Coordinates {
        latitude: at.0,
        longitude: at.1,
    });
    Link {
        source,
        target: Target {
            place: Place::Point(point),
            measurements: measurements.iter().copied().collect(),
            window: window(),
            datum: None,
        },
    }
}

pub(crate) fn chain(links: Vec<Link>, max_age_s: Option<i64>) -> Chain {
    Chain { links, max_age_s }
}

pub(crate) fn session(chains: BTreeMap<Measurement, Chain>, wanted: &[Measurement]) -> Session {
    let requirements = Requirements {
        measurements: wanted.iter().copied().collect(),
        past_hours: 24 * 7,
        future_days: 7,
    };
    Session::new(chains, requirements, ctx()).with_usage(SharedUsage::new(UsageLedger::new()))
}

/// Long arrays become `{len, first, last}` so snapshots stay readable; everything else, the
/// provenance of every series included, is kept.
pub(crate) fn condense(value: Value) -> Value {
    match value {
        Value::Array(items) if items.len() > 4 => json!({
            "len": items.len(),
            "first": condense(items[0].clone()),
            "last": condense(items[items.len() - 1].clone()),
        }),
        Value::Array(items) => Value::Array(items.into_iter().map(condense).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, condense(v))).collect())
        }
        other => other,
    }
}

fn to<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serialisable")
}

/// The merged outcome as condensed JSON, plus the requests served.
pub(crate) fn summary(outcome: &SessionOutcome, served: &[String]) -> Value {
    json!({
        "water": condense(to(&outcome.water)),
        "marine": condense(to(&outcome.marine)),
        "sources": to(&outcome.sources),
        "stations": to(&outcome.stations),
        "attribution": to(&outcome.attribution),
        "errors": to(&outcome.errors),
        "served": served,
    })
}
