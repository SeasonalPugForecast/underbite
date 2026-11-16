//! Drives the weather, water and storm sessions for a spot through the transport.
//!
//! The configuration maps onto the sessions: the registry of water and marine sources the CLI
//! runs, the measurements a spot's water type and cards need, each measurement's configured
//! chain and limits, and the weather providers built from `[sources.weather]`.
//!
//! Discovery comes first: the station lists (CO-OPS, NDBC, USGS sites and NWPS gauges in a box
//! around the rounded spot), the hydrography around a river, lake or estuary spot (3DHP), and
//! the ranking with the spot's pins and exclusions. NDBC stations kept on their list flags alone
//! are confirmed against their files. Lists, rankings, observed coverage and responses are
//! cached; coordinates leave the device only rounded by the fetch context, and the exact spot
//! is used on the device (distances, snapping to the hydrography).
//!
//! Then the sessions run one after the other on the same transport, one request at a time: the
//! water and marine session over the discovery plan, whether's weather session, the normals of
//! the latest USGS discharge or lake level for its day, and the storm-ahead check with nearby
//! hurricanes. The sky is computed for the spot's local days, and everything lands in one
//! `SpotSnapshot` with its
//! credits and the errors of the sources that were passed over. The snapshot and the call ledger
//! are saved; every request is logged only through `redact_url`.
#![allow(dead_code)] // The spot, stations and tides commands call these; they are wired in separately.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use underbite_config::schema::Config;
use underbite_config::sources::{SourceGroup, SourcesConfig};
use underbite_core::discover::cache::{DiscoveryCache, ObservedCoverage, StationChoices};
use underbite_core::discover::hydro::{self as hydrography, HydroOptions, Point};
use underbite_core::discover::rank::{
    ChainSpec, DEFAULT_STATIONS_PER_SOURCE, DiscoveryInput, Limits, LinkPlace, Plan, Ranking,
    StationKey, distance_m,
};
use underbite_core::model::{
    Coverage, Measurement, Requirements, SourceError as ModelSourceError, Spot, StationCandidate,
    WaterType,
};
use underbite_core::provenance::{Relation, SourceId, StationKind, StationRef};
use underbite_core::time::UnixTime;
use underbite_core::units::{Canonical, to_canonical};
use underbite_core::view::{CardId, MeasurementLimit};
use underbite_providers::coops::mdapi::{self, Metadata, StationList};
use underbite_providers::coops::{Coops, CoopsHarmonics};
use underbite_providers::hydro;
use underbite_providers::ndbc::{self, Ndbc};
use underbite_providers::nwps::{self, Nwps};
use underbite_providers::nws_marine::NwsMarine;
use underbite_providers::open_meteo_marine::OpenMeteoMarine;
use underbite_providers::request::{
    CacheHint, Coordinates, Place, Request, Resource, Response, RoundedPoint, Target, Window,
};
use underbite_providers::session::usage::{SharedUsage, UsageLedger};
use underbite_providers::source::{
    FetchContext, FragmentValue, Fragments, Source, SourceError, check_response,
};
use underbite_providers::usgs::{Usgs, sites};

use super::cache::{COOPS_STATION_LIST_TTL_S, Cache, STATION_LIST_TTL_S};
use super::transport::{Call, Fetched, Transport, TransportError, user_agent};
use jiff::tz::TimeZone;
use underbite_core::astro::solunar::{DayBounds, SolunarSettings, sky_conditions};
use underbite_core::model::{Percentile, StormWatch, WaterConditions};
use underbite_core::model::{SkyConditions, SourceErrorKind, SpotSnapshot};
use underbite_core::provenance::Attribution;
use underbite_core::time::standard_offset;
use underbite_core::time::{local_date, local_date_bounds, zone};
use underbite_providers::session::usage::OPEN_METEO;
use underbite_providers::session::{Attempt, Chain, Link, Session, SessionOutcome};
use underbite_providers::source::{Period, Secrets};
use underbite_providers::storms::{StormOptions, StormOutcome, StormWatchPlan};
use underbite_providers::usgs::statistics::Reading;
use whether_core::model::ProviderId;
use whether_core::model::WeatherSnapshot;
use whether_core::requirements::DataRequirements;
use whether_providers::Provider;
use whether_providers::nws::{self, Nws};
use whether_providers::open_meteo::{OpenMeteo, OpenMeteoSettings};
use whether_providers::regions::covers_us;
use whether_providers::{
    Attempt as WeatherAttempt, FetchContext as WeatherContext, FetchError, FetchSession,
    HttpResponse, ProviderError,
};

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

/// The chain and limits of every required measurement: the configured sources this tool runs
/// that supply the measurement, in order, each once; gridded marine models only on marine
/// water.
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
                let runs = water_source(&id).is_some_and(|s| s.measurements().contains(&m));
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

// ---------------------------------------------------------------------------------------------
// The host: transport, response cache and call ledger
// ---------------------------------------------------------------------------------------------

/// Cache namespaces.
const NS_STATIONS: &str = "stations";
const NS_DISCOVERY: &str = "discovery";
const NS_CHOICES: &str = "choices";
const NS_USAGE: &str = "usage";
const NS_SPOT: &str = "spot";
/// The key of the call ledger and of the observed coverage.
const LEDGER_KEY: &str = "ledger";
const COVERAGE_KEY: &str = "coverage";
/// How long a coverage check counts before a station is checked again, seconds.
const COVERAGE_MAX_AGE_S: i64 = 24 * 3_600;
/// Rounds of coverage checks and ranking one discovery runs at most: a check can promote
/// stations nobody has checked yet into the plan, and those are checked in the next round.
const COVERAGE_ROUNDS: usize = 3;

/// One fetch's way out: the transport, the response cache and the call ledger shared by the
/// sessions. Requests go out one at a time.
pub(crate) struct Host<'a> {
    transport: &'a dyn Transport,
    cache: Option<&'a Cache>,
    now: UnixTime,
    usage: SharedUsage,
    allow_metered: bool,
}

impl<'a> Host<'a> {
    /// A host at `now`, with the ledger saved in `cache` (an empty one without a cache).
    pub(crate) fn new(
        transport: &'a dyn Transport,
        cache: Option<&'a Cache>,
        now: UnixTime,
        allow_metered: bool,
    ) -> Self {
        let ledger = cache
            .and_then(|c| c.get::<UsageLedger>(NS_USAGE, LEDGER_KEY))
            .map(|stored| stored.value)
            .unwrap_or_default();
        Self {
            transport,
            cache,
            now,
            usage: SharedUsage::new(ledger),
            allow_metered,
        }
    }

    /// The ledger the sessions spend from.
    pub(crate) fn usage(&self) -> &SharedUsage {
        &self.usage
    }

    /// Saves the ledger; a metered request goes out only after this succeeds.
    fn save_usage(&self) -> Result<(), String> {
        let Some(cache) = self.cache else {
            return Ok(());
        };
        cache
            .put(NS_USAGE, LEDGER_KEY, &self.usage.snapshot(), self.now)
            .map_err(|e| format!("the call ledger was not saved: {e}"))
    }

    /// Sends `call`, or answers it from the response cache, and stores what may be stored.
    fn send(&self, call: &Call) -> Result<Fetched, TransportError> {
        if let Some(hit) = self.cache.and_then(|c| c.load_response(call, self.now)) {
            tracing::debug!(url = %call.redacted_url(), "cached");
            return Ok(hit);
        }
        tracing::debug!(url = %call.redacted_url(), "GET");
        let fetched = self.transport.send(call)?;
        tracing::debug!(
            status = fetched.status,
            bytes = fetched.body.len(),
            "response"
        );
        if let Some(cache) = self.cache
            && let Err(e) = cache.store_response(call, &fetched, self.now)
        {
            tracing::warn!(error = %e, "response not cached");
        }
        Ok(fetched)
    }

