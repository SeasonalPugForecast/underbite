//! Ranking candidate stations by distance, water body, freshness and coverage.
//!
//! [`rank`] takes every candidate station near a spot (from the sources' station lists), the
//! coverage observed at those stations, the configured chain and limits per measurement, the
//! spot's pinned and excluded stations and optional relation hints from the hydrography. Per
//! measurement it keeps the stations in order, each with the reasons for its place, and lists
//! the stations dropped with the reason. [`Ranking::plan`] turns that into the ordered links per
//! measurement that a fetch session takes.
//!
//! The order is lexicographic, never a score:
//! 1. pinned stations;
//! 2. stations whose observed coverage lacks the measurement after all others;
//! 3. same water body, then upstream or downstream, then any other relation;
//! 4. the station's primary product before a secondary one;
//! 5. nearer first;
//! 6. by source, id and bin, so equal stations keep a stable order.
//!
//! A station is dropped when excluded, beyond the distance limit, older than the age limit,
//! when the hydrography puts it on other water than a river spot's own for a river measurement,
//! or when it places the spot but not the station (a pin overrides these four), when it is a
//! weak-and-variable current station (no predictions), or when it is a current bin other than
//! the one kept for its station: a pinned bin, else the bin nearest the surface. When the
//! hydrography does not place the spot, a river station is kept by distance, as `Nearby`, with
//! that relation among its reasons.

use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::cache::{Observed, ObservedCoverage, StationChoices};
use crate::model::{Coverage, Measurement, StationCandidate};
use crate::provenance::{Relation, SourceId, StationKind, StationRef};
use crate::time::UnixTime;

/// Distance limit when the configuration sets none, metres.
pub const DEFAULT_MAX_DISTANCE_M: f64 = 50_000.0;
/// Age limit of a station's latest value when the configuration sets none, seconds.
pub const DEFAULT_MAX_AGE_S: i64 = 24 * 3_600;
/// Stations per source a plan tries for one measurement.
pub const DEFAULT_STATIONS_PER_SOURCE: usize = 3;

const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Great-circle distance between two points, metres (haversine).
pub fn distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let a = ((p2 - p1) / 2.0).sin().powi(2)
        + p1.cos() * p2.cos() * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * a.sqrt().min(1.0).asin()
}

/// A station, or one current bin of it, in its source's own id space. As a pin, an exclusion
/// or a hint, a key without a bin stands for every bin of the station.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StationKey {
    pub source: SourceId,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<u16>,
}

impl StationKey {
    /// A whole station.
    pub fn new(source: SourceId, id: impl Into<String>) -> Self {
        Self {
            source,
            id: id.into(),
            bin: None,
        }
    }

    /// One bin of the station.
    #[must_use]
    pub fn with_bin(self, bin: u16) -> Self {
        Self {
            bin: Some(bin),
            ..self
        }
    }

    /// The key of a listed station, with its bin.
    pub fn of(station: &StationRef) -> Self {
        Self {
            source: station.source.clone(),
            id: station.id.clone(),
            bin: station.bin,
        }
    }

    /// The station without its bin.
    #[must_use]
    pub fn station(&self) -> Self {
        Self {
            bin: None,
            ..self.clone()
        }
    }

    /// Whether this key, used as a pin or an exclusion, covers `other`.
    pub fn covers(&self, other: &StationKey) -> bool {
        self.source == other.source
            && self.id == other.id
            && (self.bin.is_none() || self.bin == other.bin)
    }
}

/// How far and how old a station's values may be for one measurement.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub max_distance_m: f64,
    pub max_age_s: i64,
}

impl Limits {
    /// The limits of a source group's `max_distance_km` and `max_age_minutes`, with
    /// [`DEFAULT_MAX_DISTANCE_M`] and [`DEFAULT_MAX_AGE_S`] for those not set.
    pub fn from_config(max_distance_km: Option<f64>, max_age_minutes: Option<u32>) -> Self {
        Self {
            max_distance_m: max_distance_km.map_or(DEFAULT_MAX_DISTANCE_M, |km| km * 1_000.0),
            max_age_s: max_age_minutes.map_or(DEFAULT_MAX_AGE_S, |m| i64::from(m) * 60),
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::from_config(None, None)
    }
}

/// The configured chain of one measurement: sources in order of preference, and limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainSpec {
    pub sources: Vec<SourceId>,
    pub limits: Limits,
}

/// Everything a ranking depends on.
#[derive(Clone, Debug, Default)]
pub struct DiscoveryInput {
    /// The spot.
    pub latitude: f64,
    pub longitude: f64,
    pub now: UnixTime,
    /// Candidates from every station list. A station listed twice (a CO-OPS station in the
    /// tide-prediction and the water-level lists) is merged.
    pub candidates: Vec<StationCandidate>,
    /// The measurements to rank, each with its chain and limits.
    pub chains: BTreeMap<Measurement, ChainSpec>,
    /// The spot's pinned and excluded stations.
    pub choices: StationChoices,
    /// Coverage confirmed or denied by earlier fetches.
    pub observed: ObservedCoverage,
    /// Relation of stations to the spot from the hydrography, keyed without bin. A station
    /// listed here has a known relation, `Nearby` included (a tributary, another river); a
    /// station missing from it is one the hydrography does not reach.
    pub relations: BTreeMap<StationKey, Relation>,
    /// Whether the hydrography places the spot on the river network. River measurements then
    /// take only stations it places on the spot's own water.
    pub spot_placed: bool,
    /// Forecast gauges and the site each one forecasts (an NWPS gauge and its USGS site, by
    /// `usgsId`). The gauge takes the site's relation, pin and exclusion.
    pub joins: BTreeMap<StationKey, StationKey>,
    /// Depth of current bins, metres, keyed with the bin (CO-OPS lists give feet).
    pub bin_depths_m: BTreeMap<StationKey, f32>,
}

/// What is known of a station's coverage of one measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    /// A list, a series catalogue or a coverage check gives the time of a value.
    Reported,
    /// Listed or flagged as offered, not confirmed.
    Listed,
    /// A coverage check found no value: ranked after every other station.
    NotReported,
}

