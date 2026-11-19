//! Weather and marine forecasts: Open-Meteo, NWS (weather and marine zones) and Open-Meteo
//! Marine.

use underbite_core::model::Measurement as M;
use underbite_core::time::{UnixTime, UtcOffset};
use underbite_providers::nws_marine::NwsMarine;
use underbite_providers::open_meteo_marine::{HistoryKind, OpenMeteoMarine};
use underbite_providers::request::Coordinates;
use whether_core::model::ProviderId;
use whether_core::requirements::DataRequirements;
use whether_providers::nws::{self, Nws};
use whether_providers::open_meteo::{OpenMeteo, OpenMeteoSettings};
use whether_providers::{FetchContext, FetchSession, HttpResponse, Provider, redact_url};

use super::{
    BATTERY, GAP, ctx, fail, get_call, now, parsed, point_target, requirements, run, transport,
};
use underbite_fetch::Call;

/// What the weather session asks for: the cards' fields, a week of hours, no air quality.
fn weather_requirements() -> DataRequirements {
    DataRequirements {
        air_quality: false,
        hourly_hours: 168,
        ..DataRequirements::full()
    }
}

/// Runs whether's session over `primary` at the rounded Battery and checks the snapshot has
/// hourly and daily steps and no failed attempt.
fn weather_at_battery(
    primary: &[&dyn Provider],
    supplements: &[&dyn Provider],
    credited: &[ProviderId],
) {
    let transport = transport();
    let app = ctx();
    let loc = app
        .round(Coordinates {
            latitude: BATTERY.0,
            longitude: BATTERY.1,
        })
        .coordinates();
    let context = FetchContext {
        now: app.now,
        user_agent: app.user_agent.clone(),
        utc_offset_hint: None,
        if_modified_since: None,
        language: "en".to_string(),
    };
    let mut session = FetchSession::new(primary, supplements, loc, weather_requirements(), context);
    let mut urls = Vec::new();
    while let Some(request) = session.next_request() {
        if !urls.is_empty() {
            std::thread::sleep(GAP);
        }
        let mut call = Call::get(request.url.clone());
        call.headers.clone_from(&request.headers);
        let fetched = super::send(&transport, &call);
        urls.push(format!(
            "{} -> HTTP {}",
            redact_url(&request.url),
            fetched.status
        ));
        session.on_response(&HttpResponse {
            status: fetched.status,
            body: &fetched.body,
            date: fetched.date,
            expires: fetched.expires,
            last_modified: fetched.last_modified.as_deref(),
        });
        assert!(
            urls.len() <= 12,
            "live check failed: the weather session sent more than 12 requests: {urls:?}"
        );
    }
    let (snapshot, attempts) = match session.finish(app.now) {
        Ok(done) => done,
        Err(whether_providers::FetchError::AllProvidersFailed(attempts)) => {
            panic!("live check failed: no weather snapshot: {attempts:?}\n  asked: {urls:#?}")
        }
        Err(e) => panic!("live check failed: no weather snapshot ({e})\n  asked: {urls:#?}"),
    };
    let failed: Vec<_> = attempts.iter().filter(|a| a.error.is_some()).collect();
    assert!(
        failed.is_empty(),
        "live check failed: weather requests failed: {failed:?}\n  asked: {urls:?}"
    );
    assert!(
        !snapshot.hourly.is_empty() && !snapshot.daily.is_empty(),
        "live check failed: the snapshot has {} hourly and {} daily steps\n  asked: {urls:?}",
        snapshot.hourly.len(),
        snapshot.daily.len()
    );
    assert!(
        snapshot.current.is_some(),
        "live check failed: the snapshot has no current conditions\n  asked: {urls:?}"
    );
    for id in credited {
        assert!(
            attempts.iter().any(|a| a.provider == *id),
            "live check failed: {id:?} was never asked\n  asked: {urls:#?}"
        );
    }
}

/// Open-Meteo forecast through whether's provider: hourly, daily and current at the Battery.
#[test]
#[ignore = "live: needs the network"]
fn live_open_meteo_weather_battery() {
    let provider = OpenMeteo::new(OpenMeteoSettings::default());
    weather_at_battery(&[&provider], &[], &[]);
}

/// NWS weather through whether's provider, as the app uses it (a supplement to Open-Meteo
/// for places in the United States): `/points`, the forecasts and the alerts at the Battery.
#[test]
#[ignore = "live: needs the network"]
fn live_nws_weather_battery() {
    let primary = OpenMeteo::new(OpenMeteoSettings::default());
    let supplement = Nws::new(nws::DEFAULT_BASE_URL);
    weather_at_battery(&[&primary], &[&supplement], &[ProviderId::Nws]);
}

/// NWS marine: the zone of the Battery (a coastal zone with a forecast endpoint) gives its
/// forecast and alerts; an open-ocean point gives an offshore text product.
#[test]
#[ignore = "live: needs the network"]
fn live_nws_marine_zones() {
    let wanted = [M::MarineForecast, M::MarineAlerts];
    let target = point_target(BATTERY, &wanted);
    let found = run(&NwsMarine, &target, &requirements(&wanted), 4);
    found.expect_all("NWS marine zone at the Battery", &[M::MarineForecast]);

    std::thread::sleep(GAP);
    let target = point_target((38.5, -72.5), &wanted);
    let found = run(&NwsMarine, &target, &requirements(&wanted), 4);
    found.expect_all(
        "NWS offshore product south of Long Island",
        &[M::MarineForecast],
    );
}

/// Open-Meteo Marine: waves, sea surface temperature, currents and sea level in one request
/// at buoy 44025's position and at the Battery, then a month-old history of waves.
#[test]
#[ignore = "live: needs the network"]
fn live_open_meteo_marine() {
    let wanted = [
        M::WavesForecast,
        M::SeaSurfaceTempForecast,
        M::OceanCurrent,
        M::Tide,
    ];
    let target = point_target((40.251, -73.164), &wanted);
    let found = run(&OpenMeteoMarine, &target, &requirements(&wanted), 1);
    found.expect_all(
        "Open-Meteo Marine at buoy 44025",
        &[M::WavesForecast, M::SeaSurfaceTempForecast],
    );

    std::thread::sleep(GAP);
    let target = point_target(BATTERY, &wanted);
    let found = run(&OpenMeteoMarine, &target, &requirements(&wanted), 1);
    found.expect_all("Open-Meteo Marine at the Battery", &[M::WavesForecast]);

    std::thread::sleep(GAP);
    let app = ctx();
    let day = |days_ago: i64| UnixTime(now().0 - days_ago * 86_400).local_date(UtcOffset(0));
    let request = parsed(
        "https://marine-api.open-meteo.com/v1/marine",
        "the history request",
        OpenMeteoMarine.history_request(&target, HistoryKind::Waves, day(30), day(20), &app),
    );
    let fetched = get_call(&transport(), &Call::from(&request));
    let history = parsed(
        &request.url,
        "the Open-Meteo Marine history",
        OpenMeteoMarine.parse_history(&request, &fetched.as_response(), &app),
    );
    if history.waves.is_none() {
        fail(&request.url, "the wave history holds no waves");
    }
}
