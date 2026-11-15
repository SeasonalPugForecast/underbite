//! The Battery (New York Harbor) through a session, from recorded fixtures.

mod replay;

use std::collections::BTreeMap;
use std::sync::Arc;

use replay::{Route, chain, ok, point_link, station, station_link, with_status};
use underbite_core::model::Measurement as M;
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef};
use underbite_providers::coops::{Coops, CoopsHarmonics};
use underbite_providers::ndbc::Ndbc;
use underbite_providers::nws_marine::NwsMarine;
use underbite_providers::open_meteo_marine::OpenMeteoMarine;
use underbite_providers::session::{Chain, Link};
use underbite_providers::source::Source;

const BATTERY: (f64, f64) = (40.70, -74.02);
/// Water temperature values older than this are passed over, seconds.
const MAX_AGE_S: i64 = 180 * 60;

fn coops() -> Arc<dyn Source> {
    Arc::new(Coops)
}

fn ndbc() -> Arc<dyn Source> {
    Arc::new(Ndbc)
}

fn battery_gauge() -> StationRef {
    station(
        SourceId::COOPS,
        "8518750",
        "The Battery",
        StationKind::TideHarmonic,
        (40.7001, -74.0142),
        None,
    )
}

fn currents_station() -> StationRef {
    station(
        SourceId::COOPS,
        "NYH1927",
        "The Battery Reach",
        StationKind::CurrentHarmonic,
        (40.6998, -74.0167),
        Some(13),
    )
}

fn buoy(id: &str, name: &str, at: (f64, f64)) -> StationRef {
    station(SourceId::NDBC, id, name, StationKind::Buoy, at, None)
}

fn buoy_44025() -> StationRef {
    buoy(
        "44025",
        "Long Island, 30 NM south of Islip",
        (40.251, -73.164),
    )
}

/// Chains of the Battery scenario; `tide_links` are the links of the tide chain.
fn chains(tide_links: Vec<Link>) -> BTreeMap<M, Chain> {
    let mut chains = BTreeMap::new();
    chains.insert(M::Tide, chain(tide_links, None));
    chains.insert(
        M::TidalCurrent,
        chain(
            vec![station_link(
                coops(),
                currents_station(),
                Relation::SameWaterBody,
                &[M::TidalCurrent],
            )],
            None,
        ),
    );
    chains.insert(
        M::WaterTemperature,
        chain(
            vec![
                station_link(
                    coops(),
                    battery_gauge(),
                    Relation::SameWaterBody,
                    &[M::WaterTemperature],
                ),
                station_link(
                    ndbc(),
                    buoy_44025(),
                    Relation::Nearby,
                    &[M::WaterTemperature],
                ),
            ],
            Some(MAX_AGE_S),
        ),
    );
    chains.insert(
        M::Waves,
        chain(
            vec![
                station_link(ndbc(), buoy_44025(), Relation::Nearby, &[M::Waves]),
                point_link(Arc::new(OpenMeteoMarine), BATTERY, &[M::Waves]),
            ],
            None,
        ),
    );
    let harbor_entrance = buoy("44065", "New York Harbor Entrance", (40.369, -73.703));
    for m in [M::ObservedWind, M::ObservedPressure] {
        chains.insert(
            m,
            chain(
                vec![station_link(
                    ndbc(),
                    harbor_entrance.clone(),
                    Relation::Nearby,
                    &[m],
                )],
                None,
            ),
        );
    }
    for m in [M::WavesForecast, M::SeaSurfaceTempForecast] {
        chains.insert(
            m,
            chain(
                vec![point_link(Arc::new(OpenMeteoMarine), BATTERY, &[m])],
                None,
            ),
        );
    }
    for m in [M::MarineForecast, M::MarineAlerts] {
        chains.insert(
            m,
            chain(vec![point_link(Arc::new(NwsMarine), BATTERY, &[m])], None),
        );
    }
    chains
}

const WANTED: [M; 10] = [
    M::Tide,
    M::TidalCurrent,
    M::WaterTemperature,
    M::Waves,
    M::ObservedWind,
    M::ObservedPressure,
    M::WavesForecast,
    M::SeaSurfaceTempForecast,
    M::MarineForecast,
    M::MarineAlerts,
];

const DG: &str = "api.tidesandcurrents.noaa.gov/api/prod/datagetter";
const MDAPI: &str = "api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8518750";

fn routes() -> Vec<Route> {
    let mut routes = coops_routes();
    routes.extend(buoy_and_model_routes());
    routes.extend(nws_routes());
    routes
}

