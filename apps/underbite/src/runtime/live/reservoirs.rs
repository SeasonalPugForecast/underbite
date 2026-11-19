//! Reservoirs (CWMS, RISE) and blooms (`CyAN`, NOAA's Gulf forecast), at the projects and lakes
//! the recorded fixtures use.

use underbite_core::model::Measurement as M;
use underbite_core::provenance::{SourceId, StationKind};
use underbite_core::time::UnixTime;
use underbite_providers::blooms::cyan::{self, Frequency, request_url};
use underbite_providers::blooms::hab::HabGulf;
use underbite_providers::request::Coordinates;
use underbite_providers::reservoirs::cwms::Cwms;
use underbite_providers::reservoirs::rise::Rise;
use underbite_providers::source::SourceError;

use super::{
    GAP, ctx, expect, fail, get, point_target, requirements, run, station, station_target,
    transport,
};

/// USACE CWMS Data API: pool elevation, release and inflow of Raystown Lake (NAB).
#[test]
#[ignore = "live: needs the network"]
fn live_cwms_raystown() {
    let wanted = [M::LakeLevel, M::Reservoir];
    let raystown = station(
        SourceId::CWMS,
        "NAB/Raystown",
        "Raystown Lake",
        StationKind::WaterLevel,
        (40.42, -78.09),
        None,
    );
    let target = station_target(raystown, &wanted, None);
    let found = run(&Cwms, &target, &requirements(&wanted), 3);
    found.expect_all("CWMS at Raystown Lake", &[M::LakeLevel]);
}

/// Reclamation's RISE API: pool elevation and release of Flaming Gorge Reservoir (location
/// 1535).
#[test]
#[ignore = "live: needs the network"]
fn live_rise_flaming_gorge() {
    let wanted = [M::LakeLevel, M::Reservoir];
    let flaming = station(
        SourceId::RISE,
        "1535",
        "Flaming Gorge Reservoir",
        StationKind::WaterLevel,
        (40.98, -109.57),
        None,
    );
    let target = station_target(flaming, &wanted, None);
    let found = run(&Rise, &target, &requirements(&wanted), 3);
    found.expect_all("RISE at Flaming Gorge", &[M::LakeLevel]);
}

/// EPA `CyAN`: the weekly and daily composites of Oneida Lake and the weekly composites at the
/// Battery's named estuary location parse. Old composites (a lake out of season) are not a
/// failure; a changed shape is.
#[test]
#[ignore = "live: needs the network"]
fn live_cyan_locations() {
    let transport = transport();
    let now = ctx().now;
    let round = |lat: f64, lon: f64| {
        ctx().round(Coordinates {
            latitude: lat,
            longitude: lon,
        })
    };
    let checks = [
        (round(43.20, -76.00), Frequency::Weekly, true),
        (round(43.20, -76.00), Frequency::Daily, true),
        (round(40.70, -74.02), Frequency::Weekly, false),
    ];
    for (n, (point, frequency, must_know)) in checks.into_iter().enumerate() {
        if n > 0 {
            std::thread::sleep(GAP);
        }
        let url = request_url(point, frequency);
        let fetched = get(&transport, &url);
        match cyan::parse(&fetched.body, UnixTime(now.0)) {
            Ok(_) | Err(SourceError::Stale { .. }) => {}
            Err(SourceError::NoData(why)) if !must_know => {
                eprintln!(
                    "{}: no data ({why:?})",
                    underbite_providers::request::redact_url(&url)
                );
            }
            Err(e) => fail(&url, format!("the CyAN answer no longer parses: {e}")),
        }
    }
}

/// NOAA NCCOS Gulf respiratory irritation forecast: the current model run's zip parses for a
/// beach in Pinellas County. A run older than the card shows is not a failure.
#[test]
#[ignore = "live: needs the network"]
fn live_noaa_hab_gulf_forecast() {
    let wanted = [M::Bloom];
    let target = point_target((27.978, -82.83), &wanted);
    let found = run(&HabGulf, &target, &requirements(&wanted), 1);
    expect(
        "https://nccospublicstor.blob.core.windows.net/hab-data/no_Explorer/rif_model/csv/RIFv2_currentmodelrun.zip",
        found.has(M::Bloom) || !found.quiet.is_empty(),
        "the Gulf forecast gave neither a bloom forecast nor a quiet outcome",
    );
    for note in &found.quiet {
        eprintln!("{note}");
    }
}