    /// Sends a request of `source` outside a session: a metered one spends from the ledger
    /// (saved first) unless the cache answers it.
    fn get(&self, source: &dyn Source, request: &Request) -> Result<Fetched, SourceError> {
        let call = Call::from(request);
        if let Some(hit) = self.cache.and_then(|c| c.load_response(&call, self.now)) {
            return Ok(hit);
        }
        if request.metered {
            let budget = source.budget().ok_or_else(|| {
                SourceError::Unsupported("a metered request without a budget".into())
            })?;
            if !self.allow_metered {
                return Err(SourceError::RateLimited {
                    retry_after_s: None,
                });
            }
            let now = self.now;
            self.usage
                .with(|ledger| ledger.try_spend(&source.id(), budget, now))
                .map_err(|quota| SourceError::RateLimited {
                    retry_after_s: Some(quota.retry_after_s(now)),
                })?;
            self.save_usage().map_err(SourceError::Transport)?;
        }
        self.send(&call)
            .map_err(|e| SourceError::Transport(e.to_string()))
    }

    fn cached<T: for<'de> Deserialize<'de>>(
        &self,
        namespace: &str,
        key: &str,
        ttl_s: i64,
    ) -> Option<T> {
        self.cache?.get_within(namespace, key, self.now, ttl_s)
    }

    fn stored<T: for<'de> Deserialize<'de>>(&self, namespace: &str, key: &str) -> Option<T> {
        self.cache?.get::<T>(namespace, key).map(|s| s.value)
    }

    fn keep<T: Serialize>(&self, namespace: &str, key: &str, value: &T) {
        if let Some(cache) = self.cache
            && let Err(e) = cache.put(namespace, key, value, self.now)
        {
            tracing::warn!(namespace, error = %e, "not cached");
        }
    }
}

/// The pins and exclusions of a spot.
pub(crate) fn load_choices(cache: &Cache, spot_id: &str) -> StationChoices {
    cache
        .get(NS_CHOICES, &Cache::spot_key(spot_id))
        .map(|stored| stored.value)
        .unwrap_or_default()
}

/// Saves the pins and exclusions of a spot.
pub(crate) fn save_choices(
    cache: &Cache,
    spot_id: &str,
    choices: &StationChoices,
    now: UnixTime,
) -> std::io::Result<()> {
    cache.put(NS_CHOICES, &Cache::spot_key(spot_id), choices, now)
}

/// A snapshot error for `source` that concerns no single measurement.
fn source_error(source: SourceId, what: &str, error: &SourceError) -> ModelSourceError {
    let mut e = error.to_model(source, None);
    e.message = format!("{what}: {}", e.message);
    e
}

// ---------------------------------------------------------------------------------------------
// Station lists
// ---------------------------------------------------------------------------------------------

/// Half the side of the USGS site search box, degrees (about 25 km).
const USGS_HALF_WIDTH_DEG: f64 = 0.25;

/// A station list as cached.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StoredList {
    candidates: Vec<StationCandidate>,
    /// Depth of current bins, metres.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bin_depths_m: Vec<(StationKey, f32)>,
    /// Parts that failed: the list is used but not cached, so the next fetch asks again.
    #[serde(skip)]
    gaps: Vec<SourceError>,
}

/// Every station near the spot from the lists of the sources in its chains.
#[derive(Debug, Default)]
struct Lists {
    candidates: Vec<StationCandidate>,
    bin_depths_m: BTreeMap<StationKey, f32>,
    /// Whether a list came from the network (rankings are redone).
    refreshed: bool,
    errors: Vec<ModelSourceError>,
}

impl Lists {
    fn add(&mut self, list: StoredList) {
        self.candidates.extend(list.candidates);
        self.bin_depths_m.extend(list.bin_depths_m);
    }
}

/// A target for list and lookup requests: the rounded spot, no measurement.
fn point_target(point: RoundedPoint, now: UnixTime) -> Target {
    Target {
        place: Place::Point(point),
        measurements: BTreeSet::new(),
        window: Window { from: now, to: now },
        datum: None,
    }
}

/// The cache key of a list around a rounded point: a hash, so no coordinates name a file.
fn area_key(list: &str, point: RoundedPoint) -> String {
    let (lat, lon) = point.fmt_param();
    format!("{list}-{}", Cache::spot_key(&format!("{lat},{lon}")))
}

/// The depth of each current bin in a CO-OPS `currentpredictions` list, metres. The list gives
/// feet (its `units`); the station parser keeps the bins but not their depths.
fn bin_depths(body: &[u8]) -> Result<Vec<(StationKey, f32)>, SourceError> {
    #[derive(Deserialize)]
    struct Body {
        units: Option<String>,
        stations: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        currbin: Option<u16>,
        depth: Option<f32>,
    }
    let body: Body = serde_json::from_slice(body).map_err(|e| SourceError::Parse(e.to_string()))?;
    let unit = match body.units.as_deref() {
        Some("metric") => "m",
        _ => "ft",
    };
    let mut out = Vec::new();
    for entry in body.stations {
        let (Some(bin), Some(depth)) = (entry.currbin, entry.depth) else {
            continue;
        };
        let Ok(Canonical::Length(metres)) = to_canonical(depth, unit) else {
            return Err(SourceError::Parse(format!("bin depth unit {unit}")));
        };
        out.push((
            StationKey::new(SourceId::COOPS, entry.id).with_bin(bin),
            metres,
        ));
    }
    Ok(out)
}

/// One CO-OPS list, with bin depths for the current list.
fn coops_list(
    host: &Host<'_>,
    list: StationList,
    target: &Target,
) -> Result<StoredList, SourceError> {
    let request = mdapi::stations_request(list, target.clone());
    let fetched = host.get(&Coops, &request)?;
    let response = fetched.as_response();
    let Metadata::Stations(candidates) = mdapi::parse(&request, &response)? else {
        return Err(SourceError::Parse("not a station list".into()));
    };
    let bin_depths_m = if list == StationList::CurrentPredictions {
        bin_depths(&fetched.body)?
    } else {
        Vec::new()
    };
    Ok(StoredList {
        candidates,
        bin_depths_m,
        gaps: Vec::new(),
    })
}

/// NDBC's active stations.
fn ndbc_list(host: &Host<'_>, target: &Target) -> Result<StoredList, SourceError> {
    let request = ndbc::stations::request(target.clone());
    let fetched = host.get(&Ndbc, &request)?;
    let body = check_response(&request, &fetched.as_response())?;
    let text = std::str::from_utf8(body).map_err(|e| SourceError::Parse(e.to_string()))?;
    Ok(StoredList {
        candidates: ndbc::stations::parse(text)?,
        bin_depths_m: Vec::new(),
        gaps: Vec::new(),
    })
}

/// USGS sites in the box around the spot with the series they serve.
fn usgs_list(
    host: &Host<'_>,
    target: &Target,
    ctx: &FetchContext,
) -> Result<StoredList, SourceError> {
    let usgs = Usgs::new();
    let mut stations: Vec<StationRef> = Vec::new();
    let mut gaps = Vec::new();
    // One request per site type; a type that fails leaves the others usable.
    for request in sites::locations_requests(target, USGS_HALF_WIDTH_DEG, ctx)? {
        let found = host.get(&usgs, &request).and_then(|fetched| {
            sites::parse_locations(check_response(&request, &fetched.as_response())?)
        });
        match found {
            Ok(found) => {
                for station in found {
                    if !stations.iter().any(|s| s.id == station.id) {
                        stations.push(station);
                    }
                }
            }
            Err(e) => gaps.push(e),
        }
    }
    let request = sites::metadata_request(target, USGS_HALF_WIDTH_DEG, ctx)?;
    let fetched = host.get(&usgs, &request)?;
    let offers = sites::parse_metadata(check_response(&request, &fetched.as_response())?)?;
    Ok(StoredList {
        candidates: sites::candidates(stations, &offers),
        bin_depths_m: Vec::new(),
        gaps,
    })
}