fn coops_routes() -> Vec<Route> {
    vec![
        ok(
            &[
                DG,
                "product=predictions",
                "station=8518750",
                "interval=hilo",
            ],
            "coops",
            "coops-8518750-hilo-2026-11-08.json",
        ),
        ok(
            &[DG, "product=predictions", "station=8518750", "interval=6"],
            "coops",
            "coops-8518750-predictions-6min-2026-11-08.json",
        ),
        ok(
            &[DG, "product=water_level", "station=8518750"],
            "coops",
            "coops-8518750-water-level-2026-11-07.json",
        ),
        ok(
            &[DG, "product=water_temperature", "station=8518750"],
            "coops",
            "coops-8518750-water-temperature-2026-11-07.json",
        ),
        ok(
            &[
                DG,
                "product=currents_predictions",
                "station=NYH1927",
                "interval=max_slack",
            ],
            "coops",
            "coops-NYH1927-bin13-max-slack-2026-11-08.json",
        ),
        ok(
            &[
                DG,
                "product=currents_predictions",
                "station=NYH1927",
                "interval=6",
            ],
            "coops",
            "coops-NYH1927-bin13-currents-predictions-6min-2026-11-08.json",
        ),
    ]
}

fn buoy_and_model_routes() -> Vec<Route> {
    vec![
        ok(
            &["ndbc.noaa.gov/data/realtime2/44025.txt"],
            "ndbc",
            "ndbc-44025.txt",
        ),
        ok(
            &["ndbc.noaa.gov/data/realtime2/44025.spec"],
            "ndbc",
            "ndbc-44025.spec",
        ),
        with_status(
            404,
            &["ndbc.noaa.gov/data/realtime2/44025.ocean"],
            "ndbc",
            "ndbc-44025.ocean-404.html",
        ),
        ok(
            &["ndbc.noaa.gov/data/realtime2/44065.txt"],
            "ndbc",
            "ndbc-44065-head300.txt",
        ),
        ok(
            &["ndbc.noaa.gov/data/realtime2/44065.spec"],
            "ndbc",
            "ndbc-44065-spec-head150.spec",
        ),
        ok(
            &[
                "marine-api.open-meteo.com",
                "latitude=40.7",
                "longitude=-74.02",
            ],
            "open-meteo-marine",
            "marine-40.70_-74.02-hourly-2d.json",
        ),
    ]
}

fn nws_routes() -> Vec<Route> {
    vec![
        ok(
            &["api.weather.gov/points/40.7,-74.02"],
            "nws-marine",
            "points-40.7033_-74.0170.geo.json",
        ),
        ok(
            &["api.weather.gov/zones?point=40.7,-74.02"],
            "nws-marine",
            "zones-coastal-point-40.7033_-74.0170.geo.json",
        ),
        ok(
            &["api.weather.gov/zones/coastal/ANZ338/forecast"],
            "nws-marine",
            "zone-forecast-ANZ338.ld.json",
        ),
        ok(
            &["api.weather.gov/alerts/active", "zone=ANZ338"],
            "nws-marine",
            "alerts-active-zone-ANZ338.geo.json",
        ),
    ]
}

#[test]
fn battery_spot_through_the_session() {
    let links = vec![station_link(
        coops(),
        battery_gauge(),
        Relation::SameWaterBody,
        &[M::Tide],
    )];
    let session = replay::session(chains(links), &WANTED);
    let (outcome, served) = replay::replay(session, &routes());
    insta::assert_json_snapshot!("battery", replay::summary(&outcome, &served));
}

#[test]
fn battery_tide_falls_back_to_computed_predictions_when_coops_is_down() {
    let links = vec![
        station_link(
            coops(),
            battery_gauge(),
            Relation::SameWaterBody,
            &[M::Tide],
        ),
        station_link(
            Arc::new(CoopsHarmonics),
            battery_gauge(),
            Relation::SameWaterBody,
            &[M::Tide],
        ),
    ];
    let session = replay::session(chains(links), &[M::Tide]);
    let routes = vec![
        // The service is down: every data request answers 503 (any recorded body will do, the
        // status decides).
        with_status(
            503,
            &["api.tidesandcurrents.noaa.gov/api/prod/datagetter"],
            "coops",
            "coops-error-no-product.txt",
        ),
        ok(
            &[MDAPI, "datums.json"],
            "coops",
            "coops-mdapi-8518750-datums-metric.json",
        ),
        ok(
            &[MDAPI, "harcon.json"],
            "coops",
            "coops-mdapi-8518750-harcon-metric.json",
        ),
    ];
    let (outcome, served) = replay::replay(session, &routes);
    insta::assert_json_snapshot!("battery_computed_tide", replay::summary(&outcome, &served));
}
