//! Discovery: the stations around a spot ranked per measurement, from the station lists, the
//! hydrography, NWPS joins, the spot's pins and exclusions, stored rankings and coverage
//! checks of NDBC stations.

use std::collections::{BTreeMap, BTreeSet};

use underbite_config::schema::Config;
use underbite_core::discover::cache::{DiscoveryCache, ObservedCoverage, StationChoices};
use underbite_core::discover::rank::{
    ChainSpec, DEFAULT_STATIONS_PER_SOURCE, DiscoveryInput, LinkPlace, Plan, Ranking, StationKey,
    distance_m,
};
use underbite_core::model::{
    Measurement, Requirements, SourceError as ModelSourceError, Spot, StationCandidate,
};
use underbite_core::provenance::{Relation, SourceId, StationRef};
use underbite_core::time::UnixTime;
use underbite_providers::hydro;
use underbite_providers::ndbc::Ndbc;
use underbite_providers::nwps::{self, Nwps};
use underbite_providers::request::{Coordinates, Place, Resource, Target, Window};
use underbite_providers::source::{
    FetchContext, FragmentValue, Fragments, Source, SourceError, check_response,
};

use crate::host::{Driver, FetchHost};
use crate::hydrography::{Placement, relations, river_stations, uses_hydrography};
use crate::lists::station_lists;
use crate::registry::chain_specs;
use crate::source_error;
use crate::spot::{FetchOptions, fetch_context};
use crate::store::{self, spot_key};

/// NWPS gauges whose USGS site is looked up, nearest first.
const JOINED_GAUGES: usize = 3;
/// How long a coverage check counts before a station is checked again, seconds.
const COVERAGE_MAX_AGE_S: i64 = 24 * 3_600;
/// Rounds of coverage checks and ranking one discovery runs at most: a check can promote
/// stations nobody has checked yet into the plan, and those are checked in the next round.
const COVERAGE_ROUNDS: usize = 3;

/// The USGS site each of the nearest NWPS gauges forecasts, from the gauges' metadata.
async fn joins<H: FetchHost>(
    host: &Driver<'_, H>,
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
        match gauge_site(host, ctx, &target).await {
            Ok(Some(site)) => {
                out.insert(
                    StationKey::new(SourceId::NWPS, station.id.clone()),
                    StationKey::new(SourceId::USGS, format!("USGS-{site}")),
                );
            }
            Ok(None) => {}
            Err(e) => tracing::debug!(gauge = %station.id, error = %e, "gauge metadata"),
        }
    }
    out
}

/// The USGS site of the NWPS gauge `target` names, from its metadata.
async fn gauge_site<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    target: &Target,
) -> Result<Option<String>, SourceError> {
    let request = Nwps
        .plan(target, &Requirements::default(), ctx)?
        .into_iter()
        .find(|r| r.resource == Resource::NwpsGauge)
        .ok_or_else(|| SourceError::Unsupported("no gauge request".into()))?;
    let fetched = host.get(&Nwps, &request).await?;
    let gauge = nwps::parse_gauge(check_response(&request, &fetched.as_response())?)?;
    Ok(gauge.usgs_id)
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
async fn check_coverage<H: FetchHost>(
    host: &Driver<'_, H>,
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
        match coverage(host, ctx, &target).await {
            Ok(found) => changed |= observed.merge(&key, &checked, &found, host.now),
            Err(e) => tracing::debug!(station = %key.id, error = %e, "coverage check"),
        }
    }
    changed
}

/// What the standard file of the NDBC station `target` names confirms it reports.
async fn coverage<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    target: &Target,
) -> Result<BTreeMap<Measurement, underbite_core::model::Coverage>, SourceError> {
    let request = Ndbc.coverage_request(target, COVERAGE_FILE)?;
    let fetched = host.get(&Ndbc, &request).await?;
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
            FragmentValue::Coverage { coverage, .. } => Some((f.measurement, *coverage)),
            _ => None,
        })
        .collect())
}