/// NWPS gauges in the box around the spot.
fn nwps_list(
    host: &Host<'_>,
    target: &Target,
    ctx: &FetchContext,
) -> Result<StoredList, SourceError> {
    let request = Nwps
        .plan(target, &Requirements::default(), ctx)?
        .into_iter()
        .next()
        .ok_or_else(|| SourceError::Unsupported("no gauge list request".into()))?;
    let fetched = host.get(&Nwps, &request)?;
    let body = check_response(&request, &fetched.as_response())?;
    Ok(StoredList {
        candidates: nwps::parse_gauges(body)?,
        bin_depths_m: Vec::new(),
        gaps: Vec::new(),
    })
}

/// A list from the cache while it is younger than `ttl_s`, else from `fetch`.
fn list(
    host: &Host<'_>,
    lists: &mut Lists,
    source: SourceId,
    key: &str,
    ttl_s: i64,
    fetch: impl FnOnce() -> Result<StoredList, SourceError>,
) {
    if let Some(stored) = host.cached::<StoredList>(NS_STATIONS, key, ttl_s) {
        lists.add(stored);
        return;
    }
    match fetch() {
        Ok(mut stored) => {
            for gap in std::mem::take(&mut stored.gaps) {
                tracing::warn!(%source, error = %gap, "station list incomplete");
                lists
                    .errors
                    .push(source_error(source.clone(), "station list", &gap));
            }
            if lists.errors.iter().all(|e| e.source != source) {
                host.keep(NS_STATIONS, key, &stored);
            }
            lists.refreshed = true;
            lists.add(stored);
        }
        Err(e) => {
            tracing::warn!(%source, error = %e, "station list");
            lists.errors.push(source_error(source, "station list", &e));
        }
    }
}

/// CO-OPS water-level stations on tidal water usually carry a water temperature sensor and a
/// weather station, which their list does not say: they are offered as secondary, unconfirmed
/// products, so a fetch tries them and records why when they have nothing recent.
fn with_station_observations(candidates: &mut [StationCandidate]) {
    for c in candidates.iter_mut().filter(|c| {
        c.station.source == SourceId::COOPS
            && c.station.kind == StationKind::WaterLevel
            && c.offers.contains_key(&Measurement::Tide)
    }) {
        for m in [
            Measurement::WaterTemperature,
            Measurement::ObservedWind,
            Measurement::ObservedPressure,
        ] {
            c.offers.entry(m).or_insert(Coverage {
                last_value_at: None,
                primary: false,
            });
        }
    }
}

/// The lists of every station source in `chains`.
fn station_lists(
    host: &Host<'_>,
    ctx: &FetchContext,
    point: RoundedPoint,
    chains: &BTreeMap<Measurement, ChainSpec>,
) -> Lists {
    let sources: BTreeSet<&SourceId> = chains.values().flat_map(|c| &c.sources).collect();
    let target = point_target(point, host.now);
    let mut lists = Lists::default();
    if sources.contains(&SourceId::COOPS) || sources.contains(&SourceId::COOPS_HARMONICS) {
        for kind in [
            StationList::TidePredictions,
            StationList::CurrentPredictions,
            StationList::WaterLevels,
        ] {
            let key = format!("coops-{}", kind.type_name());
            list(
                host,
                &mut lists,
                SourceId::COOPS,
                &key,
                COOPS_STATION_LIST_TTL_S,
                || coops_list(host, kind, &target),
            );
        }
        with_station_observations(&mut lists.candidates);
    }
    if sources.contains(&SourceId::NDBC) {
        list(
            host,
            &mut lists,
            SourceId::NDBC,
            "ndbc-active",
            STATION_LIST_TTL_S,
            || ndbc_list(host, &target),
        );
    }
    if sources.contains(&SourceId::USGS) {
        let key = area_key("usgs", point);
        list(
            host,
            &mut lists,
            SourceId::USGS,
            &key,
            STATION_LIST_TTL_S,
            || usgs_list(host, &target, ctx),
        );
    }
    if sources.contains(&SourceId::NWPS) {
        let key = area_key("nwps", point);
        list(
            host,
            &mut lists,
            SourceId::NWPS,
            &key,
            STATION_LIST_TTL_S,
            || nwps_list(host, &target, ctx),
        );
    }
    lists
}

// ---------------------------------------------------------------------------------------------
// Hydrography and discovery
// ---------------------------------------------------------------------------------------------

/// How far around the spot the hydrography is read, metres.
const HYDRO_RADIUS_M: f64 = 5_000.0;
/// NWPS gauges whose USGS site is looked up, nearest first.
const JOINED_GAUGES: usize = 3;

/// Water types whose stations are related to the spot through the river network: rivers,
/// lakes and the estuaries and bays they open into.
fn uses_hydrography(water: WaterType) -> bool {
    matches!(
        water,
        WaterType::RiverStream
            | WaterType::LakeReservoir
            | WaterType::Pond
            | WaterType::Ice
            | WaterType::TidalRiverEstuary
            | WaterType::BayInshore
    )
}