/// Why a station has its place, or why it was dropped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Reason {
    Pinned,
    Excluded,
    TooFar {
        distance_m: f32,
        limit_m: f32,
    },
    Stale {
        age_s: i64,
        limit_s: i64,
    },
    /// A weak-and-variable current station: no predictions.
    WeakAndVariable,
    /// Another bin of the station is used.
    OtherBin {
        kept: u16,
    },
    Reported {
        at: UnixTime,
    },
    Listed,
    NotReported {
        checked_at: UnixTime,
    },
    Relation {
        relation: Relation,
    },
    /// For a river measurement, the hydrography puts the station on other water than the
    /// spot's: not the same water body, nor up or downstream on the same river.
    OtherWaterBody {
        relation: Relation,
    },
    /// For a river measurement, the hydrography places the spot on the network but not the
    /// station: whether it is on the spot's own river is unknown.
    NotPlaced,
    ForecastGaugeFor {
        site: StationKey,
    },
    /// Not the station's primary product for the measurement.
    Secondary,
    Distance {
        distance_m: f32,
    },
}

/// A station kept for a measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RankedStation {
    pub station: StationRef,
    pub distance_m: f32,
    pub relation: Relation,
    pub coverage: CoverageState,
    /// Time of the latest value, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_value_at: Option<UnixTime>,
    /// Age of the latest value at ranking time, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_s: Option<i64>,
    pub primary: bool,
    pub pinned: bool,
    /// The factors of its place, in the order they count.
    pub reasons: Vec<Reason>,
}

/// A station left out for a measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DroppedStation {
    pub station: StationRef,
    pub distance_m: f32,
    pub reason: Reason,
}

/// The ranking of one measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasurementRanking {
    pub chain: ChainSpec,
    pub stations: Vec<RankedStation>,
    pub dropped: Vec<DroppedStation>,
}

/// A map keyed by station as a list of `[key, relation]` pairs.
mod relation_pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Relation, StationKey};

    pub(super) fn serialize<S: Serializer>(
        map: &BTreeMap<StationKey, Relation>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<StationKey, Relation>, D::Error> {
        Ok(Vec::<(StationKey, Relation)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}

/// Version of the ranking rules; a ranking made under other rules is redone. Raise it whenever
/// [`rank`] would choose differently from the same input.
pub const RANK_RULES: u32 = 1;

/// The stations of a spot, ranked per measurement, and what the ranking was computed from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ranking {
    /// [`RANK_RULES`] when the ranking was made; 0 for rankings stored before it was recorded.
    #[serde(default)]
    pub rules: u32,
    pub latitude: f64,
    pub longitude: f64,
    pub at: UnixTime,
    pub choices: StationChoices,
    /// [`ObservedCoverage::revision`] of the coverage used.
    pub coverage_revision: u64,
    /// The hydrography's relations and whether it placed the spot, as ranked. Stored as a list
    /// of pairs: a station key is not a JSON object key.
    #[serde(default, with = "relation_pairs")]
    pub relations: BTreeMap<StationKey, Relation>,
    #[serde(default)]
    pub spot_placed: bool,
    pub measurements: BTreeMap<Measurement, MeasurementRanking>,
}

/// Where a planned link takes its values from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "place", rename_all = "snake_case")]
pub enum LinkPlace {
    Station {
        station: StationRef,
        distance_m: f32,
        relation: Relation,
    },
    /// The spot itself, for gridded and satellite sources; the host rounds it before sending.
    Point {
        latitude: f64,
        longitude: f64,
        relation: Relation,
    },
}

/// One link of a planned chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanLink {
    pub source: SourceId,
    pub place: LinkPlace,
}

/// Why a source of the configured chain has no link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SkipReason {
    /// No station of the source serves the measurement within the limits.
    NoStation,
    /// Modelled sea level is never used near a CO-OPS tide station.
    CoopsTideStationNearby {
        station: StationKey,
        distance_m: f32,
    },
}

/// A source of the configured chain left out of the plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkippedSource {
    pub source: SourceId,
    pub reason: SkipReason,
}

/// The links of one measurement, in the order a session tries them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlannedChain {
    pub links: Vec<PlanLink>,
    /// The oldest latest value accepted, seconds before now.
    pub max_age_s: i64,
    pub skipped: Vec<SkippedSource>,
}

/// Per measurement, the links a fetch session takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub chains: BTreeMap<Measurement, PlannedChain>,
}

/// A station after merging its list entries.
struct Merged {
    station: StationRef,
    offers: BTreeMap<Measurement, Coverage>,
    first: usize,
}

fn merge(candidates: &[StationCandidate]) -> BTreeMap<StationKey, Merged> {
    let mut out: BTreeMap<StationKey, Merged> = BTreeMap::new();
    for (first, c) in candidates.iter().enumerate() {
        match out.entry(StationKey::of(&c.station)) {
            Entry::Vacant(e) => {
                e.insert(Merged {
                    station: c.station.clone(),
                    offers: c.offers.clone(),
                    first,
                });
            }
            Entry::Occupied(mut e) => {
                let m = e.get_mut();
                // A prediction station also plans its water-level observations, so its
                // prediction-list entry stands for both.
                if m.station.kind == StationKind::WaterLevel
                    && c.station.kind != StationKind::WaterLevel
                {
                    m.station = c.station.clone();
                }
                for (measurement, coverage) in &c.offers {
                    m.offers
                        .entry(*measurement)
                        .and_modify(|have| {
                            have.last_value_at = have.last_value_at.max(coverage.last_value_at);
                            have.primary |= coverage.primary;
                        })
                        .or_insert(*coverage);
                }
            }
        }
    }
    out
}

/// The bin kept per station among those not excluded: a pinned bin, else the shallowest of known depth, else the first
/// listed (CO-OPS lists a station's bins from the surface down).
fn kept_bins(
    merged: &BTreeMap<StationKey, Merged>,
    input: &DiscoveryInput,
) -> BTreeMap<StationKey, u16> {
    let mut best: BTreeMap<StationKey, (bool, f32, usize, u16)> = BTreeMap::new();
    for (key, m) in merged {
        let Some(bin) = key.bin else { continue };
        // An excluded bin is dropped whatever happens, so it cannot be the one kept.
        let site = input.joins.get(&key.station());
        if input.choices.is_excluded(key) || site.is_some_and(|s| input.choices.is_excluded(s)) {
            continue;
        }
        let depth = input
            .bin_depths_m
            .get(key)
            .copied()
            .unwrap_or(f32::INFINITY);
        let this = (!input.choices.pinned.contains(key), depth, m.first, bin);
        let better = |a: &(bool, f32, usize, u16), b: &(bool, f32, usize, u16)| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .is_lt()
        };
        best.entry(key.station())
            .and_modify(|b| {
                if better(&this, b) {
                    *b = this;
                }
            })
            .or_insert(this);
    }
    best.into_iter().map(|(k, (.., bin))| (k, bin)).collect()
}

/// Coverage of one measurement from the lists and the checks.
struct Seen {
    state: CoverageState,
    last_value_at: Option<UnixTime>,
    primary: bool,
    reason: Reason,
}

