//! Drives the weather, water and storm sessions for a spot through the transport.
//!
//! This part maps the configuration onto the sessions: the registry of water and marine
//! sources the CLI runs, the measurements a spot's water type and cards need, each
//! measurement's configured chain and limits, and the weather providers built from
//! `[sources.weather]`.

#![allow(dead_code)] // The spot, stations and tides commands call these; they are wired in separately.

use std::collections::BTreeMap;
use std::sync::Arc;

use underbite_config::schema::Config;
use underbite_config::sources::{SourceGroup, SourcesConfig};
use underbite_core::discover::rank::{ChainSpec, Limits};
use underbite_core::model::{Measurement, Requirements, WaterType};
use underbite_core::provenance::SourceId;
use underbite_core::view::{CardId, MeasurementLimit};
use underbite_providers::coops::{Coops, CoopsHarmonics};
use underbite_providers::ndbc::Ndbc;
use underbite_providers::nwps::Nwps;
use underbite_providers::nws_marine::NwsMarine;
use underbite_providers::open_meteo_marine::OpenMeteoMarine;
use underbite_providers::request::Coordinates;
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

/// The water and marine sources this tool runs, by configuration id. Other ids in a chain
/// (sources not implemented yet) are skipped at run time.
pub(crate) const WATER_SOURCES: [SourceId; 7] = [
    SourceId::COOPS,
    SourceId::COOPS_HARMONICS,
    SourceId::USGS,
    SourceId::NDBC,
    SourceId::NWPS,
    SourceId::OPEN_METEO_MARINE,
    SourceId::NWS,
];

/// Sources that only make sense on salt water and the Great Lakes (gridded marine models).
const MARINE_ONLY: [SourceId; 1] = [SourceId::OPEN_METEO_MARINE];