/// Every page of a 3DHP query.
fn hydro_pages<T>(
    host: &Host<'_>,
    first: hydro::HydroQuery,
    parse: fn(&Response<'_>) -> Result<Vec<T>, SourceError>,
) -> Result<Vec<T>, SourceError> {
    let mut out = Vec::new();
    let mut query = Some(first);
    while let Some(q) = query {
        let call = Call::get(q.url.clone()).with_cache(CacheHint {
            max_age: Some(q.max_age_s),
            ..CacheHint::default()
        });
        let fetched = host
            .send(&call)
            .map_err(|e| SourceError::Transport(e.to_string()))?;
        let response = fetched.as_response();
        out.extend(parse(&response)?);
        query = hydro::next_page(&q, &response)?;
    }
    Ok(out)
}

/// The relation of each station to the spot from the flowlines and waterbodies around it. A
/// station or spot off the mapped streams has no known relation and is left out; a `Nearby`
/// between two placed ones (a tributary, another river) is known and kept.
fn relations(
    host: &Host<'_>,
    ctx: &FetchContext,
    spot: &Spot,
    stations: &[StationRef],
) -> Result<BTreeMap<StationKey, Relation>, SourceError> {
    let at = Coordinates {
        latitude: spot.latitude,
        longitude: spot.longitude,
    };
    let flowlines = hydro_pages(
        host,
        hydro::flowlines_query(at, HYDRO_RADIUS_M, ctx)?,
        hydro::parse_flowlines,
    )?;
    let waterbodies = hydro_pages(
        host,
        hydro::waterbodies_query(at, HYDRO_RADIUS_M, ctx)?,
        hydro::parse_waterbodies,
    )?;
    let hints = hydrography::relations(
        Point::new(spot.latitude, spot.longitude),
        stations,
        &flowlines,
        &waterbodies,
        HydroOptions::default(),
    );
    let placed = |hint: &hydrography::RelationHint| {
        hint.relation != Relation::Nearby
            || !hint
                .reasons
                .iter()
                .any(|r| matches!(r, hydrography::HydroReason::NotOnMappedStream { .. }))
    };
    Ok(hints
        .into_iter()
        .filter(|(_, hint)| placed(hint))
        .map(|((source, id), hint)| (StationKey::new(source, id), hint.relation))
        .collect())
}

/// The USGS site each of the nearest NWPS gauges forecasts, from the gauges' metadata.
fn joins(
    host: &Host<'_>,
    ctx: &FetchContext,
    spot: &Spot,
    candidates: &[StationCandidate],
) -> BTreeMap<StationKey, StationKey> {
    let mut gauges: Vec<(f64, &StationRef)> = candidates
        .iter()
        .filter(|c| c.station.source == SourceId::NWPS)
        .map(|c| {
            let d = distance_m(
                spot.latitude,
                spot.longitude,
                c.station.latitude,
                c.station.longitude,
            );
            (d, &c.station)
        })
        .collect();
    gauges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = BTreeMap::new();
    for (d, station) in gauges.into_iter().take(JOINED_GAUGES) {
        let target = Target {
            place: Place::Station {
                station: station.clone(),
                distance_m: Some(d as f32),
                relation: Relation::Nearby,
            },
            measurements: BTreeSet::from([Measurement::FloodCategories]),
            window: Window {
                from: host.now,
                to: host.now,
            },
            datum: None,
        };
        let gauge = Nwps
            .plan(&target, &Requirements::default(), ctx)
            .and_then(|requests| {
                let request = requests
                    .into_iter()
                    .find(|r| r.resource == Resource::NwpsGauge)
                    .ok_or_else(|| SourceError::Unsupported("no gauge request".into()))?;
                let fetched = host.get(&Nwps, &request)?;
                nwps::parse_gauge(check_response(&request, &fetched.as_response())?)
            });
        match gauge {
            Ok(gauge) => {
                if let Some(site) = gauge.usgs_id {
                    out.insert(
                        StationKey::new(SourceId::NWPS, station.id.clone()),
                        StationKey::new(SourceId::USGS, format!("USGS-{site}")),
                    );
                }
            }
            Err(e) => tracing::debug!(gauge = %station.id, error = %e, "gauge metadata"),
        }
    }
    out
}

/// The NDBC station files that confirm what planned NDBC stations report.
const COVERAGE_FILE: Resource = Resource::NdbcStandard;
/// What the standard file can confirm.
const STANDARD_FILE_MEASUREMENTS: [Measurement; 4] = [
    Measurement::ObservedWind,
    Measurement::ObservedPressure,
    Measurement::WaterTemperature,
    Measurement::Waves,
];

/// Checks the planned NDBC stations kept on their list flags alone; returns whether what a
/// station is known to report changed.
fn check_coverage(
    host: &Host<'_>,
    ctx: &FetchContext,
    ranking: &Ranking,
    plan: &Plan,
    candidates: &[StationCandidate],
    observed: &mut ObservedCoverage,
) -> bool {
    let unconfirmed = ranking.unconfirmed();
    let planned: BTreeSet<StationKey> = plan
        .chains
        .values()
        .flat_map(|c| &c.links)
        .filter_map(|link| match &link.place {
            LinkPlace::Station { station, .. } => Some(StationKey::of(station)),
            LinkPlace::Point { .. } => None,
        })
        .filter(|key| unconfirmed.contains(key))
        .collect();
    let mut changed = false;
    for key in planned {
        let Some(candidate) = candidates
            .iter()
            .find(|c| StationKey::of(&c.station) == key)
        else {
            continue;
        };
        let checked: BTreeSet<Measurement> = candidate
            .offers
            .keys()
            .copied()
            .filter(|m| STANDARD_FILE_MEASUREMENTS.contains(m))
            .collect();
        let target = Target {
            place: Place::Station {
                station: candidate.station.clone(),
                distance_m: None,
                relation: Relation::Nearby,
            },
            measurements: checked.clone(),
            window: Window {
                from: host.now,
                to: host.now,
            },
            datum: None,
        };
        let found = Ndbc
            .coverage_request(&target, COVERAGE_FILE)
            .and_then(|request| {
                let fetched = host.get(&Ndbc, &request)?;
                let mut out = Fragments::new();
                // No standard file (not found): the station reports none of it.
                match Ndbc.parse(&request, &fetched.as_response(), ctx, &mut out) {
                    Ok(()) | Err(SourceError::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
                Ok(out
                    .fragments()
                    .iter()
                    .filter_map(|f| match &f.value {
                        FragmentValue::Coverage { coverage, .. } => {
                            Some((f.measurement, *coverage))
                        }
                        _ => None,
                    })
                    .collect::<BTreeMap<_, _>>())
            });
        match found {
            Ok(found) => changed |= observed.merge(&key, &checked, &found, host.now),
            Err(e) => tracing::debug!(station = %key.id, error = %e, "coverage check"),
        }
    }
    changed
}

/// Ranks, checks the planned stations nobody has checked yet and ranks again, until no planned
/// station is unchecked (or [`COVERAGE_ROUNDS`] have run), so the next fetch starts from the
/// same plan.
fn settle(
    host: &Host<'_>,
    ctx: &FetchContext,
    spot_id: &str,
    rankings: &mut DiscoveryCache,
    input: &mut DiscoveryInput,
) -> (Ranking, Plan) {
    let mut ranking = rankings.ranking(spot_id, input).clone();
    let mut plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
    for _ in 0..COVERAGE_ROUNDS {
        if !check_coverage(
            host,
            ctx,
            &ranking,
            &plan,
            &input.candidates,
            &mut input.observed,
        ) {
            return (ranking, plan);
        }
        ranking = rankings.ranking(spot_id, input).clone();
        plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
    }
    tracing::debug!(
        rounds = COVERAGE_ROUNDS,
        "coverage checks left for the next fetch"
    );
    (ranking, plan)
}

/// What discovery found for a spot.
#[derive(Debug)]
pub(crate) struct Discovery {
    /// The stations per measurement, with reasons.
    pub(crate) ranking: Ranking,
    /// The links per measurement a session takes.
    pub(crate) plan: Plan,
    /// Lists or hydrography that failed.
    pub(crate) errors: Vec<ModelSourceError>,
    /// Whether the hydrography related stations to the spot (its credit is due).
    pub(crate) hydrography: bool,
}

/// Ranks the stations around `spot` for `chains`: lists, hydrography, pins and exclusions,
/// cached rankings and coverage checks.
pub(crate) fn discover(
    host: &Host<'_>,
    ctx: &FetchContext,
    spot: &Spot,
    chains: BTreeMap<Measurement, ChainSpec>,
) -> Discovery {
    let point = ctx.round(Coordinates {
        latitude: spot.latitude,
        longitude: spot.longitude,
    });
    let lists = station_lists(host, ctx, point, &chains);
    let mut errors = lists.errors;
    let reach_m = chains
        .values()
        .map(|c| c.limits.max_distance_m)
        .fold(0.0, f64::max);
    let mut nearby: Vec<StationRef> = Vec::new();
    for c in &lists.candidates {
        let d = distance_m(
            spot.latitude,
            spot.longitude,
            c.station.latitude,
            c.station.longitude,
        );
        let listed = nearby
            .iter()
            .any(|s| s.source == c.station.source && s.id == c.station.id);
        if d <= reach_m && !listed {
            nearby.push(c.station.clone());
        }
    }
    let mut hydrography = false;
    let relations = if uses_hydrography(spot.water_type) && !nearby.is_empty() {
        match relations(host, ctx, spot, &nearby) {
            Ok(r) => {
                hydrography = true;
                r
            }
            Err(e) => {
                tracing::warn!(error = %e, "hydrography");
                errors.push(source_error(hydro::SOURCE, "hydrography", &e));
                BTreeMap::new()
            }
        }
    } else {
        BTreeMap::new()
    };
    let joins = if chains.values().any(|c| c.sources.contains(&SourceId::NWPS)) {
        joins(host, ctx, spot, &lists.candidates)
    } else {
        BTreeMap::new()
    };
    let spot_key = Cache::spot_key(&spot.id);
    let mut observed: ObservedCoverage =
        host.stored(NS_DISCOVERY, COVERAGE_KEY).unwrap_or_default();
    observed.prune(host.now, COVERAGE_MAX_AGE_S);
    let mut rankings: DiscoveryCache = host.stored(NS_DISCOVERY, &spot_key).unwrap_or_default();
    if lists.refreshed {
        rankings.invalidate(&spot.id);
    }
    let mut input = DiscoveryInput {
        latitude: spot.latitude,
        longitude: spot.longitude,
        now: host.now,
        candidates: lists.candidates,
        chains,
        choices: host
            .cache
            .map(|c| load_choices(c, &spot.id))
            .unwrap_or_default(),
        observed,
        relations,
        joins,
        bin_depths_m: lists.bin_depths_m,
    };
    let (ranking, plan) = settle(host, ctx, &spot.id, &mut rankings, &mut input);
    host.keep(NS_DISCOVERY, COVERAGE_KEY, &input.observed);
    host.keep(NS_DISCOVERY, &spot_key, &rankings);
    Discovery {
        ranking,
        plan,
        errors,
        hydrography,
    }
}

// ---------------------------------------------------------------------------------------------
// The water session
// ---------------------------------------------------------------------------------------------

/// The session's chains from the discovery plan: each link's source from the registry and its
/// place (a point link is the spot rounded by the context).
fn session_chains(
    plan: &Plan,
    requirements: &Requirements,
    ctx: &FetchContext,
) -> BTreeMap<Measurement, Chain> {
    let window = Window::around(ctx.now, requirements);
    plan.chains
        .iter()
        .map(|(&m, planned)| {
            let links = planned
                .links
                .iter()
                .filter_map(|link| {
                    let source = water_source(&link.source)?;
                    let place = match &link.place {
                        LinkPlace::Station {
                            station,
                            distance_m,
                            relation,
                        } => Place::Station {
                            station: station.clone(),
                            distance_m: Some(*distance_m),
                            relation: *relation,
                        },
                        LinkPlace::Point {
                            latitude,
                            longitude,
                            ..
                        } => Place::Point(ctx.round(Coordinates {
                            latitude: *latitude,
                            longitude: *longitude,
                        })),
                    };
                    let target = Target {
                        place,
                        measurements: BTreeSet::from([m]),
                        window,
                        datum: None,
                    };
                    Some(Link { source, target })
                })
                .collect();
            let chain = Chain {
                links,
                max_age_s: Some(planned.max_age_s),
            };
            (m, chain)
        })
        .collect()
}

/// Runs the water session to its end, one request at a time.
fn run_session(host: &Host<'_>, mut session: Session) -> SessionOutcome {
    while let Some(request) = session.next_request() {
        let request = request.clone();
        if request.metered
            && let Err(e) = host.save_usage()
        {
            tracing::warn!(error = %e, "metered request not sent");
            session.on_transport_error(&e);
            continue;
        }
        match host.send(&Call::from(&request)) {
            Ok(fetched) => session.on_response(&fetched.as_response()),
            Err(e) => session.on_transport_error(&e.to_string()),
        }
    }
    session.finish(host.now)
}

// ---------------------------------------------------------------------------------------------
// Weather
// ---------------------------------------------------------------------------------------------

/// The weather library's response for a fetched one.
fn weather_response(fetched: &Fetched) -> HttpResponse<'_> {
    HttpResponse {
        status: fetched.status,
        body: &fetched.body,
        date: fetched.date,
        expires: fetched.expires,
        last_modified: fetched.last_modified.as_deref(),
    }
}

/// The snapshot's kind for a weather provider's error.
fn weather_error_kind(error: &ProviderError) -> SourceErrorKind {
    match error {
        ProviderError::RateLimited { .. } | ProviderError::Quota(_) => SourceErrorKind::RateLimited,
        ProviderError::Http { status: 404, .. } => SourceErrorKind::NotFound,
        ProviderError::Malformed(_) => SourceErrorKind::Parse,
        ProviderError::Transport(_) => SourceErrorKind::Transport,
        _ => SourceErrorKind::Other,
    }
}

/// What the weather session produced.
struct Weather {
    snapshot: Option<WeatherSnapshot>,
    errors: Vec<ModelSourceError>,
    attribution: Vec<Attribution>,
}

/// Runs whether's session over `chain` at the rounded spot. Every call to Open-Meteo is counted
/// in the ledger it shares with Open-Meteo Marine and the storm check (one limit per address).
fn fetch_weather(
    host: &Host<'_>,
    chain: &WeatherChain,
    loc: Coordinates,
    user_agent: &str,
) -> Weather {
    let primary: Vec<&dyn Provider> = chain.primary.iter().map(AsRef::as_ref).collect();
    let supplements: Vec<&dyn Provider> = chain.supplements.iter().map(AsRef::as_ref).collect();
    let ctx = WeatherContext {
        now: host.now,
        user_agent: user_agent.to_string(),
        utc_offset_hint: None,
        if_modified_since: None,
        language: "en".to_string(),
    };
    let mut session = FetchSession::new(&primary, &supplements, loc, weather_requirements(), ctx);
    while let Some(request) = session.next_request() {
        let call = Call::from(request);
        let cached = host.cache.and_then(|c| c.load_response(&call, host.now));
        if cached.is_none() && call.url.contains("open-meteo.com") {
            let now = host.now;
            host.usage.with(|l| l.count(&OPEN_METEO, Period::Day, now));
            if let Err(e) = host.save_usage() {
                tracing::warn!(error = %e, "weather request not sent");
                session.on_transport_error(&e);
                continue;
            }
        }
        let fetched = cached.map_or_else(|| host.send(&call), Ok);
        match fetched {
            Ok(fetched) => session.on_response(&weather_response(&fetched)),
            Err(e) => session.on_transport_error(&e.to_string()),
        }
    }
    let attempts = match session.finish(host.now) {
        Ok((snapshot, attempts)) => {
            let attribution = std::iter::once(snapshot.provider)
                .chain(snapshot.supplemented_by.iter().copied())
                .filter_map(|id| {
                    let provider = chain
                        .primary
                        .iter()
                        .chain(&chain.supplements)
                        .find(|p| p.id() == id)?;
                    let credit = provider.attribution();
                    Some(Attribution {
                        source: SourceId::new(id.id()),
                        credit: credit.text.to_string(),
                        url: Some(credit.url.to_string()),
                    })
                })
                .collect();
            return Weather {
                errors: weather_errors(&attempts),
                snapshot: Some(snapshot),
                attribution,
            };
        }
        Err(FetchError::AllProvidersFailed(attempts)) => attempts,
        Err(e) => {
            let error = ModelSourceError {
                source: SourceId::new(ProviderId::OpenMeteo.id()),
                measurement: None,
                kind: SourceErrorKind::Other,
                message: format!("weather: {e}"),
            };
            return Weather {
                snapshot: None,
                errors: vec![error],
                attribution: Vec::new(),
            };
        }
    };
    let mut errors = weather_errors(&attempts);
    if attempts.is_empty() {
        errors.push(ModelSourceError {
            source: SourceId::new("weather"),
            measurement: None,
            kind: SourceErrorKind::Other,
            message: "weather: no weather provider configured".into(),
        });
    }
    Weather {
        snapshot: None,
        errors,
        attribution: Vec::new(),
    }
}

/// The failed attempts of the weather session as snapshot errors.
fn weather_errors(attempts: &[WeatherAttempt]) -> Vec<ModelSourceError> {
    attempts
        .iter()
        .filter_map(|a| {
            let error = a.error.as_ref()?;
            Some(ModelSourceError {
                source: SourceId::new(a.provider.id()),
                measurement: None,
                kind: weather_error_kind(error),
                message: format!("weather {:?}: {error}", a.endpoint).to_lowercase(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Time zone and sky
// ---------------------------------------------------------------------------------------------

/// The spot's time zone: the one stored with the spot (configured, or given with `--tz`), else
/// the zone the forecast resolved for the place. `None` when neither names a known zone.
fn time_zone(spot: &Spot, weather: Option<&WeatherSnapshot>) -> Option<(String, TimeZone)> {
    let stored = Some(spot.time_zone.as_str()).filter(|name| !name.is_empty());
    let forecast = weather.and_then(|w| w.location.tz_name.as_deref());
    [stored, forecast]
        .into_iter()
        .flatten()
        .find_map(|name| zone(name).ok().map(|tz| (name.to_string(), tz)))
}

/// Sun, moon and solunar periods for `days` local days from today.
fn sky(
    spot: &Spot,
    tz: &TimeZone,
    now: UnixTime,
    days: u8,
    settings: SolunarSettings,
) -> SkyConditions {
    let mut bounds = Vec::new();
    let mut date = local_date(now, tz);
    for _ in 0..days {
        let Ok(day) = date else { break };
        let Ok((start, end)) = local_date_bounds(day, tz) else {
            break;
        };
        bounds.push(DayBounds {
            date: day,
            start,
            end,
        });
        date = local_date(end, tz);
    }
    sky_conditions(spot.latitude, spot.longitude, &bounds, settings)
}

// ---------------------------------------------------------------------------------------------
// Storms and normals
// ---------------------------------------------------------------------------------------------

/// Runs the storm-ahead check and the hurricane watch for `spot`, given its previous watch.
fn fetch_storms(
    host: &Host<'_>,
    ctx: &FetchContext,
    spot: &Spot,
    previous: Option<&StormWatch>,
    hurricane_distance_km: f64,
) -> StormOutcome {
    let options = StormOptions {
        hurricane_distance_km,
        allow_metered: host.allow_metered,
    };
    let mut plan = StormWatchPlan::new(spot, previous, &options, ctx, Some(host.usage.clone()));
    while let Some(request) = plan.next_request() {
        let call = Call::from(&request.http).with_cache(request.cache);
        if request.http.metered
            && let Err(e) = host.save_usage()
        {
            tracing::warn!(error = %e, "storm request not sent");
            plan.on_transport_error(&e);
            continue;
        }
        match host.send(&call) {
            Ok(fetched) => plan.on_response(&fetched.as_response()),
            Err(e) => plan.on_transport_error(&e.to_string()),
        }
    }
    plan.finish()
}

/// Where the latest discharge or lake level stands among the normals of its day, from the
/// USGS Statistics API (or the site's daily means when it has none). Only values from a USGS
/// site have normals; the day is the site's standard-time day in the spot's zone.
fn fetch_normals(
    host: &Host<'_>,
    ctx: &FetchContext,
    water: &mut WaterConditions,
    tz: Option<&TimeZone>,
) -> Vec<ModelSourceError> {
    let mut errors = Vec::new();
    let level_datum = water.level.as_ref().map(|h| h.datum);
    let wanted = [
        (Measurement::Discharge, water.discharge_m3s.as_ref(), None),
        (
            Measurement::LakeLevel,
            water.level.as_ref().map(|h| &h.series),
            level_datum,
        ),
    ];
    let mut found: Vec<(Measurement, Percentile)> = Vec::new();
    for (measurement, series, datum) in wanted {
        let Some(series) = series else { continue };
        let (Some(latest), Some(station)) = (series.latest(), &series.provenance().station) else {
            continue;
        };
        if station.source != SourceId::USGS {
            continue;
        }
        let Some(offset) = tz.and_then(|tz| {
            let date = local_date(latest.time, tz).ok()?;
            standard_offset(tz, date).ok()
        }) else {
            continue;
        };
        let provenance = series.provenance();
        let target = Target {
            place: Place::Station {
                station: station.clone(),
                distance_m: provenance.distance_m,
                relation: provenance.relation,
            },
            measurements: BTreeSet::from([measurement]),
            window: Window {
                from: host.now,
                to: host.now,
            },
            datum,
        };
        let reading = Reading {
            measurement,
            datum,
            value: latest.value,
            at: latest.time,
        };
        let usgs = Usgs::new();
        let first = match usgs.statistics_request(&target, &reading, offset, ctx) {
            Ok(request) => request,
            Err(e) => {
                errors.push(e.to_model(SourceId::USGS, Some(measurement)));
                continue;
            }
        };
        let mut queue = vec![first];
        let mut asked = 0;
        while let Some(request) = queue.pop() {
            asked += 1;
            let mut out = Fragments::new();
            let next = host.get(&usgs, &request).and_then(|fetched| {
                let response = fetched.as_response();
                usgs.parse(&request, &response, ctx, &mut out)?;
                usgs.follow_up(&request, &response, ctx)
            });
            match next {
                Ok(next) if asked <= usgs.max_follow_ups() => queue.extend(next),
                Ok(_) => {}
                Err(e) => errors.push(e.to_model(SourceId::USGS, Some(measurement))),
            }
            for fragment in out.fragments() {
                if let FragmentValue::Percentile(p) = &fragment.value {
                    found.push((fragment.measurement, p.clone()));
                }
            }
        }
    }
    for (measurement, percentile) in found {
        match measurement {
            Measurement::Discharge => water.discharge_percentile_for_date = Some(percentile),
            Measurement::LakeLevel => water.level_percentile_for_date = Some(percentile),
            _ => {}
        }
    }
    errors
}

// ---------------------------------------------------------------------------------------------
// One spot, end to end
// ---------------------------------------------------------------------------------------------

/// How a fetch runs, besides the spot and the configuration.
#[derive(Clone, Debug)]
pub(crate) struct FetchOptions {
    /// Whether metered calls may be spent (USGS, Open-Meteo Marine, the storm point).
    pub(crate) allow_metered: bool,
    /// Keys the sources may send.
    pub(crate) secrets: Secrets,
    /// `underbite/<version>`.
    pub(crate) user_agent: String,
}

impl FetchOptions {
    /// Metered calls allowed, the USGS key from `USGS_API_KEY` (read through `lookup`).
    pub(crate) fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            allow_metered: true,
            secrets: Secrets {
                usgs_api_key: lookup("USGS_API_KEY").filter(|k| !k.trim().is_empty()),
            },
            user_agent: user_agent(),
        }
    }
}

/// A spot's fetch: the snapshot and how it came about.
#[derive(Debug)]
pub(crate) struct SpotFetch {
    /// Everything known about the spot.
    pub(crate) snapshot: SpotSnapshot,
    /// The stations per measurement with the reasons for their places.
    pub(crate) ranking: Ranking,
    /// The links the water session took.
    pub(crate) plan: Plan,
    /// Every water request, in order, with what became of it.
    pub(crate) attempts: Vec<Attempt>,
}

/// Fetches everything a spot's cards need at `now`: discovery, the water and marine session,
/// the weather, the sky; caches updated and the call ledger saved. The USGS key comes from
/// `USGS_API_KEY`.
pub(crate) fn fetch_spot(
    spot: &Spot,
    config: &Config,
    now: UnixTime,
    transport: &dyn Transport,
    cache: Option<&Cache>,
) -> SpotSnapshot {
    let options = FetchOptions::from_env(|name| std::env::var(name).ok());
    fetch_spot_with(spot, config, now, transport, cache, &options).snapshot
}

/// [`fetch_spot`] with explicit options, returning how the snapshot came about too.
pub(crate) fn fetch_spot_with(
    spot: &Spot,
    config: &Config,
    now: UnixTime,
    transport: &dyn Transport,
    cache: Option<&Cache>,
    options: &FetchOptions,
) -> SpotFetch {
    let host = Host::new(transport, cache, now, options.allow_metered);
    let ctx = FetchContext::new(now, options.user_agent.clone())
        .with_coordinate_precision(config.sources.coordinate_precision)
        .with_secrets(options.secrets.clone());
    let rounded = ctx
        .round(Coordinates {
            latitude: spot.latitude,
            longitude: spot.longitude,
        })
        .coordinates();
    let requirements = requirements(config, spot.water_type);
    let chains = chain_specs(&config.sources, &requirements, spot.water_type);
    let previous: Option<SpotSnapshot> = host.stored(NS_SPOT, &Cache::spot_key(&spot.id));
    let discovery = discover(&host, &ctx, spot, chains);

    let session = Session::new(
        session_chains(&discovery.plan, &requirements, &ctx),
        requirements.clone(),
        ctx.clone(),
    )
    .with_usage(host.usage.clone())
    .allow_metered(options.allow_metered);
    let mut water = run_session(&host, session);

    let weather_chain = weather_chain(&config.sources.weather, &rounded);
    let weather = fetch_weather(&host, &weather_chain, rounded, &options.user_agent);

    let zone = time_zone(spot, weather.snapshot.as_ref());
    let normals = fetch_normals(
        &host,
        &ctx,
        &mut water.water,
        zone.as_ref().map(|(_, tz)| tz),
    );
    // The storm group's chain switches the check off when empty; the plan runs both parts.
    let storms = (!config.sources.storm.chain.is_empty()).then(|| {
        let previous = previous.as_ref().and_then(|p| p.storm.as_ref());
        fetch_storms(
            &host,
            &ctx,
            spot,
            previous,
            config.storms.hurricane_distance_km,
        )
    });

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

    host.keep(NS_SPOT, &Cache::spot_key(&spot.id), &snapshot);
    if let Err(e) = host.save_usage() {
        tracing::warn!(error = %e, "call ledger");
    }
    SpotFetch {
        snapshot,
        ranking: discovery.ranking,
        plan: discovery.plan,
        attempts: water.attempts,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::transport::Replay;
    use super::*;
    use Measurement as M;
    use underbite_config::sources::KNOWN_SOURCES;
    use underbite_core::model::MarineConditions;
    use underbite_core::time::parse_rfc3339;

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

    fn now() -> UnixTime {
        parse_rfc3339("2026-11-08T14:30:00Z").expect("time")
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
    }

    fn replay(scenario: &str) -> Replay {
        Replay::load(&fixtures().join("replay").join(scenario)).expect("routes")
    }

    fn battery() -> Spot {
        Spot {
            id: "battery".into(),
            name: "The Battery".into(),
            latitude: 40.7033,
            longitude: -74.0170,
            water_type: WaterType::TidalRiverEstuary,
            water_body: None,
            time_zone: "America/New_York".into(),
        }
    }

    fn croton() -> Spot {
        Spot {
            id: "croton".into(),
            name: "Croton River below the dam".into(),
            latitude: 41.2252,
            longitude: -73.8588,
            water_type: WaterType::RiverStream,
            water_body: None,
            time_zone: "America/New_York".into(),
        }
    }

    fn ctx() -> FetchContext {
        FetchContext::new(now(), "underbite/test")
    }

    fn discovered(spot: &Spot, transport: &Replay, cache: &Cache) -> Discovery {
        let config = Config::default();
        let host = Host::new(transport, Some(cache), now(), true);
        let req = requirements(&config, spot.water_type);
        let chains = chain_specs(&config.sources, &req, spot.water_type);
        discover(&host, &ctx(), spot, chains)
    }

    fn first_link(d: &Discovery, m: Measurement) -> (String, String, Option<u16>, Relation) {
        let link = &d.plan.chains[&m].links[0];
        match &link.place {
            LinkPlace::Station {
                station, relation, ..
            } => (
                link.source.to_string(),
                station.id.clone(),
                station.bin,
                *relation,
            ),
            LinkPlace::Point { relation, .. } => {
                (link.source.to_string(), "point".into(), None, *relation)
            }
        }
    }

    #[test]
    fn bin_depths_are_read_in_metres() {
        let body = std::fs::read(
            fixtures()
                .join("raw/coops/coops-mdapi-stations-currentpredictions-newyork-trimmed.json"),
        )
        .expect("fixture");
        let depths: BTreeMap<StationKey, f32> =
            bin_depths(&body).expect("depths").into_iter().collect();
        let key = StationKey::new(SourceId::COOPS, "NYH1927").with_bin(13);
        assert!(
            (depths[&key] - 7.0 * 0.3048).abs() < 1e-4,
            "{}",
            depths[&key]
        );
        assert!(bin_depths(b"{}").is_err());
    }

    #[test]
    fn battery_discovery_takes_the_battery_and_the_hudson_river_entrance_current_bin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let transport = replay("battery");
        let d = discovered(&battery(), &transport, &cache);
        assert!(d.errors.is_empty(), "{:#?}", d.errors);
        assert!(d.hydrography);
        assert_eq!(first_link(&d, M::Tide).1, "8518750");
        let current = first_link(&d, M::TidalCurrent);
        assert_eq!((current.1.as_str(), current.2), ("NYH1927", Some(13)));
        // With the hydrography's geometry the stations on the harbour's own water rank by distance.
        let second = &d.plan.chains[&M::TidalCurrent].links[1];
        assert!(
            matches!(&second.place, LinkPlace::Station { station, relation, .. }
            if station.id == "NYH1920" && station.bin == Some(12)
                && *relation == Relation::SameWaterBody)
        );
        assert_eq!(first_link(&d, M::MarineAlerts).0, "nws");
        assert_eq!(first_link(&d, M::WavesForecast).0, "open-meteo-marine");
        // CO-OPS water levels stand in for the station's own temperature sensor.
        assert_eq!(first_link(&d, M::WaterTemperature).1, "8518750");

        // Discovery checks the planned buoys' files until its plan settles, the stations a
        // check promotes included: the next run starts from the same plan, checks no file and
        // takes the lists from the cache.
        let buoys = |d: &Discovery| -> Vec<String> {
            d.plan.chains[&M::WaterTemperature]
                .links
                .iter()
                .filter(|l| l.source == SourceId::NDBC)
                .filter_map(|l| match &l.place {
                    LinkPlace::Station { station, .. } => Some(station.id.clone()),
                    LinkPlace::Point { .. } => None,
                })
                .collect()
        };
        assert_eq!(buoys(&d), ["batn6", "robn4", "mhrn6"]);
        let sent = transport.seen().len();
        let again = discovered(&battery(), &transport, &cache);
        assert_eq!(again.plan, d.plan);
        assert_eq!(first_link(&again, M::TidalCurrent).1, "NYH1927");
        let lists = [
            "stations.json",
            "activestations.xml",
            "monitoring-locations",
            "realtime2/",
        ];
        assert!(
            transport.seen()[sent..]
                .iter()
                .all(|u| lists.iter().all(|l| !u.contains(l))),
            "{:#?}",
            &transport.seen()[sent..]
        );
    }

    #[test]
    fn croton_discovery_takes_the_dam_gauge() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let transport = replay("croton");
        let d = discovered(&croton(), &transport, &cache);
        // No estuary sites are listed around Croton: an empty answer, not an error.
        assert_eq!(d.errors, [], "{:#?}", d.errors);
        for m in [M::Discharge, M::GageHeight] {
            let link = first_link(&d, m);
            assert_eq!(
                (link.0.as_str(), link.1.as_str()),
                ("usgs", "USGS-01375000")
            );
        }
        assert!(!d.plan.chains.contains_key(&M::Tide));
        // The gauge list has no USGS ids: Katonah's own metadata names its site.
        let katonah = &d.ranking.measurements[&M::FloodCategories].stations[0];
        assert_eq!(katonah.station.id, "KTNN6");
        assert!(katonah.reasons.iter().any(|r| matches!(r,
            underbite_core::discover::rank::Reason::ForecastGaugeFor { site } if site.id == "USGS-01374901")));
        // The hydrography around the dam does not reach Katonah: kept on distance, as nearby.
        let nearby = underbite_core::discover::rank::Reason::Relation {
            relation: Relation::Nearby,
        };
        assert_eq!(katonah.relation, Relation::Nearby);
        assert!(katonah.reasons.contains(&nearby), "{:?}", katonah.reasons);
    }

    fn fetched(spot: &Spot, config: &Config, scenario: &str, cache: &Cache) -> (SpotFetch, Replay) {
        let transport = replay(scenario);
        let options = FetchOptions {
            allow_metered: true,
            secrets: Secrets::default(),
            user_agent: "underbite/test".into(),
        };
        let out = fetch_spot_with(spot, config, now(), &transport, Some(cache), &options);
        (out, transport)
    }

    /// Fails unless every coordinate `url` sends has at most 2 decimals. A 3DHP query sends an
    /// envelope or a point around the rounded spot: its centre is checked.
    fn assert_rounded(url: &str) {
        let on_grid = |v: f64| ((v * 100.0).round() - v * 100.0).abs() < 1e-6;
        if url.contains("3DHP_all") {
            let geometry = url
                .split_once('?')
                .and_then(|(_, q)| q.split('&').find_map(|p| p.strip_prefix("geometry=")))
                .expect("geometry");
            let n: Vec<f64> = geometry
                .split(',')
                .map(|v| v.parse().expect("number"))
                .collect();
            let centre = match n[..] {
                [x0, y0, x1, y1] => [f64::midpoint(x0, x1), f64::midpoint(y0, y1)],
                [x, y] => [x, y],
                _ => panic!("{url}"),
            };
            assert!(centre.into_iter().all(on_grid), "{url}");
            return;
        }
        for token in url.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')) {
            if let Some((_, fraction)) = token.split_once('.') {
                let digits = fraction.trim_end_matches('0');
                assert!(
                    digits.len() <= 2 || !digits.bytes().all(|b| b.is_ascii_digit()),
                    "{url}: {token}"
                );
            }
        }
    }

    fn errors(snapshot: &SpotSnapshot) -> Vec<(String, Option<Measurement>, SourceErrorKind)> {
        snapshot
            .errors
            .iter()
            .map(|e| (e.source.to_string(), e.measurement, e.kind))
            .collect()
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
    fn the_battery_end_to_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let (out, transport) = fetched(&battery(), &Config::default(), "battery", &cache);
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

        // The CO-OPS stations' water temperature ends on 7 November, older than 180 minutes, and
        // they report no wind; each is recorded before the buoys answer.
        let stale = (
            "coops".to_string(),
            Some(M::WaterTemperature),
            SourceErrorKind::Stale,
        );
        let no_wind = (
            "coops".to_string(),
            Some(M::ObservedWind),
            SourceErrorKind::NoData,
        );
        assert_eq!(
            errors(s),
            [
                stale.clone(),
                stale,
                no_wind.clone(),
                no_wind.clone(),
                no_wind
            ]
        );
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
        for url in transport.seen() {
            assert_rounded(&url);
        }

        // The snapshot and the ledger are saved: Marine and the forecast count against
        // Open-Meteo's one limit.
        let saved: SpotSnapshot = cache
            .get(NS_SPOT, &Cache::spot_key("battery"))
            .expect("saved")
            .value;
        assert_eq!(&saved, s);
        let ledger: UsageLedger = cache.get(NS_USAGE, LEDGER_KEY).expect("ledger").value;
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
    fn the_storm_point_is_reused_for_three_hours() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let (first, _) = fetched(&battery(), &Config::default(), "battery", &cache);
        let transport = replay("battery");
        let options = FetchOptions {
            allow_metered: true,
            secrets: Secrets::default(),
            user_agent: "underbite/test".into(),
        };
        let later = UnixTime(now().0 + 3_600);
        let again = fetch_spot_with(
            &battery(),
            &Config::default(),
            later,
            &transport,
            Some(&cache),
            &options,
        );
        assert!(
            transport.seen().iter().all(|u| !u.contains("&models=")),
            "{:#?}",
            transport.seen()
        );
        let (a, b) = (
            first.snapshot.storm.expect("first"),
            again.snapshot.storm.expect("again"),
        );
        assert_eq!(a.point, b.point);
        assert_eq!(b.checked_at, later);
    }

    #[test]
    fn croton_discharge_takes_its_place_among_the_normals_of_8_november() {
        use underbite_core::provenance::{Provenance, Quality, UsgsSiteType, ValueKind};
        use underbite_core::series::{Sample, Series};
        const CFS: f32 = 0.028_316_846;
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let transport = replay("croton");
        let host = Host::new(&transport, Some(&cache), now(), true);
        let dam = StationRef {
            source: SourceId::USGS,
            id: "USGS-01375000".into(),
            name: "CROTON R AT NEW CROTON DAM NR CROTON-ON-HUDSON NY".into(),
            kind: StationKind::UsgsSite(UsgsSiteType::Stream),
            latitude: 41.225_027_8,
            longitude: -73.858_888_9,
            bin: None,
            reference_id: None,
        };
        let at = parse_rfc3339("2026-11-08T14:15:00Z").expect("time");
        let provenance = Provenance {
            source: SourceId::USGS,
            station: Some(dam),
            distance_m: Some(20.0),
            relation: Relation::SameWaterBody,
            observed_at: at,
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        };
        let mut water = WaterConditions {
            discharge_m3s: Some(Series::new(provenance, vec![Sample::new(at, 12.4 * CFS)])),
            ..WaterConditions::default()
        };
        let tz = zone("America/New_York").expect("zone");
        let errors = fetch_normals(&host, &ctx(), &mut water, Some(&tz));
        assert_eq!(errors, []);
        let percentile = water.discharge_percentile_for_date.expect("percentile");
        assert_eq!(percentile.day_of_year, "11-08");
        assert!(percentile.thresholds.p50.is_some());
        let station = percentile
            .provenance
            .and_then(|p| p.station)
            .expect("station");
        assert_eq!(station.id, "USGS-01375000");
        assert!(
            transport
                .seen()
                .iter()
                .any(|u| u.contains("observationNormals"))
        );
        // Without a zone there is no standard-time day to ask about.
        let mut water = WaterConditions {
            discharge_m3s: water.discharge_m3s.take(),
            ..WaterConditions::default()
        };
        assert_eq!(fetch_normals(&host, &ctx(), &mut water, None), []);
        assert!(water.discharge_percentile_for_date.is_none());
    }

    #[test]
    fn croton_end_to_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let mut config = Config::default();
        config.sources.flow.max_distance_km = Some(17.0);
        let (out, transport) = fetched(&croton(), &config, "croton", &cache);
        let s = &out.snapshot;
        // River water: no tide, current, waves or marine forecast.
        assert_eq!(s.marine, MarineConditions::default());
        assert!(s.weather.is_some());
        assert_eq!(s.sky.days.len(), 7);
        let link = &out.plan.chains[&M::Discharge].links[0];
        assert!(
            matches!(&link.place, LinkPlace::Station { station, .. } if station.id == "USGS-01375000")
        );
        // Expected errors: the buoy list's Haverstraw station has no data file (it reports water
        // quality only), and Katonah defines no flood stages before West Nyack, the next gauge
        // within 17 km, supplies them.
        assert_eq!(
            errors(s),
            [
                (
                    "ndbc".into(),
                    Some(M::WaterTemperature),
                    SourceErrorKind::NotFound
                ),
                (
                    "nwps".into(),
                    Some(M::FloodCategories),
                    SourceErrorKind::NoData
                ),
            ]
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
        let flood = s.water.flood_categories.as_ref().expect("flood categories");
        let minor = flood.minor.as_ref().expect("minor");
        assert!((minor.metres - 6.0 * 0.3048).abs() < 1e-3, "{minor:?}");
        let gauges = &out.plan.chains[&M::FloodCategories].links;
        assert!(
            matches!(&gauges[1].place, LinkPlace::Station { station, .. } if station.id == "WNYN6")
        );
        for url in transport.seen() {
            assert_rounded(&url);
        }
    }

    #[test]
    fn the_forecast_names_the_zone_of_a_spot_without_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let mut spot = croton();
        spot.time_zone = String::new();
        let (out, _) = fetched(&spot, &Config::default(), "croton", &cache);
        assert_eq!(out.snapshot.spot.time_zone, "America/New_York");
        assert_eq!(out.snapshot.sky.days.len(), 7);
    }

    #[test]
    fn rounding_checks_catch_precise_coordinates() {
        assert_rounded("https://api.example/x?latitude=40.7&longitude=-74.02");
        assert!(
            std::panic::catch_unwind(|| assert_rounded("https://a.example/points/40.7033,-74.017"))
                .is_err()
        );
    }
}