/// Ranks, checks the planned stations nobody has checked yet and ranks again, until no planned
/// station is unchecked (or [`COVERAGE_ROUNDS`] have run), so the next fetch starts from the
/// same plan.
async fn settle<H: FetchHost>(
    host: &Driver<'_, H>,
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
        )
        .await
        {
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
pub struct Discovery {
    /// The stations per measurement, with reasons.
    pub ranking: Ranking,
    /// The links per measurement a session takes.
    pub plan: Plan,
    /// Lists or hydrography that failed.
    pub errors: Vec<ModelSourceError>,
    /// Whether the hydrography related stations to the spot (its credit is due).
    pub hydrography: bool,
    /// The name of the water the hydrography places the spot on, when the map names it.
    pub water_name: Option<String>,
}

/// The stations around `spot` that can serve `requirements` under `config`, ranked per
/// measurement, at `now`: [`discover`] over the configured chains, with the coordinates
/// rounded as the configuration says.
pub async fn stations_near<H: FetchHost>(
    host: &H,
    spot: &Spot,
    config: &Config,
    requirements: &Requirements,
    now: UnixTime,
    options: &FetchOptions,
) -> Discovery {
    let chains = chain_specs(&config.sources, requirements, spot.water_type);
    let ctx = fetch_context(config, now, options);
    discover(host, &ctx, spot, chains, options.allow_metered).await
}

/// Ranks the stations around `spot` for `chains` at the context's time: lists, hydrography,
/// pins and exclusions, stored rankings and coverage checks. Metered calls are spent only when
/// `allow_metered`. The call ledger, with any pause a 429 set, is saved before it returns.
pub async fn discover<H: FetchHost>(
    host: &H,
    ctx: &FetchContext,
    spot: &Spot,
    chains: BTreeMap<Measurement, ChainSpec>,
    allow_metered: bool,
) -> Discovery {
    let driver = Driver::new(host, ctx.now, allow_metered).await;
    let found = discover_with(&driver, ctx, spot, chains).await;
    if let Err(e) = driver.save_usage().await {
        tracing::warn!(error = %e, "call ledger");
    }
    found
}

/// [`discover`] within a fetch, on its driver.
pub(crate) async fn discover_with<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
    chains: BTreeMap<Measurement, ChainSpec>,
) -> Discovery {
    let at = Coordinates {
        latitude: spot.latitude,
        longitude: spot.longitude,
    };
    let lists = station_lists(host, ctx, at, ctx.round(at), &chains).await;
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
    let placement = if uses_hydrography(spot.water_type) && !nearby.is_empty() {
        let river = river_stations(spot, &lists.candidates, &chains);
        match relations(host, ctx, spot, &nearby, &river).await {
            Ok(r) => {
                hydrography = true;
                r
            }
            Err(e) => {
                tracing::warn!(error = %e, "hydrography");
                errors.push(source_error(hydro::SOURCE, "hydrography", &e));
                Placement::default()
            }
        }
    } else {
        Placement::default()
    };
    let joins = if chains.values().any(|c| c.sources.contains(&SourceId::NWPS)) {
        joins(host, ctx, spot, &lists.candidates).await
    } else {
        BTreeMap::new()
    };
    let spot_key = spot_key(&spot.id);
    let mut observed: ObservedCoverage = host
        .stored(store::DISCOVERY, store::COVERAGE_KEY)
        .await
        .unwrap_or_default();
    observed.prune(host.now, COVERAGE_MAX_AGE_S);
    let mut rankings: DiscoveryCache = host
        .stored(store::DISCOVERY, &spot_key)
        .await
        .unwrap_or_default();
    if lists.refreshed {
        rankings.invalidate(&spot.id);
    }
    let choices: StationChoices = host
        .stored(store::CHOICES, &spot_key)
        .await
        .unwrap_or_default();
    let water_name = placement.water_name;
    let mut input = DiscoveryInput {
        latitude: spot.latitude,
        longitude: spot.longitude,
        now: host.now,
        candidates: lists.candidates,
        chains,
        choices,
        observed,
        relations: placement.relations,
        spot_placed: placement.spot_placed,
        joins,
        bin_depths_m: lists.bin_depths_m,
    };
    let (ranking, plan) = settle(host, ctx, &spot.id, &mut rankings, &mut input).await;
    host.keep(store::DISCOVERY, store::COVERAGE_KEY, &input.observed)
        .await;
    host.keep(store::DISCOVERY, &spot_key, &rankings).await;
    Discovery {
        ranking,
        plan,
        errors,
        hydrography,
        water_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::requirements;
    use crate::testing::{MemoryHost, battery, croton, ctx};
    use Measurement as M;
    use underbite_core::discover::rank::Reason;

    fn discovered(spot: &Spot, host: &MemoryHost) -> Discovery {
        let config = Config::default();
        let req = requirements(&config, spot.water_type);
        let chains = chain_specs(&config.sources, &req, spot.water_type);
        pollster::block_on(discover(host, &ctx(), spot, chains, true))
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
    fn battery_discovery_takes_the_battery_and_the_hudson_river_entrance_current_bin() {
        let host = MemoryHost::replay("battery");
        let d = discovered(&battery(), &host);
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
        // takes the lists from the store.
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
        let sent = host.seen().len();
        let again = discovered(&battery(), &host);
        assert_eq!(again.plan, d.plan);
        assert_eq!(first_link(&again, M::TidalCurrent).1, "NYH1927");
        let lists = [
            "stations.json",
            "activestations.xml",
            "monitoring-locations",
            "realtime2/",
        ];
        assert!(
            host.seen()[sent..]
                .iter()
                .all(|u| lists.iter().all(|l| !u.contains(l))),
            "{:#?}",
            &host.seen()[sent..]
        );
        assert_eq!(host.missed(), Vec::<String>::new());
    }

    #[test]
    fn croton_discovery_takes_the_dam_gauge() {
        let host = MemoryHost::replay("croton");
        let d = discovered(&croton(), &host);
        // No estuary sites are listed around Croton: an empty answer, not an error.
        assert_eq!(d.errors, [], "{:#?}", d.errors);
        // The spot is a few metres below the dam gauge, on the same reach: the same water.
        for m in [M::Discharge, M::GageHeight] {
            let link = first_link(&d, m);
            assert_eq!(
                (link.0.as_str(), link.1.as_str(), link.3),
                ("usgs", "USGS-01375000", Relation::SameWaterBody)
            );
        }
        assert!(!d.plan.chains.contains_key(&M::Tide));
        // No forecast gauge is on the Croton: each is on another river (Katonah, West Nyack,
        // placed by the hydrography around them) or not placed at all (West Point, beyond the
        // Hudson's outline), and none is planned.
        let floods = &d.ranking.measurements[&M::FloodCategories];
        assert_eq!(floods.stations, []);
        assert_eq!(d.plan.chains[&M::FloodCategories].links, []);
        let reason = |id: &str| {
            let s = floods.dropped.iter().find(|s| s.station.id == id);
            s.expect("dropped").reason.clone()
        };
        let other = Reason::OtherWaterBody {
            relation: Relation::Nearby,
        };
        assert_eq!(reason("KTNN6"), other);
        assert_eq!(reason("WNYN6"), other);
        assert_eq!(reason("WPTN6"), Reason::NotPlaced);
        assert!(
            floods.dropped.iter().all(|s| matches!(
                s.reason,
                Reason::OtherWaterBody { .. } | Reason::NotPlaced | Reason::TooFar { .. }
            )),
            "{:#?}",
            floods.dropped
        );
    }

    #[test]
    fn a_spot_on_beltzville_lake_takes_its_project_pool_and_inflow() {
        use underbite_core::discover::rank::Limits;
        use underbite_core::model::WaterType;
        use underbite_providers::session::Session;

        use crate::sessions::{run_session, session_chains};

        let spot = Spot {
            id: "beltzville".into(),
            name: "Beltzville Lake".into(),
            latitude: 40.855,
            longitude: -75.627,
            water_type: WaterType::LakeReservoir,
            water_body: None,
            time_zone: "America/New_York".into(),
        };
        // The hydrography around the spot answers nothing mapped.
        let at = Coordinates {
            latitude: spot.latitude,
            longitude: spot.longitude,
        };
        let radius = crate::hydrography::HYDRO_RADIUS_M;
        let empty = r#"{"features":[]}"#;
        let host = MemoryHost::replay("beltzville")
            .answering(
                hydro::flowlines_query(at, radius, &ctx())
                    .expect("query")
                    .url,
                empty,
            )
            .answering(
                hydro::waterbodies_query(at, radius, &ctx())
                    .expect("query")
                    .url,
                empty,
            );
        // Only the reservoir sources: no station list is asked of anyone.
        let lake = ChainSpec {
            sources: vec![SourceId::CWMS, SourceId::RISE],
            limits: Limits::default(),
        };
        let chains = BTreeMap::from([(M::LakeLevel, lake.clone()), (M::Reservoir, lake)]);
        let d = pollster::block_on(discover(&host, &ctx(), &spot, chains, true));
        assert_eq!(d.errors, [], "{:#?}", d.errors);
        for m in [M::LakeLevel, M::Reservoir] {
            let link = first_link(&d, m);
            assert_eq!(
                (link.0.as_str(), link.1.as_str()),
                ("cwms", "NAP/Beltzville")
            );
        }

        // The session takes the project's pool as the lake level and its inflow as the
        // reservoir block.
        let requirements = Requirements {
            measurements: BTreeSet::from([M::LakeLevel, M::Reservoir]),
            ..Requirements::default()
        };
        let driver = pollster::block_on(Driver::new(&host, ctx().now, true));
        let session = Session::new(
            session_chains(&d.plan, &requirements, &ctx()),
            requirements.clone(),
            ctx(),
        );
        let out = pollster::block_on(run_session(&driver, session));
        assert_eq!(out.errors, [], "{:#?}", out.errors);
        let level = out.water.level.as_ref().expect("pool");
        let pool = level.series.samples();
        assert!((191.0..192.0).contains(&pool[0].value), "{:?}", pool[0]);
        let reservoir = out.water.reservoir.as_ref().expect("reservoir");
        assert_eq!(reservoir.project, "Beltzville Lake");
        assert!(reservoir.outflow_m3s.is_none());
        let inflow = reservoir.inflow_m3s.as_ref().expect("inflow");
        assert_eq!(inflow.samples().len(), 2);
        assert!(out.attribution.iter().any(|a| a.source == SourceId::CWMS));
        assert_eq!(host.missed(), Vec::<String>::new());
    }

    #[test]
    fn a_429_during_discovery_is_saved_with_the_ledger() {
        use crate::Fetched;
        use underbite_core::discover::rank::Limits;

        let host = MemoryHost::default().answering_part("usgs.gov", Fetched::new(429, "slow down"));
        let lake = ChainSpec {
            sources: vec![SourceId::USGS],
            limits: Limits::default(),
        };
        let chains = BTreeMap::from([(M::Discharge, lake)]);
        let d = pollster::block_on(discover(&host, &ctx(), &croton(), chains, true));
        assert_ne!(d.errors.len(), 0);
        let paused = pollster::block_on(async {
            let fresh = Driver::new(&host, ctx().now, true).await;
            fresh
                .usage
                .snapshot()
                .paused_until(&SourceId::USGS, ctx().now)
        });
        assert!(paused.is_some(), "{:?}", host.seen());
    }

    #[test]
    fn pins_and_exclusions_come_from_the_store() {
        let host = MemoryHost::replay("battery");
        let mut choices = StationChoices::default();
        choices.exclude(StationKey::new(SourceId::COOPS, "8518750"));
        host.put(store::CHOICES, &spot_key(&battery().id), &choices);
        let d = discovered(&battery(), &host);
        assert_ne!(first_link(&d, M::Tide).1, "8518750");
    }
}
