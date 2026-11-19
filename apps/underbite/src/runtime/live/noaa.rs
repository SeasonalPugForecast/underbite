//! CO-OPS, NDBC and NWPS at the Battery, buoy 44025 and the lower Hudson.

use underbite_core::datum::VerticalDatum;
use underbite_core::model::Measurement as M;
use underbite_core::provenance::{SourceId, StationKind};
use underbite_providers::coops::mdapi::{self, Metadata, StationList};
use underbite_providers::coops::{Coops, CoopsHarmonics};
use underbite_providers::ndbc::{Ndbc, stations};
use underbite_providers::nwps::Nwps;
use underbite_providers::request::Resource;

use super::{
    BATTERY, GAP, LOWER_HUDSON, ctx, expect, get_call, parsed, point_target, requirements, run,
    station, station_target, transport,
};
use underbite_fetch::Call;

fn battery_gauge() -> underbite_core::provenance::StationRef {
    station(
        SourceId::COOPS,
        "8518750",
        "The Battery",
        StationKind::TideHarmonic,
        (40.7001, -74.0142),
        None,
    )
}

fn battery_reach() -> underbite_core::provenance::StationRef {
    station(
        SourceId::COOPS,
        "NYH1927",
        "The Battery Reach",
        StationKind::CurrentHarmonic,
        (40.6998, -74.0167),
        Some(13),
    )
}

fn buoy_44025() -> underbite_core::provenance::StationRef {
    station(
        SourceId::NDBC,
        "44025",
        "Long Island, 30 NM south of Islip",
        StationKind::Buoy,
        (40.251, -73.164),
        None,
    )
}

/// CO-OPS data API: high and low tides and the curve at the Battery.
#[test]
#[ignore = "live: needs the network"]
fn live_coops_tides_battery() {
    let wanted = [M::Tide];
    let target = station_target(battery_gauge(), &wanted, Some(VerticalDatum::Mllw));
    let found = run(&Coops, &target, &requirements(&wanted), 3);
    found.expect_all("CO-OPS tide predictions at 8518750", &wanted);
}

/// CO-OPS data API: observed water temperature, pressure and wind at the Battery. The wind
/// sensor may say "no data", which is not a failure.
#[test]
#[ignore = "live: needs the network"]
fn live_coops_observations_battery() {
    let wanted = [M::WaterTemperature, M::ObservedPressure, M::ObservedWind];
    let target = station_target(battery_gauge(), &wanted, Some(VerticalDatum::Mllw));
    let found = run(&Coops, &target, &requirements(&wanted), 4);
    found.expect_all(
        "CO-OPS observations at 8518750",
        &[M::WaterTemperature, M::ObservedPressure],
    );
}

/// CO-OPS data API: current predictions at the Battery Reach (NYH1927, bin 13).
#[test]
#[ignore = "live: needs the network"]
fn live_coops_currents_battery_reach() {
    let wanted = [M::TidalCurrent];
    let target = station_target(battery_reach(), &wanted, None);
    let found = run(&Coops, &target, &requirements(&wanted), 3);
    found.expect_all("CO-OPS current predictions at NYH1927", &wanted);
}

/// CO-OPS metadata API: the datums and harmonic constants of the Battery (the follow-up chain
/// of the computed-prediction source), and the harmonics of the Battery Reach.
#[test]
#[ignore = "live: needs the network"]
fn live_coops_harmonics_battery() {
    let tide = [M::Tide];
    let target = station_target(battery_gauge(), &tide, Some(VerticalDatum::Mllw));
    let found = run(&CoopsHarmonics, &target, &requirements(&tide), 3);
    found.expect_all("CO-OPS tide harmonics at 8518750", &tide);

    let current = [M::TidalCurrent];
    let target = station_target(battery_reach(), &current, None);
    let found = run(&CoopsHarmonics, &target, &requirements(&current), 2);
    found.expect_all("CO-OPS current harmonics at NYH1927", &current);
}

