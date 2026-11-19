//! Canadian sources: DFO's Integrated Water Level System and ECCC's hydrometric service.
//!
//! The DFO fixtures are invented (the service's licence keeps recorded data out of the
//! repository), so these checks use the stations the provider notes name: Vancouver (07735,
//! tidal) and Toronto (13320, a Great Lakes gauge with an IGLD 85 offset). ECCC uses the Bow
//! River at Calgary (05BH004), the station its fixtures record.

use underbite_core::map::Bbox;
use underbite_core::model::Measurement as M;
use underbite_core::provenance::{SourceId, StationKind, StationRef};
use underbite_fetch::Call;
use underbite_providers::dfo::{
    Dfo, Region, metadata_url, parse_metadata, parse_stations, station_by_code_url, stations_url,
};
use underbite_providers::eccc::{self, Eccc, stations_request as eccc_stations_url};
use underbite_providers::request::{Request, Resource};
use underbite_providers::source::{Fragments, Source};

use super::{
    GAP, ctx, expect, fail, get_call, parsed, point_target, requirements, run, station_target,
    transport,
};

/// The IWLS station of `code` (a five-digit string) with the id its data requests need.
fn iwls_station(code: &str) -> StationRef {
    let transport = transport();
    let url = station_by_code_url(code);
    let target = point_target((49.3, -123.2), &[M::Tide]);
    let request = Request::new(url.clone(), Resource::DfoStations, target);
    let fetched = get_call(&transport, &Call::from(&request));
    let stations = parsed(
        &url,
        "the IWLS station record",
        parse_stations(&request, &fetched.as_response()),
    );
    let Some(found) = stations.iter().find(|s| s.code == code) else {
        fail(
            &url,
            format!("IWLS has no station {code} (it listed {})", stations.len()),
        );
    };
    found.station_ref()
}

/// IWLS: the Pacific station list, then the tide of Vancouver (high and low water, predicted
/// and observed levels) and its metadata.
#[test]
#[ignore = "live: needs the network"]
fn live_dfo_iwls_vancouver() {
    let transport = transport();
    let target = point_target((49.3, -123.2), &[M::Tide]);
    let url = stations_url(Some(Region::Pacific));
    let request = Request::new(url.clone(), Resource::DfoStations, target);
    let fetched = get_call(&transport, &Call::from(&request));
    let list = parsed(
        &url,
        "the IWLS Pacific station list",
        parse_stations(&request, &fetched.as_response()),
    );
    expect(
        &url,
        list.iter().any(|s| s.code == "07735"),
        format!("the Pacific list has {} stations but not 07735", list.len()),
    );

    std::thread::sleep(GAP);
    let vancouver = list
        .into_iter()
        .find(|s| s.code == "07735")
        .map(|s| s.station_ref())
        .expect("checked above");
    let wanted = [M::Tide];
    let target = station_target(vancouver.clone(), &wanted, None);
    let found = run(&Dfo, &target, &requirements(&wanted), 3);
    found.expect_all("IWLS tide at Vancouver 07735", &wanted);

    std::thread::sleep(GAP);
    let url = metadata_url(&vancouver.id);
    let request = Request::new(url.clone(), Resource::DfoMetadata, target);
    let fetched = get_call(&transport, &Call::from(&request));
    let metadata = parsed(
        &url,
        "the IWLS metadata",
        parse_metadata(&request, &fetched.as_response()),
    );
    expect(
        &url,
        metadata.tidal == Some(true),
        format!("Vancouver is not tidal in the metadata: {metadata:?}"),
    );
}