fn seen(listed: Option<&Coverage>, observed: Option<&Observed>) -> Option<Seen> {
    let (last_value_at, primary) = match (listed, observed) {
        (
            _,
            Some(Observed {
                coverage: Some(c), ..
            }),
        ) => (
            c.last_value_at.max(listed.and_then(|l| l.last_value_at)),
            c.primary,
        ),
        (Some(l), Some(o)) => {
            return Some(Seen {
                state: CoverageState::NotReported,
                last_value_at: None,
                primary: l.primary,
                reason: Reason::NotReported {
                    checked_at: o.checked_at,
                },
            });
        }
        (Some(l), None) => (l.last_value_at, l.primary),
        (None, _) => return None,
    };
    let (state, reason) = match last_value_at {
        Some(at) => (CoverageState::Reported, Reason::Reported { at }),
        None => (CoverageState::Listed, Reason::Listed),
    };
    Some(Seen {
        state,
        last_value_at,
        primary,
        reason,
    })
}

/// Ranks the candidates of `input` for every measurement of its chains.
pub fn rank(input: &DiscoveryInput) -> Ranking {
    let merged = merge(&input.candidates);
    let kept = kept_bins(&merged, input);
    let measurements = input
        .chains
        .iter()
        .map(|(m, chain)| (*m, rank_one(input, &merged, &kept, *m, chain)))
        .collect();
    Ranking {
        rules: RANK_RULES,
        latitude: input.latitude,
        longitude: input.longitude,
        at: input.now,
        choices: input.choices.clone(),
        coverage_revision: input.observed.revision(),
        relations: input.relations.clone(),
        spot_placed: input.spot_placed,
        measurements,
    }
}

fn rank_one(
    input: &DiscoveryInput,
    merged: &BTreeMap<StationKey, Merged>,
    kept: &BTreeMap<StationKey, u16>,
    measurement: Measurement,
    chain: &ChainSpec,
) -> MeasurementRanking {
    let mut stations = Vec::new();
    let mut dropped = Vec::new();
    for (key, m) in merged {
        let observed = input.observed.get(key, measurement);
        let Some(seen) = seen(m.offers.get(&measurement), observed) else {
            continue;
        };
        let distance = distance_m(
            input.latitude,
            input.longitude,
            m.station.latitude,
            m.station.longitude,
        );
        match judge(
            input,
            kept,
            measurement,
            chain.limits,
            key,
            m,
            seen,
            distance,
        ) {
            Ok(ranked) => stations.push(ranked),
            Err(reason) => dropped.push(DroppedStation {
                station: m.station.clone(),
                distance_m: distance as f32,
                reason,
            }),
        }
    }
    stations.sort_by(order);
    MeasurementRanking {
        chain: chain.clone(),
        stations,
        dropped,
    }
}

/// Measurements of a river at a gauge, which only mean something on the spot's own river.
fn is_river(measurement: Measurement) -> bool {
    matches!(
        measurement,
        Measurement::Discharge
            | Measurement::GageHeight
            | Measurement::FlowForecast
            | Measurement::FloodCategories
    )
}

/// The station's relation from the hydrography, its own or its site's, when known. A related
/// relation of either wins over `Nearby`.
fn known_relation(
    input: &DiscoveryInput,
    key: &StationKey,
    site: Option<&StationKey>,
) -> Option<Relation> {
    let own = input.relations.get(&key.station()).copied();
    let of_site = site.and_then(|s| input.relations.get(&s.station()).copied());
    let related = |r: &Relation| *r != Relation::Nearby;
    own.filter(related)
        .or(of_site.filter(related))
        .or(own)
        .or(of_site)
}

#[allow(clippy::too_many_arguments)]
fn judge(
    input: &DiscoveryInput,
    kept: &BTreeMap<StationKey, u16>,
    measurement: Measurement,
    limits: Limits,
    key: &StationKey,
    m: &Merged,
    seen: Seen,
    distance: f64,
) -> Result<RankedStation, Reason> {
    let choices = &input.choices;
    let site = input.joins.get(&key.station());
    if choices.is_excluded(key) || site.is_some_and(|s| choices.is_excluded(s)) {
        return Err(Reason::Excluded);
    }
    if let (Some(bin), Some(&k)) = (key.bin, kept.get(&key.station()))
        && bin != k
    {
        return Err(Reason::OtherBin { kept: k });
    }
    if m.station.kind == StationKind::CurrentWeakVariable {
        return Err(Reason::WeakAndVariable);
    }
    let pinned = choices.is_pinned(key) || site.is_some_and(|s| choices.is_pinned(s));
    let mut reasons = Vec::new();
    if pinned {
        reasons.push(Reason::Pinned);
    }
    let distance_m = distance as f32;
    let age_s = seen.last_value_at.map(|t| input.now.0 - t.0);
    let mut over = Vec::new();
    if distance > limits.max_distance_m {
        over.push(Reason::TooFar {
            distance_m,
            limit_m: limits.max_distance_m as f32,
        });
    }
    if let Some(age_s) = age_s
        && age_s > limits.max_age_s
    {
        over.push(Reason::Stale {
            age_s,
            limit_s: limits.max_age_s,
        });
    }
    let known = known_relation(input, key, site);
    let river = is_river(measurement);
    if river {
        match known {
            Some(relation) if relation_class(relation) == OTHER_WATER => {
                over.push(Reason::OtherWaterBody { relation });
            }
            None if input.spot_placed => over.push(Reason::NotPlaced),
            _ => {}
        }
    }
    if !pinned && let Some(first) = over.first() {
        return Err(first.clone());
    }
    reasons.extend(over);
    reasons.push(seen.reason);
    let relation = known.unwrap_or(Relation::Nearby);
    // A river station the hydrography does not place is only near the spot, and says so.
    if relation != Relation::Nearby || (river && known.is_none()) {
        reasons.push(Reason::Relation { relation });
    }
    if let Some(site) = site {
        reasons.push(Reason::ForecastGaugeFor { site: site.clone() });
    }
    if !seen.primary {
        reasons.push(Reason::Secondary);
    }
    reasons.push(Reason::Distance { distance_m });
    Ok(RankedStation {
        station: m.station.clone(),
        distance_m,
        relation,
        coverage: seen.state,
        last_value_at: seen.last_value_at,
        age_s,
        primary: seen.primary,
        pinned,
        reasons,
    })
}

/// The class of relations other than the same water body or the same river.
const OTHER_WATER: u8 = 2;

fn relation_class(relation: Relation) -> u8 {
    match relation {
        Relation::SameWaterBody => 0,
        Relation::Upstream | Relation::Downstream => 1,
        _ => OTHER_WATER,
    }
}

