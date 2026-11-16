//! One spot, end to end: the water type when it is to be inferred, discovery, the water and
//! marine session, the weather, the normals, the storm watch and the sky, into one snapshot.

use underbite_config::schema::Config;
use underbite_core::astro::solunar::SolunarSettings;
use underbite_core::discover::hydro::WaterTypeSuggestion;
#[cfg(test)]
use underbite_core::discover::rank::Plan;
use underbite_core::model::{Spot, SpotSnapshot};
use underbite_core::time::UnixTime;
use underbite_providers::hydro;
use underbite_providers::session::Session;
use underbite_providers::source::{FetchContext, Secrets};
use underbite_providers::storms::StormOutcome;

use crate::discovery::discover_with;
use crate::host::{Driver, FetchHost};
use crate::normals::fetch_normals;
use crate::registry::{chain_specs, requirements, weather_chain};
use crate::sessions::{fetch_weather, run_session, session_chains};
use crate::sky::{sky, time_zone};
use crate::store::{self, spot_key};
use crate::storms::fetch_storms;
use crate::water_type::{infer_water_type, spot_point};

/// How a fetch runs, besides the spot, the configuration and the time.
#[derive(Clone, Debug)]
pub struct FetchOptions {
    /// Whether metered calls may be spent (USGS, Open-Meteo Marine, the storm point).
    pub allow_metered: bool,
    /// Keys the sources may send.
    pub secrets: Secrets,
    /// The User-Agent the weather providers name.
    pub user_agent: String,
    /// Whether the spot's water type is inferred before planning, the spot's own one used
    /// only when nothing points anywhere.
    pub infer_water_type: bool,
}

/// A spot's fetch: the snapshot and how it came about.
#[derive(Debug)]
pub struct SpotFetch {
    /// Everything known about the spot.
    pub snapshot: SpotSnapshot,
    /// The water type inferred, with its reasons, when it was asked for and something
    /// pointed to one.
    pub inferred: Option<WaterTypeSuggestion>,
    /// The links the water session took.
    #[cfg(test)]
    pub(crate) plan: Plan,
}

/// The sources' context for a fetch at `now`: the configured coordinate precision, the keys
/// and the User-Agent of `options`.
pub(crate) fn fetch_context(
    config: &Config,
    now: UnixTime,
    options: &FetchOptions,
) -> FetchContext {
    FetchContext::new(now, options.user_agent.clone())
        .with_coordinate_precision(config.sources.coordinate_precision)
        .with_secrets(options.secrets.clone())
}

