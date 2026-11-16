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
use super::transport::{Call, Fetched, Transport, TransportError};
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

/// The relation of each station to the spot from the flowlines and waterbodies around it.
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
    Ok(hints
        .into_iter()
        .filter(|(_, hint)| hint.relation != Relation::Nearby)
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
    let mut ranking = rankings.ranking(&spot.id, &input).clone();
    let mut plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
    if check_coverage(
        host,
        ctx,
        &ranking,
        &plan,
        &input.candidates,
        &mut input.observed,
    ) {
        ranking = rankings.ranking(&spot.id, &input).clone();
        plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
    }
    host.keep(NS_DISCOVERY, COVERAGE_KEY, &input.observed);
    host.keep(NS_DISCOVERY, &spot_key, &rankings);
    Discovery {
        ranking,
        plan,
        errors,
        hydrography,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::transport::Replay;
    use super::*;
    use Measurement as M;
    use underbite_config::sources::KNOWN_SOURCES;
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
    fn battery_discovery_takes_the_battery_and_the_pinned_current_bin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let mut choices = StationChoices::default();
        choices.pin(StationKey::new(SourceId::COOPS, "NYH1927").with_bin(13));
        save_choices(&cache, "battery", &choices, now()).expect("choices");
        let transport = replay("battery");
        let d = discovered(&battery(), &transport, &cache);
        assert!(d.errors.is_empty(), "{:#?}", d.errors);
        assert!(d.hydrography);
        assert_eq!(first_link(&d, M::Tide).1, "8518750");
        let current = first_link(&d, M::TidalCurrent);
        assert_eq!((current.1.as_str(), current.2), ("NYH1927", Some(13)));
        // Without hydrography geometry the nearer Dimond Reef follows, at its shallowest bin.
        let second = &d.plan.chains[&M::TidalCurrent].links[1];
        assert!(matches!(&second.place, LinkPlace::Station { station, .. }
            if station.id == "NYH1919" && station.bin == Some(3)));
        assert_eq!(first_link(&d, M::MarineAlerts).0, "nws");
        assert_eq!(first_link(&d, M::WavesForecast).0, "open-meteo-marine");
        // CO-OPS water levels stand in for the station's own temperature sensor.
        assert_eq!(first_link(&d, M::WaterTemperature).1, "8518750");

        // A second fetch takes the lists from the cache.
        let sent = transport.seen().len();
        let again = discovered(&battery(), &transport, &cache);
        assert_eq!(again.plan, d.plan);
        let lists = [
            "stations.json",
            "activestations.xml",
            "monitoring-locations",
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
        // No estuary sites were recorded around Croton; the other site types still count.
        assert_eq!(d.errors.len(), 1, "{:#?}", d.errors);
        assert!(d.errors[0].message.contains("site_type_code=ES"));
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
    }
}
