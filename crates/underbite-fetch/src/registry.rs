//! How the configuration maps onto the sessions: the registry of water and marine sources a
//! fetch runs, the measurements a spot's water type and cards need, each measurement's
//! configured chain and limits, and the weather providers built from `[sources.weather]`.

use std::collections::BTreeMap;
use std::sync::Arc;

use underbite_config::schema::Config;
use underbite_config::sources::{SourceGroup, SourcesConfig};
use underbite_config::view_input::group_of;
pub use underbite_config::view_input::measurement_limits;
use underbite_core::discover::rank::{ChainSpec, Limits};
use underbite_core::model::{Measurement, Requirements, WaterType};
use underbite_core::provenance::SourceId;
use underbite_core::view::CardId;
use underbite_providers::blooms::cyan::Cyan;
use underbite_providers::blooms::hab::HabGulf;
use underbite_providers::coops::{Coops, CoopsHarmonics};
use underbite_providers::dfo::Dfo;
use underbite_providers::eccc::Eccc;
use underbite_providers::glerl::Glsea;
use underbite_providers::ndbc::Ndbc;
use underbite_providers::nwps::Nwps;
use underbite_providers::nws_marine::NwsMarine;
use underbite_providers::open_meteo_marine::OpenMeteoMarine;
use underbite_providers::request::Coordinates;
use underbite_providers::reservoirs::cwms::Cwms;
use underbite_providers::reservoirs::rise::Rise;
use underbite_providers::source::Source;
use underbite_providers::usgs::Usgs;
use whether_core::model::ProviderId;
use whether_core::requirements::DataRequirements;
use whether_providers::Provider;
use whether_providers::nws::{self, Nws};
use whether_providers::open_meteo::{OpenMeteo, OpenMeteoSettings};
use whether_providers::regions::covers_us;

// ---------------------------------------------------------------------------------------------
// The source registry
// ---------------------------------------------------------------------------------------------

/// The water and marine sources a fetch runs, by configuration id. Other ids in a chain
/// (sources not implemented yet) are skipped at run time. [`water_source`] is the registry;
/// this list checks it.
#[cfg(test)]
const WATER_SOURCES: [SourceId; 14] = [
    SourceId::COOPS,
    SourceId::COOPS_HARMONICS,
    SourceId::USGS,
    SourceId::NDBC,
    SourceId::NWPS,
    SourceId::OPEN_METEO_MARINE,
    SourceId::NWS,
    SourceId::DFO_IWLS,
    SourceId::ECCC_HYDROMETRIC,
    SourceId::GLERL,
    SourceId::CWMS,
    SourceId::RISE,
    SourceId::CYAN,
    SourceId::NOAA_HAB,
];

/// Sources that only make sense on salt water and the Great Lakes (gridded marine models).
const MARINE_ONLY: [SourceId; 1] = [SourceId::OPEN_METEO_MARINE];

/// Sources that serve the Great Lakes alone: the GLSEA surface-temperature grid.
const GREAT_LAKES_ONLY: [SourceId; 1] = [SourceId::GLERL];

/// Whether `source` is worth asking about water of type `water`. `CyAN` maps inland lakes,
/// reservoirs and the Great Lakes, never the open coast; the Gulf bloom forecast covers coastal
/// water only. A bloom chain therefore reaches one of the two, whatever its order.
fn fits_water(source: &SourceId, water: WaterType) -> bool {
    if MARINE_ONLY.contains(source) {
        is_marine(water)
    } else if GREAT_LAKES_ONLY.contains(source) {
        water == WaterType::GreatLakes
    } else if *source == SourceId::CYAN {
        !is_salt(water)
    } else if *source == SourceId::NOAA_HAB {
        is_salt(water)
    } else {
        true
    }
}