fn order(a: &RankedStation, b: &RankedStation) -> Ordering {
    let lacks = |s: &RankedStation| s.coverage == CoverageState::NotReported;
    b.pinned
        .cmp(&a.pinned)
        .then(lacks(a).cmp(&lacks(b)))
        .then(relation_class(a.relation).cmp(&relation_class(b.relation)))
        .then(b.primary.cmp(&a.primary))
        .then(a.distance_m.total_cmp(&b.distance_m))
        .then_with(|| StationKey::of(&a.station).cmp(&StationKey::of(&b.station)))
}

/// The station list a chain source draws its stations from; `None` for gridded and satellite
/// sources, which get a point link.
fn station_pool(source: &SourceId) -> Option<SourceId> {
    if *source == SourceId::COOPS_HARMONICS {
        return Some(SourceId::COOPS);
    }
    [
        SourceId::USGS,
        SourceId::COOPS,
        SourceId::NDBC,
        SourceId::NWPS,
        SourceId::DFO_IWLS,
        SourceId::ECCC_HYDROMETRIC,
        SourceId::CWMS,
        SourceId::RISE,
    ]
    .into_iter()
    .find(|s| s == source)
}

/// Whether `source` can compute values for a station of `kind`.
fn serves(source: &SourceId, kind: StationKind) -> bool {
    *source != SourceId::COOPS_HARMONICS
        || matches!(
            kind,
            StationKind::TideHarmonic
                | StationKind::TideSubordinate
                | StationKind::CurrentHarmonic
                | StationKind::CurrentSubordinate
        )
}

fn point_relation(source: &SourceId) -> Relation {
    if source.as_str() == "mur-sst" || *source == SourceId::CYAN {
        Relation::Satellite
    } else {
        Relation::Gridded
    }
}

impl Ranking {
    /// The links per measurement: for each source of the chain in order, its best
    /// `stations_per_source` stations, or a point link for a gridded or satellite source.
    pub fn plan(&self, stations_per_source: usize) -> Plan {
        let chains = self
            .measurements
            .iter()
            .map(|(m, r)| (*m, self.plan_one(*m, r, stations_per_source)))
            .collect();
        Plan { chains }
    }

    fn plan_one(
        &self,
        measurement: Measurement,
        ranking: &MeasurementRanking,
        per_source: usize,
    ) -> PlannedChain {
        let mut links = Vec::new();
        let mut skipped = Vec::new();
        let mut done = BTreeSet::new();
        for source in &ranking.chain.sources {
            if !done.insert(source) {
                continue;
            }
            let skip = |reason| SkippedSource {
                source: source.clone(),
                reason,
            };
            if let Some(pool) = station_pool(source) {
                let before = links.len();
                links.extend(
                    ranking
                        .stations
                        .iter()
                        .filter(|s| s.station.source == pool && serves(source, s.station.kind))
                        .take(per_source)
                        .map(|s| PlanLink {
                            source: source.clone(),
                            place: LinkPlace::Station {
                                station: s.station.clone(),
                                distance_m: s.distance_m,
                                relation: s.relation,
                            },
                        }),
                );
                if links.len() == before {
                    skipped.push(skip(SkipReason::NoStation));
                }
            } else if measurement == Measurement::Tide
                && *source == SourceId::OPEN_METEO_MARINE
                && let Some(s) = ranking.nearest_coops(ranking.chain.limits)
            {
                skipped.push(skip(SkipReason::CoopsTideStationNearby {
                    station: StationKey::of(&s.station),
                    distance_m: s.distance_m,
                }));
            } else {
                links.push(PlanLink {
                    source: source.clone(),
                    place: LinkPlace::Point {
                        latitude: self.latitude,
                        longitude: self.longitude,
                        relation: point_relation(source),
                    },
                });
            }
        }
        PlannedChain {
            links,
            max_age_s: ranking.chain.limits.max_age_s,
            skipped,
        }
    }

    /// NDBC stations kept on their list flags alone: the host confirms them with a coverage
    /// check and merges the result into the observed coverage.
    pub fn unconfirmed(&self) -> BTreeSet<StationKey> {
        self.measurements
            .values()
            .flat_map(|r| &r.stations)
            .filter(|s| s.station.source == SourceId::NDBC && s.coverage == CoverageState::Listed)
            .map(|s| StationKey::of(&s.station))
            .collect()
    }
}