/// The source registered under `id`, if this tool runs it.
pub(crate) fn water_source(id: &SourceId) -> Option<Arc<dyn Source>> {
    Some(match id.as_str() {
        "coops" => Arc::new(Coops),
        "coops-harmonics" => Arc::new(CoopsHarmonics),
        "usgs" => Arc::new(Usgs::new()),
        "ndbc" => Arc::new(Ndbc),
        "nwps" => Arc::new(Nwps),
        "open-meteo-marine" => Arc::new(OpenMeteoMarine),
        "nws" => Arc::new(NwsMarine),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Measurements, groups and chains
// ---------------------------------------------------------------------------------------------

/// Station observations of wind and pressure have no `[sources]` group: they come from CO-OPS
/// and NDBC stations, nearest first, and count only while fresh.
const OBSERVATION_CHAIN: [SourceId; 2] = [SourceId::COOPS, SourceId::NDBC];
/// How old a station's wind or pressure may be, minutes.
const OBSERVATION_MAX_AGE_MINUTES: u32 = 180;

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

/// The measurements one card shows on `water`. Flow means river flow: discharge at a tidal
/// gauge reverses with the tide, so tidal water takes tides and currents instead.
fn card_measurements(card: CardId, water: WaterType) -> Vec<Measurement> {
    use Measurement as M;
    let salt = is_salt(water);
    let marine = is_marine(water);
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
            WaterType::LakeReservoir | WaterType::Pond | WaterType::GreatLakes => {
                out.push(M::LakeLevel);
            }
            _ => {}
        },
        CardId::WaterTemp => out.push(M::WaterTemperature),
        CardId::Clarity => out.push(M::Turbidity),
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
pub(crate) fn requirements(config: &Config, water: WaterType) -> Requirements {
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

/// The `[sources]` table that configures `measurement`, by name; `None` for station
/// observations of wind and pressure, which have a fixed chain.
pub(crate) fn group_name(measurement: Measurement) -> Option<&'static str> {
    use Measurement as M;
    Some(match measurement {
        M::WaterTemperature => "water_temperature",
        M::Tide => "tide",
        M::TidalCurrent | M::OceanCurrent => "currents",
        M::Discharge | M::GageHeight | M::FlowForecast | M::FloodCategories => "flow",
        M::LakeLevel => "lake_level",
        M::Waves | M::WavesForecast => "waves",
        M::SeaSurfaceTemp | M::SeaSurfaceTempForecast => "sst",
        M::Clarity | M::Turbidity => "clarity",
        M::DissolvedOxygen
        | M::Conductivity
        | M::Salinity
        | M::Ph
        | M::Chlorophyll
        | M::Cyanobacteria => "water_quality",
        M::MarineForecast | M::MarineAlerts => "marine_forecast",
        M::ObservedWind | M::ObservedPressure => return None,
    })
}

/// The configured group of `measurement`, or the fixed observation chain.
fn group_of(sources: &SourcesConfig, measurement: Measurement) -> SourceGroup {
    let configured = group_name(measurement).and_then(|name| {
        sources
            .groups()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, g)| g.clone())
    });
    configured.unwrap_or_else(|| SourceGroup {
        chain: OBSERVATION_CHAIN.to_vec(),
        max_distance_km: None,
        max_age_minutes: Some(OBSERVATION_MAX_AGE_MINUTES),
    })
}

/// The chain and limits of every required measurement: the configured sources this tool runs,
/// in order, each once; gridded marine models only on marine water.
pub(crate) fn chain_specs(
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
                let runs = WATER_SOURCES.contains(&id);
                let fits = is_marine(water) || !MARINE_ONLY.contains(&id);
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

/// The user's distance and age limits per measurement, for greying values in the view.
pub(crate) fn measurement_limits(sources: &SourcesConfig) -> Vec<MeasurementLimit> {
    use Measurement as M;
    let all = [
        M::WaterTemperature,
        M::Discharge,
        M::GageHeight,
        M::LakeLevel,
        M::Turbidity,
        M::Tide,
        M::TidalCurrent,
        M::Waves,
        M::SeaSurfaceTemp,
        M::ObservedWind,
        M::ObservedPressure,
    ];
    all.into_iter()
        .filter_map(|m| {
            let group = group_of(sources, m);
            let limit = MeasurementLimit {
                measurement: m,
                max_distance_m: group.max_distance_km.map(|km| (km * 1_000.0) as f32),
                max_age_s: group.max_age_minutes.map(|min| i64::from(min) * 60),
            };
            (limit.max_distance_m.is_some() || limit.max_age_s.is_some()).then_some(limit)
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

/// What the weather session asks for: everything the cards can show, without air quality
/// (no card shows it).
pub(crate) fn weather_requirements() -> DataRequirements {
    DataRequirements {
        air_quality: false,
        ..DataRequirements::full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Measurement as M;
    use underbite_config::sources::KNOWN_SOURCES;

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
    fn chains_follow_the_configuration_and_skip_what_does_not_run() {
        let config = Config::default();
        let req = requirements(&config, WaterType::TidalRiverEstuary);
        let specs = chain_specs(&config.sources, &req, WaterType::TidalRiverEstuary);
        let temp = &specs[&M::WaterTemperature];
        let names: Vec<&str> = temp.sources.iter().map(SourceId::as_str).collect();
        assert_eq!(names, ["usgs", "coops", "ndbc", "open-meteo-marine"]);
        assert_eq!(temp.limits, Limits::from_config(Some(25.0), Some(180)));
        let wind = &specs[&M::ObservedWind];
        assert_eq!(wind.sources, OBSERVATION_CHAIN);
        assert_eq!(wind.limits.max_age_s, 180 * 60);
        assert_eq!(specs[&M::MarineAlerts].sources, [SourceId::NWS]);

        // Inland, the marine model is left out of the water temperature chain.
        let river = requirements(&config, WaterType::RiverStream);
        let specs = chain_specs(&config.sources, &river, WaterType::RiverStream);
        let names: Vec<&str> = specs[&M::WaterTemperature]
            .sources
            .iter()
            .map(SourceId::as_str)
            .collect();
        assert_eq!(names, ["usgs", "coops", "ndbc"]);
        assert_eq!(
            specs[&M::FloodCategories].sources,
            [SourceId::USGS, SourceId::NWPS]
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
    }
}