/// The source registered under `id`, if a fetch runs it.
pub(crate) fn water_source(id: &SourceId) -> Option<Arc<dyn Source>> {
    Some(match id.as_str() {
        "coops" => Arc::new(Coops),
        "coops-harmonics" => Arc::new(CoopsHarmonics),
        "usgs" => Arc::new(Usgs::new()),
        "ndbc" => Arc::new(Ndbc),
        "nwps" => Arc::new(Nwps),
        "open-meteo-marine" => Arc::new(OpenMeteoMarine),
        "nws" => Arc::new(NwsMarine),
        "dfo-iwls" => Arc::new(Dfo),
        "eccc-hydrometric" => Arc::new(Eccc::new()),
        "glerl" => Arc::new(Glsea),
        "cwms" => Arc::new(Cwms),
        "rise" => Arc::new(Rise),
        "cyan" => Arc::new(Cyan),
        "noaa-hab" => Arc::new(HabGulf),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Measurements, groups and chains
// ---------------------------------------------------------------------------------------------

/// Salt water, where tides, tidal currents and marine models apply.
fn is_salt(water: WaterType) -> bool {
    matches!(
        water,
        WaterType::TidalRiverEstuary
            | WaterType::BayInshore
            | WaterType::Surf
            | WaterType::Nearshore
            | WaterType::Offshore
    )
}

/// Water with waves, marine forecasts and marine alerts: salt water and the Great Lakes.
fn is_marine(water: WaterType) -> bool {
    is_salt(water) || water == WaterType::GreatLakes
}

/// Lakes and ponds, where a reservoir's project and a satellite's lake pixels apply.
fn is_lake(water: WaterType) -> bool {
    matches!(
        water,
        WaterType::LakeReservoir | WaterType::Pond | WaterType::GreatLakes
    )
}

/// The measurements one card shows on `water`. Flow means river flow: discharge at a tidal
/// gauge reverses with the tide, so tidal water takes tides and currents instead; on a lake it
/// is the level, and a reservoir's release and inflow. A bloom is the satellite's cell counts
/// on a lake and the coastal bloom forecast on salt water; rivers have neither.
fn card_measurements(card: CardId, water: WaterType) -> Vec<Measurement> {
    use Measurement as M;
    let salt = is_salt(water);
    let marine = is_marine(water);
    let lake = is_lake(water);
    let mut out = Vec::new();
    match card {
        CardId::Now => {
            out.push(M::WaterTemperature);
            if marine {
                out.extend([M::ObservedWind, M::ObservedPressure]);
            }
        }
        CardId::Tide if salt => out.push(M::Tide),
        CardId::Current if salt => out.push(M::TidalCurrent),
        CardId::Flow => match water {
            WaterType::RiverStream => out.extend([
                M::Discharge,
                M::GageHeight,
                M::FlowForecast,
                M::FloodCategories,
            ]),
            _ if lake => out.extend([M::LakeLevel, M::Reservoir]),
            _ => {}
        },
        CardId::WaterTemp => out.push(M::WaterTemperature),
        CardId::Clarity => out.push(M::Turbidity),
        CardId::Bloom if lake => out.extend([M::Bloom, M::Cyanobacteria]),
        CardId::Bloom if salt => out.push(M::Bloom),
        CardId::Waves if marine => out.extend([M::Waves, M::WavesForecast]),
        CardId::Wind if marine => out.push(M::ObservedWind),
        CardId::Pressure if marine => out.push(M::ObservedPressure),
        CardId::MarineForecast if marine => out.push(M::MarineForecast),
        CardId::Alerts if marine => out.push(M::MarineAlerts),
        CardId::Windows | CardId::GoNoGo => {
            if salt {
                out.push(M::Tide);
            }
            if marine {
                out.extend([M::ObservedWind, M::Waves, M::MarineAlerts]);
            }
        }
        _ => {}
    }
    out
}

/// What a refresh of a spot on `water` fetches: the measurements of its cards, plus the
/// marine forecast and alerts on marine water whatever the cards (they are safety
/// information for go/no-go).
pub fn requirements(config: &Config, water: WaterType) -> Requirements {
    let mut measurements: std::collections::BTreeSet<Measurement> = config
        .cards
        .order_for(water)
        .into_iter()
        .flat_map(|card| card_measurements(card, water))
        .collect();
    if is_marine(water) {
        measurements.extend([Measurement::MarineForecast, Measurement::MarineAlerts]);
    }
    Requirements {
        measurements,
        ..Requirements::default()
    }
}

/// Whether a refresh of a spot on `water` reads the stocking lists: when its stocking card
/// shows. Stocking is no measurement of a station, so it has no chain.
pub(crate) fn wants_stocking(config: &Config, water: WaterType) -> bool {
    config.cards.order_for(water).contains(&CardId::Stocking)
}

/// The chain and limits of every required measurement: the configured sources a fetch runs
/// that supply the measurement, in order, each once; gridded marine models only on marine
/// water, the GLSEA grid only on the Great Lakes, and the bloom sources on the water they map.
pub fn chain_specs(
    sources: &SourcesConfig,
    requirements: &Requirements,
    water: WaterType,
) -> BTreeMap<Measurement, ChainSpec> {
    requirements
        .measurements
        .iter()
        .map(|&m| {
            let group = group_of(sources, m);
            let mut chain: Vec<SourceId> = Vec::new();
            for id in group.chain {
                let runs = water_source(&id).is_some_and(|s| s.measurements().contains(&m));
                let fits = fits_water(&id, water);
                if runs && fits && !chain.contains(&id) {
                    chain.push(id);
                }
            }
            let spec = ChainSpec {
                sources: chain,
                limits: Limits::from_config(group.max_distance_km, group.max_age_minutes),
            };
            (m, spec)
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Weather providers
// ---------------------------------------------------------------------------------------------

/// The weather providers for one spot: the forecast chain and the supplements.
pub(crate) struct WeatherChain {
    /// In fallback order; the first that answers is the primary.
    pub(crate) primary: Vec<Box<dyn Provider>>,
    /// Fill what the primary lacks.
    pub(crate) supplements: Vec<Box<dyn Provider>>,
}

impl std::fmt::Debug for WeatherChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids = |v: &[Box<dyn Provider>]| v.iter().map(|p| p.id()).collect::<Vec<_>>();
        f.debug_struct("WeatherChain")
            .field("primary", &ids(&self.primary))
            .field("supplements", &ids(&self.supplements))
            .finish()
    }
}

/// The weather chain of `[sources.weather]` at `loc`. Its ids are the weather library's
/// provider ids. Open-Meteo forecasts with its best model for the place; the National
/// Weather Service gives official alerts as a supplement where it covers the place. Providers
/// that need an account (`OpenWeather`, Pirate Weather, MET Norway's contact) have no settings
/// here and are skipped, as are unknown ids.
pub(crate) fn weather_chain(group: &SourceGroup, loc: &Coordinates) -> WeatherChain {
    let mut chain = WeatherChain {
        primary: Vec::new(),
        supplements: Vec::new(),
    };
    let mut seen: Vec<ProviderId> = Vec::new();
    for id in &group.chain {
        let Some(provider) = provider_id(id) else {
            tracing::debug!(source = %id, "not a weather provider; skipped");
            continue;
        };
        if seen.contains(&provider) {
            continue;
        }
        seen.push(provider);
        match provider {
            ProviderId::OpenMeteo => chain
                .primary
                .push(Box::new(OpenMeteo::new(OpenMeteoSettings::default()))),
            ProviderId::Nws if covers_us(loc) => chain
                .supplements
                .push(Box::new(Nws::new(nws::DEFAULT_BASE_URL))),
            other => tracing::debug!(provider = other.id(), "weather provider not used here"),
        }
    }
    chain
}

/// The weather library's provider for a configuration id.
fn provider_id(id: &SourceId) -> Option<ProviderId> {
    serde_json::from_value(serde_json::Value::String(id.as_str().to_string())).ok()
}

/// Hours of hourly weather asked for: a week, so a trip's go/no-go has wind and weather for
/// every day the daily forecast reaches.
pub(crate) const HOURLY_HOURS: u8 = 168;

/// What the weather session asks for: everything the cards can show, a week of hours, without
/// air quality (no card shows it).
pub(crate) fn weather_requirements() -> DataRequirements {
    DataRequirements {
        air_quality: false,
        hourly_hours: HOURLY_HOURS,
        ..DataRequirements::full()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use Measurement as M;
    use underbite_config::sources::KNOWN_SOURCES;
    use underbite_config::view_input::OBSERVATION_CHAIN;

    fn ids(chain: &[Box<dyn Provider>]) -> Vec<ProviderId> {
        chain.iter().map(|p| p.id()).collect()
    }

    #[test]
    fn the_registry_runs_known_sources_under_their_own_ids() {
        for id in &WATER_SOURCES {
            assert!(KNOWN_SOURCES.contains(&id.as_str()), "{id}");
            let source = water_source(id).expect("registered");
            assert_eq!(&source.id(), id);
        }
        assert!(water_source(&SourceId::new("mur-sst")).is_none());
    }

    #[test]
    fn the_battery_needs_tides_currents_waves_and_marine_safety() {
        let config = Config::default();
        let req = requirements(&config, WaterType::TidalRiverEstuary);
        for m in [
            M::Tide,
            M::TidalCurrent,
            M::WaterTemperature,
            M::Waves,
            M::WavesForecast,
            M::ObservedWind,
            M::MarineForecast,
            M::MarineAlerts,
        ] {
            assert!(req.measurements.contains(&m), "{m:?}");
        }
        assert!(!req.measurements.contains(&M::Discharge));
        assert_eq!((req.past_hours, req.future_days), (168, 7));
    }

    #[test]
    fn a_river_needs_flow_and_nothing_tidal() {
        let req = requirements(&Config::default(), WaterType::RiverStream);
        assert_eq!(
            req.measurements.into_iter().collect::<Vec<_>>(),
            [
                M::WaterTemperature,
                M::Discharge,
                M::GageHeight,
                M::Turbidity,
                M::FloodCategories,
                M::FlowForecast,
            ]
        );
    }

    #[test]
    fn a_lake_asks_its_level_and_a_reservoirs_release() {
        let req = requirements(&Config::default(), WaterType::LakeReservoir);
        for m in [M::LakeLevel, M::Reservoir] {
            assert!(req.measurements.contains(&m), "{m:?}");
        }
        // No bloom card in the shipped orders: nothing about blooms is fetched.
        assert!(!req.measurements.contains(&M::Bloom));
        assert!(!req.measurements.contains(&M::Cyanobacteria));
        let river = requirements(&Config::default(), WaterType::RiverStream);
        assert!(!river.measurements.contains(&M::Reservoir));
    }

    #[test]
    fn a_bloom_card_asks_what_maps_its_water() {
        // Every water type on the base order, with the bloom card added.
        let mut config = Config::default();
        config.cards.order.push(CardId::Bloom);
        config.cards.river = None;
        config.cards.pond = None;
        let blooms = |water: WaterType| -> Vec<M> {
            requirements(&config, water)
                .measurements
                .into_iter()
                .filter(|m| matches!(m, M::Bloom | M::Cyanobacteria))
                .collect()
        };
        assert_eq!(
            blooms(WaterType::LakeReservoir),
            [M::Cyanobacteria, M::Bloom]
        );
        assert_eq!(blooms(WaterType::Pond), [M::Cyanobacteria, M::Bloom]);
        assert_eq!(blooms(WaterType::GreatLakes), [M::Cyanobacteria, M::Bloom]);
        assert_eq!(blooms(WaterType::BayInshore), [M::Bloom]);
        assert_eq!(blooms(WaterType::Surf), [M::Bloom]);
        assert_eq!(blooms(WaterType::RiverStream), []);

        // The lake's chains reach the satellite, the coast's the bloom forecast.
        let lake = requirements(&config, WaterType::LakeReservoir);
        let specs = chain_specs(&config.sources, &lake, WaterType::LakeReservoir);
        assert_eq!(specs[&M::Bloom].sources, [SourceId::CYAN]);
        assert_eq!(specs[&M::Cyanobacteria].sources, [SourceId::CYAN]);
        let bay = requirements(&config, WaterType::BayInshore);
        let specs = chain_specs(&config.sources, &bay, WaterType::BayInshore);
        assert_eq!(specs[&M::Bloom].sources, [SourceId::NOAA_HAB]);
    }

    #[test]
    fn chains_follow_the_configuration_and_skip_what_does_not_run() {
        let config = Config::default();
        let req = requirements(&config, WaterType::TidalRiverEstuary);
        let specs = chain_specs(&config.sources, &req, WaterType::TidalRiverEstuary);
        let temp = &specs[&M::WaterTemperature];
        let names: Vec<&str> = temp.sources.iter().map(SourceId::as_str).collect();
        // Open-Meteo Marine forecasts sea temperature, not the water temperature at a station.
        assert_eq!(names, ["usgs", "coops", "ndbc"]);
        assert_eq!(temp.limits, Limits::from_config(Some(25.0), Some(180)));
        assert_eq!(specs[&M::Waves].sources, [SourceId::NDBC]);
        assert_eq!(
            specs[&M::WavesForecast].sources,
            [SourceId::OPEN_METEO_MARINE]
        );
        let wind = &specs[&M::ObservedWind];
        assert_eq!(wind.sources, OBSERVATION_CHAIN);
        assert_eq!(wind.limits.max_age_s, 180 * 60);
        assert_eq!(specs[&M::MarineAlerts].sources, [SourceId::NWS]);

        let river = requirements(&config, WaterType::RiverStream);
        let specs = chain_specs(&config.sources, &river, WaterType::RiverStream);
        let names: Vec<&str> = specs[&M::WaterTemperature]
            .sources
            .iter()
            .map(SourceId::as_str)
            .collect();
        assert_eq!(names, ["usgs", "coops", "ndbc"]);
        assert_eq!(specs[&M::FloodCategories].sources, [SourceId::NWPS]);
        assert_eq!(specs[&M::Discharge].sources, [SourceId::USGS]);

        // A gridded marine model is never asked about inland water.
        let mut sst = config.sources.clone();
        sst.sst.chain = vec![SourceId::OPEN_METEO_MARINE];
        let wanted = Requirements {
            measurements: BTreeSet::from([M::SeaSurfaceTempForecast]),
            ..Requirements::default()
        };
        let lake = chain_specs(&sst, &wanted, WaterType::LakeReservoir);
        assert_eq!(
            lake[&M::SeaSurfaceTempForecast].sources,
            Vec::<SourceId>::new()
        );
        let bay = chain_specs(&sst, &wanted, WaterType::BayInshore);
        assert_eq!(
            bay[&M::SeaSurfaceTempForecast].sources,
            [SourceId::OPEN_METEO_MARINE]
        );
    }

    #[test]
    fn the_new_sources_run_where_they_map() {
        let config = Config::default();
        let wanted = |m: M| Requirements {
            measurements: BTreeSet::from([m]),
            ..Requirements::default()
        };
        let names = |m: M, water: WaterType| -> Vec<String> {
            chain_specs(&config.sources, &wanted(m), water)[&m]
                .sources
                .iter()
                .map(|s| s.as_str().to_string())
                .collect()
        };
        assert_eq!(
            names(M::WaterTemperature, WaterType::GreatLakes),
            ["usgs", "coops", "ndbc", "glerl"]
        );
        assert_eq!(
            names(M::WaterTemperature, WaterType::LakeReservoir),
            ["usgs", "coops", "ndbc"]
        );
        assert_eq!(
            names(M::LakeLevel, WaterType::LakeReservoir),
            ["usgs", "cwms", "rise"]
        );
        assert_eq!(
            names(M::Reservoir, WaterType::LakeReservoir),
            ["cwms", "rise"]
        );
        assert_eq!(names(M::Bloom, WaterType::LakeReservoir), ["cyan"]);
        assert_eq!(names(M::Bloom, WaterType::BayInshore), ["noaa-hab"]);
        assert_eq!(names(M::Cyanobacteria, WaterType::Pond), ["cyan"]);
        assert_eq!(
            names(M::Cyanobacteria, WaterType::Offshore),
            Vec::<String>::new()
        );
    }

    #[test]
    fn limits_come_from_the_groups_that_set_them() {
        let limits = measurement_limits(&Config::default().sources);
        let temp = limits
            .iter()
            .find(|l| l.measurement == M::WaterTemperature)
            .expect("water temperature");
        assert_eq!(temp.max_distance_m, Some(25_000.0));
        assert_eq!(temp.max_age_s, Some(10_800));
        assert!(limits.iter().all(|l| l.measurement != M::Tide));
    }

    #[test]
    fn weather_ids_are_the_weather_librarys_provider_ids() {
        for (id, provider) in [
            ("open-meteo", ProviderId::OpenMeteo),
            ("nws", ProviderId::Nws),
            ("openweather", ProviderId::OpenWeather),
            ("met-norway", ProviderId::MetNorway),
            ("pirate-weather", ProviderId::PirateWeather),
        ] {
            assert_eq!(provider_id(&SourceId::new(id)), Some(provider));
            assert_eq!(provider.id(), id);
        }
        assert_eq!(provider_id(&SourceId::new("open-meteo-marine")), None);
    }

    #[test]
    fn the_default_weather_chain_is_open_meteo_with_nws_alerts_in_the_us() {
        let group = &Config::default().sources.weather;
        let battery = Coordinates {
            latitude: 40.7,
            longitude: -74.02,
        };
        let chain = weather_chain(group, &battery);
        assert_eq!(ids(&chain.primary), [ProviderId::OpenMeteo]);
        assert_eq!(ids(&chain.supplements), [ProviderId::Nws]);

        let lisbon = Coordinates {
            latitude: 38.7,
            longitude: -9.14,
        };
        let chain = weather_chain(group, &lisbon);
        assert!(chain.supplements.is_empty());

        let keyed = SourceGroup {
            chain: vec![SourceId::new("openweather"), SourceId::new("tide-oracle")],
            max_distance_km: None,
            max_age_minutes: None,
        };
        let chain = weather_chain(&keyed, &battery);
        assert!(chain.primary.is_empty() && chain.supplements.is_empty());
        assert!(!weather_requirements().air_quality);
        // A week of hours, matching the daily forecast, so go/no-go reaches a trip's days.
        let req = weather_requirements();
        assert_eq!(req.hourly_hours, 168);
        assert!(u16::from(req.hourly_hours) >= u16::from(req.daily_days) * 24);
    }
}