/// IWLS: a Great Lakes gauge (Toronto, 13320) with water levels and an IGLD 85 offset.
#[test]
#[ignore = "live: needs the network"]
fn live_dfo_iwls_toronto_lake_gauge() {
    let toronto = iwls_station("13320");
    std::thread::sleep(GAP);
    let url = metadata_url(&toronto.id);
    let target = station_target(toronto, &[M::Tide], None);
    let request = Request::new(url.clone(), Resource::DfoMetadata, target.clone());
    let fetched = get_call(&transport(), &Call::from(&request));
    let metadata = parsed(
        &url,
        "the IWLS metadata",
        parse_metadata(&request, &fetched.as_response()),
    );
    expect(
        &url,
        metadata.igld85_offset_m().is_some(),
        format!("Toronto has no IGLD85 datum: {metadata:?}"),
    );

    // The plan also asks for high and low water, which a lake gauge does not publish (the
    // service answers 404), so only the observed levels are read here.
    std::thread::sleep(GAP);
    let app = ctx();
    let planned = parsed(
        &url,
        "the IWLS plan",
        Dfo.plan(&target, &requirements(&[M::Tide]), &app),
    );
    let Some(observed) = planned
        .iter()
        .find(|r| r.url.contains("time-series-code=wlo"))
    else {
        fail(&url, "the plan has no observed-level request");
    };
    let fetched = get_call(&transport(), &Call::from(observed));
    let mut out = Fragments::new();
    parsed(
        &observed.url,
        "the IWLS water levels",
        Dfo.parse(observed, &fetched.as_response(), &app, &mut out),
    );
    expect(
        &observed.url,
        !out.is_empty(),
        "the observed levels of Toronto 13320 gave no value",
    );
}

/// ECCC `GeoMet`: the active stations around Calgary list the Bow River station, its real-time
/// level and discharge parse, and so do the daily means for its percentile.
#[test]
#[ignore = "live: needs the network"]
fn live_eccc_hydrometric_bow_river() {
    let transport = transport();
    let app = ctx();
    let area = Bbox {
        west: -114.30,
        south: 50.90,
        east: -113.90,
        north: 51.20,
    };
    let list_url = parsed(
        "https://api.weather.gc.ca/collections/hydrometric-stations/items",
        "the stations request",
        eccc_stations_url(area, 0),
    );
    let wanted = [M::Discharge, M::GageHeight];
    let target = point_target((51.05, -114.05), &wanted);
    let request = Request::new(list_url.clone(), Resource::EcccStations, target);
    let fetched = get_call(&transport, &Call::from(&request));
    let page = parsed(
        &list_url,
        "the ECCC station list",
        eccc::parse_stations(&request, &fetched.as_response()),
    );
    let Some(bow) = page.stations.iter().find(|s| s.id == "05BH004") else {
        fail(
            &list_url,
            format!(
                "the Calgary box lists {} stations but not 05BH004",
                page.stations.len()
            ),
        );
    };
    let bow = bow.station_ref();
    assert_eq!(bow.source, SourceId::ECCC_HYDROMETRIC);
    assert_eq!(bow.kind, StationKind::WaterLevel);

    std::thread::sleep(GAP);
    let target = station_target(bow, &wanted, None);
    let eccc = Eccc::new();
    let realtime = parsed(
        "https://api.weather.gc.ca/collections/hydrometric-realtime/items",
        "the real-time request",
        eccc::realtime_request(&target, 12),
    );
    let fetched = get_call(&transport, &Call::from(&realtime));
    let mut out = Fragments::new();
    parsed(
        &realtime.url,
        "the ECCC real-time rows",
        eccc.parse(&realtime, &fetched.as_response(), &app, &mut out),
    );
    expect(
        &realtime.url,
        out.fragments()
            .iter()
            .any(|f| f.measurement == M::Discharge),
        "the real-time rows gave no discharge for 05BH004",
    );

    std::thread::sleep(GAP);
    let reading = parsed(
        &realtime.url,
        "the latest reading",
        eccc.latest_reading(&realtime, &fetched.as_response(), M::Discharge),
    );
    let daily = parsed(
        "https://api.weather.gc.ca/collections/hydrometric-daily-mean/items",
        "the daily-mean request",
        eccc.percentile_request(&target, reading),
    );
    let fetched = get_call(&transport, &Call::from(&daily));
    let mut out = Fragments::new();
    parsed(
        &daily.url,
        "the ECCC daily means",
        eccc.parse(&daily, &fetched.as_response(), &app, &mut out),
    );
    expect(
        &daily.url,
        !out.is_empty(),
        "the daily means gave no percentile for 05BH004",
    );
}