/// Fetches everything a spot's cards need at `now` through `host`: the water type when it is
/// to be inferred, discovery, the water and marine session, the weather, the normals, the storm
/// watch and the sky. The stored lists, rankings and the spot's snapshot are updated and the
/// call ledger saved. Returns the snapshot and how it came about.
pub async fn fetch_spot<H: FetchHost>(
    host: &H,
    spot: &Spot,
    config: &Config,
    now: UnixTime,
    options: &FetchOptions,
) -> SpotFetch {
    let host = Driver::new(host, now, options.allow_metered).await;
    let ctx = fetch_context(config, now, options);
    let rounded = spot_point(&ctx, spot).coordinates();
    let inferred = if options.infer_water_type {
        infer_water_type(&host, &ctx, spot, &config.sources).await
    } else {
        None
    };
    let inferred_spot;
    let spot = match &inferred {
        Some(suggestion) => {
            inferred_spot = Spot {
                water_type: suggestion.water_type,
                ..spot.clone()
            };
            &inferred_spot
        }
        None => spot,
    };
    let requirements = requirements(config, spot.water_type);
    let chains = chain_specs(&config.sources, &requirements, spot.water_type);
    let previous: Option<SpotSnapshot> = host.stored(store::SPOT, &spot_key(&spot.id)).await;
    let discovery = discover_with(&host, &ctx, spot, chains).await;

    let session = Session::new(
        session_chains(&discovery.plan, &requirements, &ctx),
        requirements.clone(),
        ctx.clone(),
    )
    .with_usage(host.usage.clone())
    .allow_metered(options.allow_metered);
    let mut water = run_session(&host, session).await;

    let weather_chain = weather_chain(&config.sources.weather, &rounded);
    let weather = fetch_weather(&host, &weather_chain, rounded, &options.user_agent).await;

    let zone = time_zone(spot, weather.snapshot.as_ref());
    let normals = fetch_normals(
        &host,
        &ctx,
        &mut water.water,
        zone.as_ref().map(|(_, tz)| tz),
    )
    .await;
    let storms = storm_watch(&host, &ctx, spot, config, previous.as_ref()).await;

    let mut snapshot = SpotSnapshot::new(spot.clone(), now);
    let solunar = SolunarSettings {
        major_minutes: config.windows.solunar_major_minutes,
        minor_minutes: config.windows.solunar_minor_minutes,
    };
    if let Some((name, tz)) = zone {
        snapshot.spot.time_zone = name;
        snapshot.sky = sky(spot, &tz, now, requirements.future_days, solunar);
    } else {
        tracing::warn!("no time zone for the spot: no sun and moon times");
    }
    snapshot.weather = weather.snapshot;
    snapshot.water = water.water;
    snapshot.marine = water.marine;
    snapshot.stations = water.stations;
    let mut attribution = water.attribution;
    attribution.extend(weather.attribution);
    if let Some(storms) = &storms {
        attribution.extend(storms.attribution.iter().cloned());
    }
    if discovery.hydrography {
        attribution.push(hydro::attribution());
    }
    for credit in attribution {
        if !snapshot.attribution.contains(&credit) {
            snapshot.attribution.push(credit);
        }
    }
    snapshot.errors = discovery.errors;
    snapshot.errors.extend(water.errors);
    snapshot.errors.extend(normals);
    snapshot.errors.extend(weather.errors);
    if let Some(storms) = storms {
        snapshot.errors.extend(storms.errors);
        snapshot.storm = Some(storms.watch);
    }

    host.keep(store::SPOT, &spot_key(&spot.id), &snapshot).await;
    if let Err(e) = host.save_usage().await {
        tracing::warn!(error = %e, "call ledger");
    }
    SpotFetch {
        snapshot,
        inferred,
        #[cfg(test)]
        plan: discovery.plan,
    }
}

