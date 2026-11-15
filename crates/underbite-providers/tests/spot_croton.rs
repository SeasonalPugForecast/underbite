//! The Croton River (USGS 01375000) through a session, from recorded fixtures.
//!
//! The Croton has no NWPS gauge, so the flood categories and flow forecast come from a recorded
//! gauge of the same state (COHN6, Mohawk River at Cohoes), far away: the test is about the
//! session's merging, not hydrology.

mod replay;

use std::collections::BTreeMap;
use std::sync::Arc;

use replay::{Route, chain, ok, station, station_link};
use underbite_core::model::Measurement as M;
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef, UsgsSiteType};
use underbite_core::time::{UnixTime, UtcOffset};
use underbite_providers::nwps::Nwps;
use underbite_providers::session::Chain;
use underbite_providers::source::{Fragments, Source};
use underbite_providers::usgs::Usgs;
use underbite_providers::usgs::statistics::Reading;

fn croton() -> StationRef {
    station(
        SourceId::USGS,
        "USGS-01375000",
        "Croton River at New Croton Dam near Croton-on-Hudson NY",
        StationKind::UsgsSite(UsgsSiteType::Stream),
        (41.2247, -73.8592),
        None,
    )
}

fn cohoes() -> StationRef {
    station(
        SourceId::NWPS,
        "COHN6",
        "Mohawk River at Cohoes NY",
        StationKind::NwpsGauge,
        (42.7733, -73.7078),
        None,
    )
}

/// The chains of the scenario. Without `history`, USGS is asked for the newest values only
/// (the window ends within the hour of `now`); with it, for the last week too.
fn chains(history: bool) -> BTreeMap<M, Chain> {
    let mut chains = BTreeMap::new();
    for m in [M::Discharge, M::GageHeight] {
        chains.insert(m, {
            let mut link = station_link(
                Arc::new(Usgs::new()),
                croton(),
                Relation::SameWaterBody,
                &[m],
            );
            if !history {
                link.target.window.from = UnixTime(replay::NOW.0 - 1_800);
            }
            chain(vec![link], None)
        });
    }
    for m in [M::FloodCategories, M::FlowForecast] {
        let mut link = station_link(Arc::new(Nwps), cohoes(), Relation::Nearby, &[m]);
        if let underbite_providers::request::Place::Station { distance_m, .. } =
            &mut link.target.place
        {
            *distance_m = Some(163_000.0);
        }
        chains.insert(m, chain(vec![link], None));
    }
    chains
}

const WANTED: [M; 4] = [
    M::Discharge,
    M::GageHeight,
    M::FloodCategories,
    M::FlowForecast,
];

fn routes() -> Vec<Route> {
    vec![
        ok(
            &["api.waterdata.usgs.gov/ogcapi/v1/collections/latest-continuous/items"],
            "usgs",
            "usgs-latest-continuous-croton-01375000.json",
        ),
        ok(
            &[
                "api.waterdata.usgs.gov/ogcapi/v1/collections/continuous/items",
                "time_series_id=e32aab5ccc774c11b172102b97abc0d8",
            ],
            "usgs",
            "usgs-continuous-croton-01375000-00060-2026-11-01_2026-11-08.json",
        ),
        ok(
            &["api.water.noaa.gov/nwps/v1/gauges/COHN6/stageflow/forecast"],
            "nwps",
            "nwps-gauge-COHN6-stageflow-forecast.json",
        ),
        ok(
            &["api.water.noaa.gov/nwps/v1/gauges/COHN6"],
            "nwps",
            "nwps-gauge-COHN6.json",
        ),
    ]
}

#[test]
fn croton_spot_through_the_session() {
    let session = replay::session(chains(false), &WANTED);
    let (outcome, served) = replay::replay(session, &routes());
    insta::assert_json_snapshot!("croton", replay::summary(&outcome, &served));
}

/// With the week's history the discharge series is the continuous record, not the newest value.
#[test]
fn croton_discharge_with_a_week_of_history() {
    let session = replay::session(chains(true), &[M::Discharge]);
    let (outcome, served) = replay::replay(session, &routes());
    insta::assert_json_snapshot!("croton_history", replay::summary(&outcome, &served));
}

/// The session never plans USGS Statistics (the caller places the reading among the normals),
/// so the percentile for 8 November is driven by hand from the session's newest discharge.
#[test]
fn croton_percentile_for_the_day_from_the_newest_discharge() {
    let session = replay::session(chains(true), &[M::Discharge]);
    let (outcome, _) = replay::replay(session, &routes());
    let discharge = outcome
        .water
        .discharge_m3s
        .expect("discharge from the session");
    let newest = discharge.samples().last().expect("a sample");
    let reading = Reading {
        measurement: M::Discharge,
        datum: None,
        value: newest.value,
        // The week's record ends at midnight UTC; the reading is placed at the fetch time.
        at: replay::NOW,
    };
    let target = chains(true)[&M::Discharge].links[0].target.clone();
    let ctx = replay::ctx();
    let request = Usgs::new()
        .statistics_request(&target, &reading, UtcOffset(-5 * 3_600), &ctx)
        .expect("a statistics request");
    assert!(request.url.contains("start_date=11-08"), "{}", request.url);
    let body = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/raw/usgs/usgs-statistics-normals-croton-01375000-00060-doy-percentile-11-05_11-10.json",
        ),
    )
    .expect("fixture");
    let mut out = Fragments::new();
    Usgs::new()
        .parse(
            &request,
            &underbite_providers::request::Response::new(200, &body),
            &ctx,
            &mut out,
        )
        .expect("parses");
    insta::assert_json_snapshot!(
        "croton_percentile",
        serde_json::json!({
            "request": request.url,
            "fragments": out.fragments().iter().map(|f| format!("{f:?}")).collect::<Vec<_>>(),
        })
    );
}