/// CO-OPS metadata API: the tide-prediction and current-prediction station lists list the
/// Battery and its reach, and the reach's bins and offsets parse.
#[test]
#[ignore = "live: needs the network"]
fn live_coops_station_lists() {
    let transport = transport();
    let wanted = [M::Tide];
    for (list, id) in [
        (StationList::TidePredictions, "8518750"),
        (StationList::CurrentPredictions, "NYH1927"),
    ] {
        let target = station_target(battery_gauge(), &wanted, None);
        let request = mdapi::stations_request(list, target);
        let call = Call::from(&request);
        let fetched = get_call(&transport, &call);
        let Metadata::Stations(stations) = parsed(
            &request.url,
            "the CO-OPS station list",
            mdapi::parse(&request, &fetched.as_response()),
        ) else {
            super::fail(&request.url, "a station list parsed as something else");
        };
        expect(
            &request.url,
            stations.iter().any(|s| s.station.id == id),
            format!("the list has {} stations but not {id}", stations.len()),
        );
        std::thread::sleep(GAP);
    }

    let target = station_target(battery_reach(), &[M::TidalCurrent], None);
    let request = parsed(
        "https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/NYH1927/bins.json",
        "the bins request",
        mdapi::bins_request(target),
    );
    let fetched = get_call(&transport, &Call::from(&request));
    let Metadata::Bins(bins) = parsed(
        &request.url,
        "the CO-OPS bins",
        mdapi::parse(&request, &fetched.as_response()),
    ) else {
        super::fail(&request.url, "bins parsed as something else");
    };
    expect(
        &request.url,
        bins.iter().any(|b| b.number == 13),
        format!("NYH1927 has {} bins, none numbered 13", bins.len()),
    );
}

/// NDBC realtime files: waves, water temperature, wind and pressure at buoy 44025.
#[test]
#[ignore = "live: needs the network"]
fn live_ndbc_realtime_44025() {
    let wanted = [
        M::Waves,
        M::WaterTemperature,
        M::ObservedWind,
        M::ObservedPressure,
    ];
    let target = station_target(buoy_44025(), &wanted, None);
    let found = run(&Ndbc, &target, &requirements(&wanted), 4);
    found.expect_all(
        "NDBC realtime files of 44025",
        &[M::Waves, M::WaterTemperature],
    );
}

/// NDBC: the active station list parses and holds 44025, and the coverage read of the
/// standard file shows what the station reports.
#[test]
#[ignore = "live: needs the network"]
fn live_ndbc_station_list_and_coverage() {
    let transport = transport();
    let target = station_target(buoy_44025(), &[M::Waves], None);
    let request = stations::request(target.clone());
    let fetched = get_call(&transport, &Call::from(&request));
    let text = String::from_utf8_lossy(&fetched.body).into_owned();
    let list = parsed(
        &request.url,
        "the NDBC active station list",
        stations::parse(&text),
    );
    expect(
        &request.url,
        list.iter().any(|c| c.station.id == "44025"),
        format!("the list has {} stations but not 44025", list.len()),
    );

    std::thread::sleep(GAP);
    let coverage = parsed(
        "https://www.ndbc.noaa.gov/data/realtime2/44025.txt",
        "the coverage request",
        Ndbc.coverage_request(&target, Resource::NdbcStandard),
    );
    let fetched = get_call(&transport, &Call::from(&coverage));
    let mut out = underbite_providers::source::Fragments::new();
    let result = underbite_providers::source::Source::parse(
        &Ndbc,
        &coverage,
        &fetched.as_response(),
        &ctx(),
        &mut out,
    );
    parsed(&coverage.url, "the NDBC coverage read", result);
    expect(
        &coverage.url,
        !out.is_empty(),
        "the standard file of 44025 shows no measurement with a value",
    );
}

/// NWPS: a box over the lower Hudson lists gauges, and the Battery gauge (BATN6) gives its
/// flood categories. The forecast of MAWN4 is read too; a gauge with no forecast is not a
/// failure.
#[test]
#[ignore = "live: needs the network"]
fn live_nwps_gauges_and_forecast() {
    let wanted = [M::FlowForecast, M::FloodCategories];
    let target = point_target(LOWER_HUDSON, &wanted);
    let found = run(&Nwps, &target, &requirements(&wanted), 1);
    assert!(
        !found.fragments.is_empty(),
        "live check failed: the NWPS box over the lower Hudson lists no gauge\n  asked: {:?}",
        found.urls
    );

    std::thread::sleep(GAP);
    let battery = station(
        SourceId::NWPS,
        "BATN6",
        "New York Harbor at The Battery",
        StationKind::NwpsGauge,
        (BATTERY.0, BATTERY.1),
        None,
    );
    let target = station_target(battery, &[M::FloodCategories], None);
    let found = run(&Nwps, &target, &requirements(&[M::FloodCategories]), 1);
    found.expect_all("NWPS gauge BATN6", &[M::FloodCategories]);

    std::thread::sleep(GAP);
    let croton_side = station(
        SourceId::NWPS,
        "MAWN4",
        "Ramapo River at Mahwah",
        StationKind::NwpsGauge,
        (41.09, -74.15),
        None,
    );
    let target = station_target(croton_side, &wanted, None);
    let found = run(&Nwps, &target, &requirements(&wanted), 2);
    found.expect_all("NWPS gauge MAWN4", &[M::FloodCategories]);
}