impl MeasurementRanking {
    /// The nearest kept CO-OPS station within the distance limit.
    fn nearest_coops(&self, limits: Limits) -> Option<&RankedStation> {
        self.stations
            .iter()
            .filter(|s| {
                s.station.source == SourceId::COOPS
                    && f64::from(s.distance_m) <= limits.max_distance_m
            })
            .min_by(|a, b| a.distance_m.total_cmp(&b.distance_m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Measurement as M;
    use crate::provenance::UsgsSiteType;
    use crate::time::parse_rfc3339;

    const FOOT_M: f64 = 0.3048;

    fn now() -> UnixTime {
        parse_rfc3339("2026-11-15T06:00:00Z").unwrap()
    }

    fn ago(minutes: i64) -> UnixTime {
        UnixTime(now().0 - minutes * 60)
    }

    fn chain(sources: &[&str], km: Option<f64>, minutes: Option<u32>) -> ChainSpec {
        ChainSpec {
            sources: sources.iter().map(|s| SourceId::new(*s)).collect(),
            limits: Limits::from_config(km, minutes),
        }
    }

    fn candidate(station: StationRef, offers: &[(M, Option<UnixTime>)]) -> StationCandidate {
        let offers = offers
            .iter()
            .map(|&(m, last_value_at)| {
                let primary = true;
                (
                    m,
                    Coverage {
                        last_value_at,
                        primary,
                    },
                )
            })
            .collect();
        StationCandidate { station, offers }
    }

    fn station(source: SourceId, id: &str, kind: StationKind, lat: f64, lon: f64) -> StationRef {
        StationRef {
            source,
            id: id.into(),
            name: id.into(),
            kind,
            latitude: lat,
            longitude: lon,
            bin: None,
            reference_id: None,
        }
    }

    /// The recorded CO-OPS station lists, read the way the metadata parser reads them, with the
    /// current bin depths (listed in feet).
    fn coops_lists() -> (Vec<StationCandidate>, BTreeMap<StationKey, f32>) {
        let mut out = Vec::new();
        let mut depths = BTreeMap::new();
        for list in ["tidepredictions", "currentpredictions", "waterlevels"] {
            let path = format!(
                "{}/../../tests/fixtures/raw/coops/coops-mdapi-stations-{list}-newyork-trimmed.json",
                env!("CARGO_MANIFEST_DIR")
            );
            let body: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            for s in body["stations"].as_array().unwrap() {
                let (kind, m) = match (list, s["type"].as_str().unwrap_or("")) {
                    ("tidepredictions", "R") => (StationKind::TideHarmonic, M::Tide),
                    ("tidepredictions", "S") => (StationKind::TideSubordinate, M::Tide),
                    ("currentpredictions", "H") => (StationKind::CurrentHarmonic, M::TidalCurrent),
                    ("currentpredictions", "S") => {
                        (StationKind::CurrentSubordinate, M::TidalCurrent)
                    }
                    ("currentpredictions", "W") => {
                        (StationKind::CurrentWeakVariable, M::TidalCurrent)
                    }
                    ("waterlevels", _) if s["tidal"] == false => {
                        (StationKind::WaterLevel, M::LakeLevel)
                    }
                    ("waterlevels", _) => (StationKind::WaterLevel, M::Tide),
                    _ => continue,
                };
                let mut st = station(
                    SourceId::COOPS,
                    s["id"].as_str().unwrap(),
                    kind,
                    s["lat"].as_f64().unwrap(),
                    s["lng"].as_f64().unwrap(),
                );
                st.bin = s["currbin"].as_u64().map(|b| u16::try_from(b).unwrap());
                if let (Some(_), Some(feet)) = (st.bin, s["depth"].as_f64()) {
                    depths.insert(StationKey::of(&st), (feet * FOOT_M) as f32);
                }
                out.push(candidate(st, &[(m, None)]));
            }
        }
        (out, depths)
    }

    /// NDBC stations near New York Harbor as `activestations.xml` lists them (`met="y"`).
    fn ndbc_list() -> Vec<StationCandidate> {
        let met = [
            (M::ObservedWind, None),
            (M::ObservedPressure, None),
            (M::WaterTemperature, None),
        ];
        let buoy = [met[0], met[1], met[2], (M::Waves, None)];
        let ndbc = |id, kind, lat, lon| station(SourceId::NDBC, id, kind, lat, lon);
        vec![
            candidate(ndbc("batn6", StationKind::Cman, 40.701, -74.014), &met),
            candidate(ndbc("44065", StationKind::Buoy, 40.368, -73.701), &buoy),
            candidate(ndbc("44025", StationKind::Buoy, 40.258, -73.175), &buoy),
        ]
    }

    /// Coverage checks of the standard files: 44025's wind and pressure columns are empty.
    fn ndbc_checks() -> ObservedCoverage {
        let checked = BTreeSet::from([
            M::ObservedWind,
            M::ObservedPressure,
            M::WaterTemperature,
            M::Waves,
        ]);
        let fresh = Coverage {
            last_value_at: Some(ago(40)),
            primary: true,
        };
        let reports = |ms: &[M]| ms.iter().map(|m| (*m, fresh)).collect::<BTreeMap<_, _>>();
        let all: Vec<M> = checked.iter().copied().collect();
        let mut observed = ObservedCoverage::default();
        let key = |id: &str| StationKey::new(SourceId::NDBC, id);
        let quiet = reports(&[M::WaterTemperature, M::Waves]);
        observed.merge(&key("44025"), &checked, &quiet, now());
        observed.merge(&key("44065"), &checked, &reports(&all), now());
        observed
    }

    fn battery() -> DiscoveryInput {
        let (mut candidates, bin_depths_m) = coops_lists();
        candidates.extend(ndbc_list());
        let tides = ["coops", "coops-harmonics", "open-meteo-marine"];
        let wind = chain(&["coops", "ndbc"], Some(100.0), Some(180));
        DiscoveryInput {
            latitude: 40.7033,
            longitude: -74.0170,
            now: now(),
            candidates,
            chains: BTreeMap::from([
                (M::Tide, chain(&tides, Some(25.0), None)),
                (M::TidalCurrent, chain(&tides, None, None)),
                (M::ObservedWind, wind.clone()),
                (M::ObservedPressure, wind),
            ]),
            observed: ndbc_checks(),
            bin_depths_m,
            ..DiscoveryInput::default()
        }
    }

    fn first(plan: &Plan, m: M) -> &PlanLink {
        &plan.chains[&m].links[0]
    }

    fn link_station(link: &PlanLink) -> (&str, &str, Option<u16>) {
        match &link.place {
            LinkPlace::Station { station, .. } => {
                (link.source.as_str(), station.id.as_str(), station.bin)
            }
            LinkPlace::Point { .. } => (link.source.as_str(), "point", None),
        }
    }

    fn ids(r: &MeasurementRanking) -> Vec<&str> {
        r.stations.iter().map(|s| s.station.id.as_str()).collect()
    }

    fn dropped<'a>(r: &'a MeasurementRanking, id: &str) -> Vec<&'a Reason> {
        r.dropped
            .iter()
            .filter(|d| d.station.id == id)
            .map(|d| &d.reason)
            .collect()
    }

    #[test]
    fn battery_tide_starts_at_the_battery_without_modelled_sea_level() {
        let ranking = rank(&battery());
        let plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
        let tide = &plan.chains[&M::Tide];
        assert_eq!(link_station(&tide.links[0]), ("coops", "8518750", None));
        // Listed for predictions and for water levels: one station, planned as harmonic.
        let battery = &ranking.measurements[&M::Tide].stations[0];
        assert_eq!(battery.station.kind, StationKind::TideHarmonic);
        assert_eq!(battery.coverage, CoverageState::Listed);
        assert!((380.0..400.0).contains(&battery.distance_m));
        assert_eq!(
            link_station(&tide.links[3]),
            ("coops-harmonics", "8518750", None)
        );
        assert!(
            tide.links
                .iter()
                .all(|l| matches!(l.place, LinkPlace::Station { .. }))
        );
        assert_eq!(
            tide.skipped,
            vec![SkippedSource {
                source: SourceId::OPEN_METEO_MARINE,
                reason: SkipReason::CoopsTideStationNearby {
                    station: StationKey::new(SourceId::COOPS, "8518750"),
                    distance_m: battery.distance_m,
                },
            }]
        );
    }

    #[test]
    fn modelled_sea_level_joins_the_tide_chain_only_without_coops_stations() {
        let mut input = battery();
        for c in &input.candidates {
            if c.station.source == SourceId::COOPS {
                input.choices.exclude(StationKey::of(&c.station));
            }
        }
        let ranking = rank(&input);
        let tide = &ranking.plan(DEFAULT_STATIONS_PER_SOURCE).chains[&M::Tide];
        let point = LinkPlace::Point {
            latitude: 40.7033,
            longitude: -74.0170,
            relation: Relation::Gridded,
        };
        assert_eq!(
            tide.links,
            vec![PlanLink {
                source: SourceId::OPEN_METEO_MARINE,
                place: point,
            }]
        );
        assert_eq!(tide.skipped.len(), 2);
        let tides = &ranking.measurements[&M::Tide];
        assert_eq!(dropped(tides, "8518750"), [&Reason::Excluded]);

        // Stations beyond the tide limit do not keep the fallback out.
        let mut far = battery();
        far.chains.get_mut(&M::Tide).unwrap().limits = Limits::from_config(Some(0.2), None);
        let tide = &rank(&far).plan(1).chains[&M::Tide];
        assert_eq!(
            link_station(&tide.links[0]),
            ("open-meteo-marine", "point", None)
        );
    }

    #[test]
    fn battery_current_keeps_the_bin_nearest_the_surface() {
        let ranking = rank(&battery());
        let current = &ranking.measurements[&M::TidalCurrent];
        // Dimond Reef (0.70 km) is nearer than the Hudson River Entrance (0.85 km).
        assert_eq!(&ids(current)[..2], ["NYH1919", "NYH1927"]);
        assert_eq!(current.stations[0].station.bin, Some(3));
        assert_eq!(current.stations[1].station.bin, Some(13));
        let other = Reason::OtherBin { kept: 13 };
        assert_eq!(dropped(current, "NYH1927"), [&other, &other]);
        assert!(
            current
                .dropped
                .iter()
                .any(|d| d.reason == Reason::WeakAndVariable)
        );

        // The hydrography puts the entrance on the spot's water body: it leads the plan.
        let mut input = battery();
        let entrance = StationKey::new(SourceId::COOPS, "NYH1927");
        input.relations.insert(entrance, Relation::SameWaterBody);
        let ranking = rank(&input);
        let plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
        let link = first(&plan, M::TidalCurrent);
        assert_eq!(link_station(link), ("coops", "NYH1927", Some(13)));
        let LinkPlace::Station { relation, .. } = &link.place else {
            panic!("a station link");
        };
        assert_eq!(*relation, Relation::SameWaterBody);
        let same = Reason::Relation {
            relation: Relation::SameWaterBody,
        };
        assert!(
            ranking.measurements[&M::TidalCurrent].stations[0]
                .reasons
                .contains(&same)
        );
    }

    #[test]
    fn an_excluded_bin_leaves_another_bin_serving() {
        let mut input = battery();
        input
            .choices
            .exclude(StationKey::new(SourceId::COOPS, "NYH1927").with_bin(13));
        let ranking = rank(&input);
        let current = &ranking.measurements[&M::TidalCurrent];
        let serving: Vec<Option<u16>> = current
            .stations
            .iter()
            .filter(|s| s.station.id == "NYH1927")
            .map(|s| s.station.bin)
            .collect();
        assert_eq!(serving.len(), 1, "one bin of the station serves");
        assert_ne!(serving[0], Some(13));
        let reasons = dropped(current, "NYH1927");
        assert!(reasons.contains(&&Reason::Excluded), "{reasons:?}");
        assert!(
            reasons
                .iter()
                .all(|r| matches!(r, Reason::Excluded | Reason::OtherBin { .. })),
            "{reasons:?}"
        );
    }

    #[test]
    fn pins_and_exclusions_change_the_order_with_reasons() {
        let mut input = battery();
        input
            .choices
            .pin(StationKey::new(SourceId::COOPS, "NYH1927").with_bin(8));
        input
            .choices
            .exclude(StationKey::new(SourceId::COOPS, "8518750"));
        let ranking = rank(&input);

        let current = &ranking.measurements[&M::TidalCurrent];
        let pinned = &current.stations[0];
        assert_eq!(
            (pinned.station.id.as_str(), pinned.station.bin),
            ("NYH1927", Some(8))
        );
        assert_eq!(pinned.reasons[0], Reason::Pinned);
        let other = Reason::OtherBin { kept: 8 };
        assert_eq!(dropped(current, "NYH1927"), [&other, &other]);

        let tide = &ranking.measurements[&M::Tide];
        assert_eq!(ids(tide)[0], "8517847"); // Brooklyn Bridge, 1.9 km
        assert_eq!(dropped(tide, "8518750"), [&Reason::Excluded]);

        // A pin beyond the limit is kept, with the limit among its reasons.
        input.choices.pin(StationKey::new(SourceId::NDBC, "44025"));
        input.chains.get_mut(&M::ObservedWind).unwrap().limits =
            Limits::from_config(Some(50.0), Some(180));
        let wind = &rank(&input).measurements[&M::ObservedWind];
        assert_eq!(ids(wind)[0], "44025");
        assert!(matches!(wind.stations[0].reasons[1], Reason::TooFar { .. }));
    }

    #[test]
    fn a_buoy_without_wind_ranks_after_one_that_reports_it() {
        let ranking = rank(&battery());
        for m in [M::ObservedWind, M::ObservedPressure] {
            let r = &ranking.measurements[&m];
            assert_eq!(ids(r), ["batn6", "44065", "44025"]);
            assert_eq!(r.stations[1].coverage, CoverageState::Reported);
            let quiet = Reason::NotReported { checked_at: now() };
            assert_eq!(r.stations[2].reasons[0], quiet);
        }
        assert_eq!(
            ranking.unconfirmed(),
            BTreeSet::from([StationKey::new(SourceId::NDBC, "batn6")])
        );
        // Off Long Island 44025 is the nearest buoy and still comes last.
        let mut offshore = battery();
        (offshore.latitude, offshore.longitude) = (40.30, -73.30);
        let wind = &rank(&offshore).measurements[&M::ObservedWind];
        assert_eq!(ids(wind), ["44065", "batn6", "44025"]);
        assert!(wind.stations[2].distance_m < wind.stations[0].distance_m);
    }

    #[test]
    fn croton_flow_ranks_the_dam_gauge_and_drops_stale_and_far_sites() {
        let usgs = |id, lat, lon| {
            let kind = StationKind::UsgsSite(UsgsSiteType::Stream);
            station(SourceId::USGS, id, kind, lat, lon)
        };
        // Series ends as in the Croton time-series metadata.
        let dam = usgs("USGS-01375000", 41.225_027_8, -73.858_888_9);
        let hunter = usgs("USGS-0137499350", 41.261_944_4, -73.842_777_8);
        let ended = parse_rfc3339("2009-07-01T04:45:00Z");
        let cross = usgs("USGS-01374901", 41.266_1, -73.666_1);
        let input = DiscoveryInput {
            latitude: 41.2190,
            longitude: -73.8660,
            now: now(),
            candidates: vec![
                candidate(
                    dam,
                    &[
                        (M::Discharge, Some(ago(15))),
                        (M::GageHeight, Some(ago(15))),
                    ],
                ),
                candidate(hunter, &[(M::Discharge, ended)]),
                candidate(cross, &[(M::Discharge, Some(ago(30)))]),
            ],
            chains: BTreeMap::from([(
                M::Discharge,
                chain(&["usgs", "nwps"], Some(10.0), Some(180)),
            )]),
            ..DiscoveryInput::default()
        };
        let ranking = rank(&input);
        let flow = &ranking.measurements[&M::Discharge];
        assert_eq!(ids(flow), ["USGS-01375000"]);
        assert_eq!(flow.stations[0].age_s, Some(15 * 60));
        assert!(matches!(
            dropped(flow, "USGS-0137499350")[..],
            [Reason::Stale {
                limit_s: 10_800,
                ..
            }]
        ));
        assert!(matches!(
            dropped(flow, "USGS-01374901")[..],
            [Reason::TooFar { distance_m, .. }] if *distance_m > 10_000.0
        ));
        let plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
        let link = first(&plan, M::Discharge);
        assert_eq!(link_station(link), ("usgs", "USGS-01375000", None));
        let chain = &plan.chains[&M::Discharge];
        assert_eq!(chain.max_age_s, 10_800);
        assert_eq!(chain.skipped[0].reason, SkipReason::NoStation);
    }

    #[test]
    fn a_forecast_gauge_takes_its_sites_relation_and_pin() {
        let site = StationKey::new(SourceId::USGS, "USGS-01374901");
        let gauge = |id, lat, lon| {
            let st = station(SourceId::NWPS, id, StationKind::NwpsGauge, lat, lon);
            candidate(st, &[(M::FloodCategories, Some(ago(30)))])
        };
        let mut input = DiscoveryInput {
            latitude: 41.2650,
            longitude: -73.6700,
            now: now(),
            candidates: vec![
                gauge("KTNN6", 41.266_1, -73.666_1),
                gauge("XRVN6", 41.260_3, -73.601_9),
            ],
            chains: BTreeMap::from([(M::FloodCategories, chain(&["nwps"], None, None))]),
            relations: BTreeMap::from([(site.clone(), Relation::SameWaterBody)]),
            joins: BTreeMap::from([(StationKey::new(SourceId::NWPS, "KTNN6"), site.clone())]),
            ..DiscoveryInput::default()
        };
        input.choices.pin(site.clone());
        let ranking = rank(&input);
        let ktnn6 = &ranking.measurements[&M::FloodCategories].stations[0];
        assert_eq!(ktnn6.station.id, "KTNN6");
        assert_eq!(ktnn6.relation, Relation::SameWaterBody);
        assert!(ktnn6.pinned);
        let joined = Reason::ForecastGaugeFor { site: site.clone() };
        assert!(ktnn6.reasons.contains(&joined));
        input.choices.exclude(site);
        let ranking = rank(&input);
        assert_eq!(ids(&ranking.measurements[&M::FloodCategories]), ["XRVN6"]);
    }

    #[test]
    fn river_values_come_from_the_spots_own_river() {
        let site = |id, lat, lon| {
            let kind = StationKind::UsgsSite(UsgsSiteType::Stream);
            station(SourceId::USGS, id, kind, lat, lon)
        };
        let gauge = |id, lat, lon| station(SourceId::NWPS, id, StationKind::NwpsGauge, lat, lon);
        let fresh = Some(ago(15));
        let flow = [(M::Discharge, fresh), (M::WaterTemperature, fresh)];
        let stages = [(M::FloodCategories, fresh)];
        let dam = StationKey::new(SourceId::USGS, "USGS-01375000");
        let brook = StationKey::new(SourceId::USGS, "USGS-0137499455");
        let hackensack = StationKey::new(SourceId::NWPS, "WNYN6");
        let mut input = DiscoveryInput {
            latitude: 41.2252,
            longitude: -73.8588,
            now: now(),
            candidates: vec![
                candidate(site("USGS-01375000", 41.225_03, -73.858_89), &flow),
                candidate(site("USGS-0137499455", 41.226_48, -73.846_80), &flow),
                candidate(gauge("WNYN6", 41.095_56, -73.964_44), &stages),
                candidate(gauge("KTNN6", 41.266_11, -73.666_11), &stages),
            ],
            chains: BTreeMap::from([
                (M::Discharge, chain(&["usgs"], None, None)),
                (M::WaterTemperature, chain(&["usgs"], None, None)),
                (M::FloodCategories, chain(&["nwps"], None, None)),
            ]),
            // The brook joins the river at its mouth, and the Hackensack is another river;
            // the hydrography places the spot but does not reach Katonah.
            relations: BTreeMap::from([
                (dam.clone(), Relation::Upstream),
                (brook.clone(), Relation::Nearby),
                (hackensack.clone(), Relation::Nearby),
            ]),
            spot_placed: true,
            ..DiscoveryInput::default()
        };
        let ranking = rank(&input);
        let discharge = &ranking.measurements[&M::Discharge];
        assert_eq!(ids(discharge), ["USGS-01375000"]);
        let other = Reason::OtherWaterBody {
            relation: Relation::Nearby,
        };
        assert_eq!(dropped(discharge, "USGS-0137499455"), [&other]);
        // Water temperature is no river measurement: the brook's mouth still counts.
        let temp = &ranking.measurements[&M::WaterTemperature];
        assert_eq!(ids(temp), ["USGS-01375000", "USGS-0137499455"]);

        // Katonah is not placed and West Nyack is on another river: both are dropped.
        let floods = &ranking.measurements[&M::FloodCategories];
        assert_eq!(ids(floods), [] as [&str; 0]);
        assert_eq!(dropped(floods, "KTNN6"), [&Reason::NotPlaced]);
        assert_eq!(dropped(floods, "WNYN6"), [&other]);
        // Water temperature takes an unplaced station by distance.
        input.candidates.push(candidate(
            site("USGS-01374901", 41.266_11, -73.666_11),
            &flow,
        ));
        let ranking = rank(&input);
        assert_eq!(
            dropped(&ranking.measurements[&M::Discharge], "USGS-01374901"),
            [&Reason::NotPlaced]
        );
        let temp = &ranking.measurements[&M::WaterTemperature];
        assert_eq!(ids(temp).last(), Some(&"USGS-01374901"));
        input.candidates.pop();

        // A gauge takes its site's relation over its own `Nearby`; a pin keeps other water.
        input
            .joins
            .insert(StationKey::new(SourceId::NWPS, "KTNN6"), dam.clone());
        input
            .relations
            .insert(StationKey::new(SourceId::NWPS, "KTNN6"), Relation::Nearby);
        input.choices.pin(brook);
        input.choices.pin(hackensack);
        let ranking = rank(&input);
        let floods = &ranking.measurements[&M::FloodCategories];
        assert_eq!(ids(floods), ["WNYN6", "KTNN6"]);
        assert!(floods.stations[0].reasons.contains(&other));
        assert_eq!(floods.stations[1].relation, Relation::Upstream);
        let discharge = &ranking.measurements[&M::Discharge];
        assert_eq!(ids(discharge), ["USGS-0137499455", "USGS-01375000"]);

        // A pin keeps a station the hydrography does not place, and says so.
        input.joins.clear();
        input
            .relations
            .remove(&StationKey::new(SourceId::NWPS, "KTNN6"));
        input.choices.pin(StationKey::new(SourceId::NWPS, "KTNN6"));
        let ranking = rank(&input);
        let floods = &ranking.measurements[&M::FloodCategories];
        let katonah = floods
            .stations
            .iter()
            .find(|s| s.station.id == "KTNN6")
            .expect("pinned");
        assert!(katonah.reasons.contains(&Reason::NotPlaced));
        assert_eq!(katonah.relation, Relation::Nearby);
    }

    #[test]
    fn a_spot_off_the_network_keeps_river_stations_by_distance() {
        let gauge = |id, lat, lon| {
            let st = station(SourceId::NWPS, id, StationKind::NwpsGauge, lat, lon);
            candidate(st, &[(M::FloodCategories, Some(ago(30)))])
        };
        // The hydrography places neither the spot nor any gauge: no relation is known.
        let input = DiscoveryInput {
            latitude: 41.2252,
            longitude: -73.8588,
            now: now(),
            candidates: vec![
                gauge("WNYN6", 41.095_56, -73.964_44),
                gauge("KTNN6", 41.266_11, -73.666_11),
            ],
            chains: BTreeMap::from([(M::FloodCategories, chain(&["nwps"], None, None))]),
            spot_placed: false,
            ..DiscoveryInput::default()
        };
        let ranking = rank(&input);
        let floods = &ranking.measurements[&M::FloodCategories];
        assert_eq!(ids(floods), ["KTNN6", "WNYN6"]);
        let nearby = Reason::Relation {
            relation: Relation::Nearby,
        };
        for s in &floods.stations {
            assert_eq!(s.relation, Relation::Nearby);
            assert!(s.reasons.contains(&nearby), "{:?}", s.reasons);
        }
        assert_eq!(floods.dropped, []);
    }

    #[test]
    fn ranking_does_not_depend_on_the_order_of_the_lists() {
        let input = battery();
        // Water levels before tide predictions: the merged station is the same.
        let tide_list = input
            .candidates
            .iter()
            .take_while(|c| c.offers.contains_key(&M::Tide))
            .count();
        let mut moved = input.clone();
        moved.candidates.rotate_left(tide_list);
        assert_eq!(rank(&moved), rank(&input));
        let json = serde_json::to_value(rank(&input).plan(2)).unwrap();
        let place = &json["chains"]["tide"]["links"][0]["place"];
        assert_eq!(place["station"]["id"], "8518750");
    }

    const BELTZVILLE: (f64, f64) = (40.8478, -75.6383);

    fn beltzville_input(lake_level: &[&str]) -> DiscoveryInput {
        let dam = station(
            SourceId::CWMS,
            "NAP/Beltzville",
            StationKind::WaterLevel,
            BELTZVILLE.0,
            BELTZVILLE.1,
        );
        let gauge = station(
            SourceId::USGS,
            "USGS-01449360",
            StationKind::UsgsSite(UsgsSiteType::Stream),
            40.8,
            -75.7,
        );
        DiscoveryInput {
            latitude: 40.86,
            longitude: -75.65,
            now: now(),
            candidates: vec![
                candidate(dam, &[(M::LakeLevel, Some(ago(30)))]),
                candidate(gauge, &[(M::Discharge, Some(ago(30)))]),
            ],
            chains: BTreeMap::from([(M::LakeLevel, chain(lake_level, None, None))]),
            ..DiscoveryInput::default()
        }
    }

    #[test]
    fn a_reservoir_project_ranks_from_its_own_source() {
        let ranking = rank(&beltzville_input(&["usgs", "cwms", "rise"]));
        let level = &ranking.measurements[&M::LakeLevel];
        assert_eq!(ids(level), ["NAP/Beltzville"]);
        assert!(level.stations[0].distance_m < 2_000.0);
        let plan = ranking.plan(DEFAULT_STATIONS_PER_SOURCE);
        let chain = &plan.chains[&M::LakeLevel];
        assert_eq!(
            chain.links.iter().map(link_station).collect::<Vec<_>>(),
            [("cwms", "NAP/Beltzville", None)]
        );
        // USGS has no lake-level station here: said, not hidden.
        assert_eq!(chain.skipped.len(), 2);
        assert!(
            chain
                .skipped
                .iter()
                .all(|s| s.reason == SkipReason::NoStation)
        );
    }

    #[test]
    fn grid_and_satellite_sources_get_point_links() {
        let mut input = beltzville_input(&[]);
        input.latitude = 41.7;
        input.longitude = -83.3;
        input.candidates.clear();
        input.chains = BTreeMap::from([
            (M::WaterTemperature, chain(&["usgs", "glerl"], None, None)),
            (M::Bloom, chain(&["cyan", "noaa-hab"], None, None)),
        ]);
        let plan = rank(&input).plan(DEFAULT_STATIONS_PER_SOURCE);
        let relation = |m: M, source: &str| {
            plan.chains[&m].links.iter().find_map(|l| match &l.place {
                LinkPlace::Point { relation, .. } if l.source.as_str() == source => Some(*relation),
                _ => None,
            })
        };
        assert_eq!(
            relation(M::WaterTemperature, "glerl"),
            Some(Relation::Gridded)
        );
        assert_eq!(relation(M::Bloom, "cyan"), Some(Relation::Satellite));
        assert_eq!(relation(M::Bloom, "noaa-hab"), Some(Relation::Gridded));
    }

    #[test]
    fn haversine_matches_a_known_distance() {
        // The Battery to Sandy Hook, 0.2337 degrees of latitude apart.
        let d = distance_m(40.7006, -74.0142, 40.4669, -74.0094);
        assert!((d - 26_000.0).abs() < 100.0, "{d}");
    }
}