/// The storm-ahead check and hurricane watch for `spot`, from its previous snapshot's watch;
/// none when the storm group's chain is empty, which switches the check off (the plan runs both
/// parts).
async fn storm_watch<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
    config: &Config,
    previous: Option<&SpotSnapshot>,
) -> Option<StormOutcome> {
    if config.sources.storm.chain.is_empty() {
        return None;
    }
    let previous = previous.and_then(|p| p.storm.as_ref());
    let distance_km = config.storms.hurricane_distance_km;
    Some(fetch_storms(host, ctx, spot, previous, distance_km).await)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::registry::measurement_limits;
    use crate::testing::{MemoryHost, assert_rounded, battery, croton, now};
    use Measurement as M;
    use underbite_core::discover::rank::LinkPlace;
    use underbite_core::model::{
        MarineConditions, Measurement, SourceErrorKind, WaterType, ZoneKind,
    };
    use underbite_core::provenance::SourceId;
    use underbite_providers::session::usage::{OPEN_METEO, UsageLedger};
    use whether_core::model::ProviderId;

    fn options(infer_water_type: bool) -> FetchOptions {
        FetchOptions {
            allow_metered: true,
            secrets: Secrets::default(),
            user_agent: "underbite/test".into(),
            infer_water_type,
        }
    }

    fn fetched_at(host: &MemoryHost, spot: &Spot, config: &Config, at: UnixTime) -> SpotFetch {
        pollster::block_on(fetch_spot(host, spot, config, at, &options(false)))
    }

    fn fetched(spot: &Spot, config: &Config, scenario: &str) -> (SpotFetch, MemoryHost) {
        let host = MemoryHost::replay(scenario);
        let out = fetched_at(&host, spot, config, now());
        (out, host)
    }

    fn errors(snapshot: &SpotSnapshot) -> Vec<(String, Option<Measurement>, SourceErrorKind)> {
        snapshot
            .errors
            .iter()
            .map(|e| (e.source.to_string(), e.measurement, e.kind))
            .collect()
    }

    fn sources_card(s: &SpotSnapshot) -> underbite_core::view::cards::sources::SourcesCard {
        let config = Config::default();
        let input = underbite_core::view::ViewInput {
            card_order: config.cards.order_for(s.spot.water_type),
            snapshot: s.clone(),
            now: now(),
            units: config.units.display_units(),
            voice: config.presentation.voice,
            clock: config.presentation.clock,
            timeline_span: config.presentation.timeline_span(),
            limits: measurement_limits(&config.sources),
            river_ranges: None,
            trout_stress_c: None,
            solunar: underbite_core::view::SolunarSettings { show: true },
        };
        underbite_core::view::cards::sources::build(&input).expect("sources card")
    }

    fn station_of<T>(series: &underbite_core::series::Series<T>) -> String {
        let p = series.provenance();
        format!(
            "{} {}",
            p.source,
            p.station.as_ref().map_or("", |s| s.id.as_str())
        )
    }

    #[test]
    fn a_fetch_can_run_on_any_thread() {
        fn sendable<T: Send>(_: &T) {}
        let host = MemoryHost::default();
        let config = Config::default();
        let options = options(true);
        let spot = battery();
        let fetch = fetch_spot(&host, &spot, &config, now(), &options);
        sendable(&fetch);
        let ctx = crate::testing::ctx();
        let chains = std::collections::BTreeMap::new();
        let discovery = crate::discover(&host, &ctx, &spot, chains, true);
        sendable(&discovery);
    }

    #[test]
    fn the_battery_end_to_end() {
        let (out, host) = fetched(&battery(), &Config::default(), "battery");
        let s = &out.snapshot;

        let tide = s.marine.tide.as_ref().expect("tide");
        assert_eq!(tide.events.len(), 27);
        let station = tide.provenance.station.as_ref().expect("station");
        assert_eq!(
            (tide.provenance.source.as_str(), station.id.as_str()),
            ("coops", "8518750")
        );
        assert!(tide.curve.is_some() && tide.observed.is_some());
        let current = s.marine.tidal_current.as_ref().expect("current");
        let at = current.curve.as_ref().expect("curve").provenance();
        let station = at.station.as_ref().expect("station");
        assert_eq!((station.id.as_str(), station.bin), ("NYH1927", Some(13)));
        assert!(current.events.len() > 20, "{}", current.events.len());

        let forecast = s.marine.marine_forecast.as_ref().expect("forecast");
        assert_eq!(forecast.id, "ANZ338");
        assert!(forecast.periods.len() > 2, "{:?}", forecast.periods);
        assert!(
            s.marine
                .marine_alerts
                .iter()
                .any(|a| a.event == "Small Craft Advisory")
        );

        // Buoy 44065 lies within the default 50 km: the observed waves and wind are its and
        // the Robbins Reef light's; the model's waves are the forecast.
        let waves = s.marine.waves.as_ref().expect("waves");
        assert_eq!(station_of(waves), "ndbc 44065");
        let waves = s.marine.waves_forecast.as_ref().expect("waves forecast");
        assert_eq!(waves.provenance().source, SourceId::OPEN_METEO_MARINE);
        let wind = s.marine.observed_wind.as_ref().expect("wind");
        assert_eq!(station_of(wind), "ndbc robn4");
        let temp = s.water.temperature_c.as_ref().expect("water temperature");
        assert_eq!(station_of(temp), "ndbc batn6");

        let weather = s.weather.as_ref().expect("weather");
        assert_eq!(weather.provider, ProviderId::OpenMeteo);
        assert!(weather.hourly.len() > 24, "{}", weather.hourly.len());
        assert_eq!(s.spot.time_zone, "America/New_York");
        assert_eq!(s.sky.days.len(), 7);
        assert_eq!(s.fetched_at, now());

        // The CO-OPS stations' water temperature and pressure end on 7 November, older than 180
        // minutes, and they report no wind; each is recorded before the buoys answer, and the
        // Battery's own barometer answers through NDBC.
        let coops = |m, kind| ("coops".to_string(), Some(m), kind);
        let (stale, no_wind, old_pressure) = (
            coops(M::WaterTemperature, SourceErrorKind::Stale),
            coops(M::ObservedWind, SourceErrorKind::NoData),
            coops(M::ObservedPressure, SourceErrorKind::Stale),
        );
        let mut expected = vec![stale; 2];
        expected.extend([no_wind.clone(), no_wind.clone(), no_wind]);
        expected.extend(vec![old_pressure; 3]);
        assert_eq!(errors(s), expected);
        let pressure = s.marine.observed_pressure_hpa.as_ref().expect("pressure");
        assert_eq!(station_of(pressure), "ndbc batn6");
        // The sources card names the CO-OPS stations' missing wind once, with the count.
        let card = sources_card(s);
        let wind: Vec<&str> = card
            .full
            .errors
            .iter()
            .filter(|e| e.source == "NOAA CO-OPS" && e.measurement.as_deref() == Some("Wind"))
            .map(|e| e.message.as_str())
            .collect();
        assert_eq!(wind.len(), 1, "{:#?}", card.full.errors);
        assert!(wind[0].ends_with(" (2 more)"), "{}", wind[0]);
        assert_eq!(card.full.errors.len(), 3, "{:#?}", card.full.errors);

        let credited: BTreeSet<&str> = s.attribution.iter().map(|a| a.source.as_str()).collect();
        for source in [
            "coops",
            "ndbc",
            "nws",
            "open-meteo-marine",
            "open-meteo",
            "usgs-3dhp",
        ] {
            assert!(credited.contains(source), "{source}: {credited:?}");
        }
        for url in host.seen() {
            assert_rounded(&url);
        }
        assert_eq!(host.missed(), Vec::<String>::new());

        // The snapshot and the ledger are stored: Marine and the forecast count against
        // Open-Meteo's one limit.
        let saved: SpotSnapshot = host
            .stored(store::SPOT, &spot_key("battery"))
            .expect("saved");
        assert_eq!(&saved, s);
        let ledger: UsageLedger = host
            .stored(store::USAGE, store::LEDGER_KEY)
            .expect("ledger");
        assert_eq!(ledger.used(&OPEN_METEO, now()), 3);

        // The storm check: the models' hours at the spot, no hurricane near.
        let watch = s.storm.as_ref().expect("storm watch");
        assert_eq!(watch.checked_at, now());
        let point = watch.point.as_ref().expect("point series");
        assert!(point.points.len() > 1, "{}", point.points.len());
        assert!(point.points.iter().all(|p| p.run.is_some()));
        assert_eq!(watch.hurricanes, []);
        assert!(credited.contains("open-meteo-storm"));
    }

    #[test]
    fn the_battery_without_a_water_type_is_inferred_tidal_and_fetched_once() {
        use underbite_core::discover::hydro::WaterTypeReason;

        // A river until inferred otherwise; nothing stored, so only the kept responses avoid
        // asking twice.
        let host = MemoryHost::replay("battery").forgetful();
        let spot = Spot {
            water_type: WaterType::RiverStream,
            ..battery()
        };
        let out = pollster::block_on(fetch_spot(
            &host,
            &spot,
            &Config::default(),
            now(),
            &options(true),
        ));
        let inferred = out.inferred.expect("inferred");
        assert_eq!(inferred.water_type, WaterType::TidalRiverEstuary);
        assert_eq!(
            inferred.reasons.first(),
            Some(&WaterTypeReason::MarineZone {
                kind: ZoneKind::Coastal
            }),
            "{:?}",
            inferred.reasons
        );
        assert!(inferred.reasons.len() > 1, "{:?}", inferred.reasons);
        let s = &out.snapshot;
        assert_eq!(s.spot.water_type, WaterType::TidalRiverEstuary);
        assert!(s.marine.tide.is_some() && s.marine.marine_forecast.is_some());

        let seen = host.seen();
        let looked_up = |url: &str| {
            [
                "api.weather.gov/points/",
                "api.weather.gov/zones?",
                "3DHP_all",
            ]
            .iter()
            .chain(&["monitoring-locations", "time-series-metadata"])
            .any(|part| url.contains(part))
        };
        let lookups: Vec<&String> = seen.iter().filter(|u| looked_up(u)).collect();
        assert!(lookups.len() >= 4, "{lookups:?}");
        for url in &lookups {
            let times = seen.iter().filter(|u| u == url).count();
            assert_eq!(times, 1, "{url} asked {times} times");
        }
    }

    #[test]
    fn the_storm_point_is_reused_for_three_hours() {
        let (first, host) = fetched(&battery(), &Config::default(), "battery");
        let sent = host.seen().len();
        let later = UnixTime(now().0 + 3_600);
        let again = fetched_at(&host, &battery(), &Config::default(), later);
        assert!(
            host.seen()[sent..].iter().all(|u| !u.contains("&models=")),
            "{:#?}",
            &host.seen()[sent..]
        );
        let (a, b) = (
            first.snapshot.storm.expect("first"),
            again.snapshot.storm.expect("again"),
        );
        assert_eq!(a.point, b.point);
        assert_eq!(b.checked_at, later);
    }

    #[test]
    fn croton_end_to_end() {
        let mut config = Config::default();
        config.sources.flow.max_distance_km = Some(17.0);
        let (out, host) = fetched(&croton(), &config, "croton");
        let s = &out.snapshot;
        // River water: no tide, current, waves or marine forecast.
        assert_eq!(s.marine, MarineConditions::default());
        assert!(s.weather.is_some());
        assert_eq!(s.sky.days.len(), 7);
        let link = &out.plan.chains[&M::Discharge].links[0];
        assert!(
            matches!(&link.place, LinkPlace::Station { station, .. } if station.id == "USGS-01375000")
        );
        // The one expected error: the buoy list's Haverstraw station has no data file (it
        // reports water quality only). No forecast gauge is asked: none is on the Croton.
        assert_eq!(
            errors(s),
            [(
                "ndbc".into(),
                Some(M::WaterTemperature),
                SourceErrorKind::NotFound
            )]
        );
        let discharge = s.water.discharge_m3s.as_ref().expect("discharge");
        assert_eq!(station_of(discharge), "usgs USGS-01375000");
        let gage = s.water.gage_height.as_ref().expect("gage height");
        assert_eq!(station_of(&gage.series), "usgs USGS-01375000");
        // Both have their histories: the 14-day window holds hundreds of readings.
        assert!(
            discharge.samples().len() > 100,
            "{}",
            discharge.samples().len()
        );
        assert!(
            gage.series.samples().len() > 100,
            "{}",
            gage.series.samples().len()
        );
        let percentile = s
            .water
            .discharge_percentile_for_date
            .as_ref()
            .expect("percentile");
        assert_eq!(percentile.day_of_year, "11-08");
        assert_eq!(
            percentile
                .provenance
                .as_ref()
                .and_then(|p| p.station.as_ref())
                .map(|st| st.id.as_str()),
            Some("USGS-01375000")
        );
        // Flood stages and forecasts would only come from a gauge on the Croton: there is none.
        assert_eq!(s.water.flood_categories, None);
        assert_eq!(s.water.discharge_forecast_m3s, None);
        for m in [M::FloodCategories, M::FlowForecast] {
            assert!(out.plan.chains.get(&m).is_none_or(|c| c.links.is_empty()));
        }
        assert!(
            host.seen().iter().all(|u| !u.contains("/stageflow")),
            "{:#?}",
            host.seen()
        );
        for url in host.seen() {
            assert_rounded(&url);
        }
    }

    #[test]
    fn the_forecast_names_the_zone_of_a_spot_without_one() {
        let mut spot = croton();
        spot.time_zone = String::new();
        let (out, _) = fetched(&spot, &Config::default(), "croton");
        assert_eq!(out.snapshot.spot.time_zone, "America/New_York");
        assert_eq!(out.snapshot.sky.days.len(), 7);
    }
}
