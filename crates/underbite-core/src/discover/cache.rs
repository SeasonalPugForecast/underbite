//! Per-spot discovery results with the user's pinned and excluded stations.
//!
//! [`StationChoices`] are a spot's pins and exclusions. [`ObservedCoverage`] holds what
//! fetches found at stations (NDBC columns that hold values, USGS series that are live), merged
//! in after each fetch. [`DiscoveryCache`] keeps the [`Ranking`] of each spot until the spot
//! moves, its choices, chains or limits change, the observed coverage changes, or it gets older
//! than the cache's age limit. New station lists or hydrography call for
//! [`DiscoveryCache::invalidate`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::rank::{ChainSpec, DiscoveryInput, RANK_RULES, Ranking, StationKey, distance_m, rank};
use crate::model::{Coverage, Measurement};
use crate::provenance::SourceId;
use crate::time::UnixTime;

/// How far a spot may move before its stations are ranked again, metres.
pub const MOVE_TOLERANCE_M: f64 = 10.0;
/// Age after which a spot's stations are ranked again, seconds.
pub const DEFAULT_CACHE_AGE_S: i64 = 6 * 3_600;

/// A spot's pinned and excluded stations. An exclusion wins over a pin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationChoices {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pinned: BTreeSet<StationKey>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub excluded: BTreeSet<StationKey>,
}

impl StationChoices {
    /// Pins a station (or one bin of it), lifting its exclusion.
    pub fn pin(&mut self, key: StationKey) {
        self.excluded.remove(&key);
        self.pinned.insert(key);
    }

    /// Excludes a station (or one bin of it), lifting its pin.
    pub fn exclude(&mut self, key: StationKey) {
        self.pinned.remove(&key);
        self.excluded.insert(key);
    }

    /// Lifts the pin or exclusion of `key`.
    pub fn clear(&mut self, key: &StationKey) {
        self.pinned.remove(key);
        self.excluded.remove(key);
    }

    /// Whether a pin covers the station or bin `key`.
    pub fn is_pinned(&self, key: &StationKey) -> bool {
        self.pinned.iter().any(|p| p.covers(key))
    }

    /// Whether an exclusion covers the station or bin `key`.
    pub fn is_excluded(&self, key: &StationKey) -> bool {
        self.excluded.iter().any(|e| e.covers(key))
    }
}

/// What a check found for one measurement of one station.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    pub checked_at: UnixTime,
    /// `None` when the check found no value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
}

/// Coverage observed at stations, per source, station id (bins share it) and measurement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedCoverage {
    stations: BTreeMap<SourceId, BTreeMap<String, BTreeMap<Measurement, Observed>>>,
    /// Counts changes to what stations report, times aside.
    revision: u64,
}

impl ObservedCoverage {
    /// Records a check of `station` at `at`: `found` is what it reports, and every
    /// measurement of `checked` missing from `found` is recorded as not reported. Returns
    /// whether that changed what the station is known to report.
    pub fn merge(
        &mut self,
        station: &StationKey,
        checked: &BTreeSet<Measurement>,
        found: &BTreeMap<Measurement, Coverage>,
        at: UnixTime,
    ) -> bool {
        let entry = self
            .stations
            .entry(station.source.clone())
            .or_default()
            .entry(station.id.clone())
            .or_default();
        let shape = |o: &Observed| o.coverage.map(|c| c.primary);
        let mut changed = false;
        for m in checked.iter().chain(found.keys()) {
            let new = Observed {
                checked_at: at,
                coverage: found.get(m).copied(),
            };
            let old = entry.insert(*m, new);
            changed |= old.is_none_or(|o| shape(&o) != shape(&new));
        }
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// The last check of `measurement` at `station` (any bin).
    pub fn get(&self, station: &StationKey, measurement: Measurement) -> Option<&Observed> {
        self.stations
            .get(&station.source)?
            .get(&station.id)?
            .get(&measurement)
    }

    /// Forgets checks older than `max_age_s`, so a station that went quiet gets another chance.
    pub fn prune(&mut self, now: UnixTime, max_age_s: i64) {
        let oldest = now.0 - max_age_s;
        let mut removed = false;
        for ids in self.stations.values_mut() {
            for checks in ids.values_mut() {
                let before = checks.len();
                checks.retain(|_, o| o.checked_at.0 >= oldest);
                removed |= checks.len() != before;
            }
            ids.retain(|_, checks| !checks.is_empty());
        }
        self.stations.retain(|_, ids| !ids.is_empty());
        if removed {
            self.revision += 1;
        }
    }

    /// Changes so far; a ranking made at an older revision is out of date.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Why a cached ranking cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum Invalid {
    Missing,
    Moved {
        distance_m: f64,
    },
    ChoicesChanged,
    ChainsChanged,
    CoverageChanged,
    /// The hydrography relates the stations or places the spot differently.
    HydrographyChanged,
    /// The ranking was made under other rules.
    RulesChanged,
    Expired {
        age_s: i64,
    },
}

/// Rankings per spot id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryCache {
    /// Age after which a ranking is redone, seconds.
    pub max_age_s: i64,
    spots: BTreeMap<String, Ranking>,
}

impl Default for DiscoveryCache {
    fn default() -> Self {
        Self::new(DEFAULT_CACHE_AGE_S)
    }
}

fn chains_match(ranking: &Ranking, chains: &BTreeMap<Measurement, ChainSpec>) -> bool {
    ranking.measurements.len() == chains.len()
        && ranking
            .measurements
            .iter()
            .all(|(m, r)| chains.get(m) == Some(&r.chain))
}

impl DiscoveryCache {
    /// An empty cache whose rankings last `max_age_s`.
    pub fn new(max_age_s: i64) -> Self {
        Self {
            max_age_s,
            spots: BTreeMap::new(),
        }
    }

    /// The cached ranking of `spot_id` if it still fits `input`.
    pub fn check(&self, spot_id: &str, input: &DiscoveryInput) -> Result<&Ranking, Invalid> {
        let ranking = self.spots.get(spot_id).ok_or(Invalid::Missing)?;
        if ranking.rules != RANK_RULES {
            return Err(Invalid::RulesChanged);
        }
        let moved = distance_m(
            ranking.latitude,
            ranking.longitude,
            input.latitude,
            input.longitude,
        );
        if moved > MOVE_TOLERANCE_M {
            return Err(Invalid::Moved { distance_m: moved });
        }
        if ranking.choices != input.choices {
            return Err(Invalid::ChoicesChanged);
        }
        if !chains_match(ranking, &input.chains) {
            return Err(Invalid::ChainsChanged);
        }
        if ranking.coverage_revision != input.observed.revision() {
            return Err(Invalid::CoverageChanged);
        }
        if ranking.relations != input.relations || ranking.spot_placed != input.spot_placed {
            return Err(Invalid::HydrographyChanged);
        }
        let age_s = input.now.0 - ranking.at.0;
        if !(0..=self.max_age_s).contains(&age_s) {
            return Err(Invalid::Expired { age_s });
        }
        Ok(ranking)
    }

    /// The ranking of `spot_id` for `input`, from the cache when it still fits.
    pub fn ranking(&mut self, spot_id: &str, input: &DiscoveryInput) -> &Ranking {
        if self.check(spot_id, input).is_err() {
            self.spots.insert(spot_id.to_string(), rank(input));
        }
        // INVARIANT: the entry was either valid or inserted just above.
        &self.spots[spot_id]
    }

    /// Drops the ranking of `spot_id`, after new station lists or hydrography.
    pub fn invalidate(&mut self, spot_id: &str) {
        self.spots.remove(spot_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::rank::Limits;

    fn key(id: &str) -> StationKey {
        StationKey::new(SourceId::NDBC, id)
    }

    fn checked() -> BTreeSet<Measurement> {
        BTreeSet::from([Measurement::ObservedWind, Measurement::WaterTemperature])
    }

    fn found(at: i64) -> BTreeMap<Measurement, Coverage> {
        BTreeMap::from([(
            Measurement::WaterTemperature,
            Coverage {
                last_value_at: Some(UnixTime(at)),
                primary: true,
            },
        )])
    }

    fn input() -> DiscoveryInput {
        DiscoveryInput {
            latitude: 40.7033,
            longitude: -74.0170,
            now: UnixTime(1_000_000),
            chains: BTreeMap::from([(
                Measurement::Tide,
                ChainSpec {
                    sources: vec![SourceId::COOPS],
                    limits: Limits::default(),
                },
            )]),
            ..DiscoveryInput::default()
        }
    }

    #[test]
    fn choices_pin_and_exclude_replace_each_other() {
        let mut c = StationChoices::default();
        let nyh = StationKey::new(SourceId::COOPS, "NYH1927");
        c.pin(nyh.clone());
        assert!(c.is_pinned(&nyh.clone().with_bin(13)));
        c.exclude(nyh.clone().with_bin(8));
        assert!(c.is_excluded(&nyh.clone().with_bin(8)));
        assert!(!c.is_excluded(&nyh.clone().with_bin(13)));
        c.exclude(nyh.clone());
        assert!(!c.is_pinned(&nyh) && c.is_excluded(&nyh.clone().with_bin(13)));
        c.clear(&nyh);
        c.clear(&nyh.clone().with_bin(8));
        assert_eq!(c, StationChoices::default());
    }

    #[test]
    fn merge_records_missing_columns_and_counts_only_real_changes() {
        let mut o = ObservedCoverage::default();
        assert!(o.merge(&key("44025"), &checked(), &found(100), UnixTime(100)));
        assert_eq!(o.revision(), 1);
        let wind = o.get(&key("44025"), Measurement::ObservedWind).unwrap();
        assert_eq!(wind.coverage, None);
        assert_eq!(wind.checked_at, UnixTime(100));
        // Same shape, newer times: no change, but the times move on.
        assert!(!o.merge(&key("44025"), &checked(), &found(200), UnixTime(200)));
        assert_eq!(o.revision(), 1);
        let temp = o.get(&key("44025"), Measurement::WaterTemperature).unwrap();
        assert_eq!(temp.coverage.unwrap().last_value_at, Some(UnixTime(200)));
        // Bins share the station's coverage.
        assert!(
            o.get(&key("44025").with_bin(3), Measurement::WaterTemperature)
                .is_some()
        );
        // Wind comes back.
        let mut more = found(300);
        more.insert(
            Measurement::ObservedWind,
            more[&Measurement::WaterTemperature],
        );
        assert!(o.merge(&key("44025"), &checked(), &more, UnixTime(300)));
        assert_eq!(o.revision(), 2);
    }

    #[test]
    fn prune_forgets_old_checks() {
        let mut o = ObservedCoverage::default();
        o.merge(&key("44025"), &checked(), &found(100), UnixTime(100));
        o.merge(&key("44065"), &checked(), &found(900), UnixTime(900));
        o.prune(UnixTime(1_000), 500);
        assert!(o.get(&key("44025"), Measurement::ObservedWind).is_none());
        assert!(o.get(&key("44065"), Measurement::ObservedWind).is_some());
        assert_eq!(o.revision(), 3);
        o.prune(UnixTime(1_000), 500);
        assert_eq!(o.revision(), 3);
    }

    #[test]
    fn cached_ranking_is_reused_until_an_input_changes() {
        let mut cache = DiscoveryCache::new(3_600);
        let base = input();
        assert_eq!(cache.check("battery", &base), Err(Invalid::Missing));
        let at = cache.ranking("battery", &base).at;
        assert_eq!(at, base.now);
        assert!(cache.check("battery", &base).is_ok());

        let mut later = base.clone();
        later.now = UnixTime(base.now.0 + 3_600);
        assert!(cache.check("battery", &later).is_ok());
        later.now = UnixTime(base.now.0 + 3_601);
        assert_eq!(
            cache.check("battery", &later),
            Err(Invalid::Expired { age_s: 3_601 })
        );

        let mut nudged = base.clone();
        nudged.latitude += 0.00005; // about 5.6 m
        assert!(cache.check("battery", &nudged).is_ok());
        nudged.latitude += 0.001;
        assert!(matches!(
            cache.check("battery", &nudged),
            Err(Invalid::Moved { distance_m }) if distance_m > 100.0
        ));

        let mut pinned = base.clone();
        pinned
            .choices
            .pin(StationKey::new(SourceId::COOPS, "8518750"));
        assert_eq!(
            cache.check("battery", &pinned),
            Err(Invalid::ChoicesChanged)
        );

        let mut limits = base.clone();
        limits.chains.get_mut(&Measurement::Tide).unwrap().limits =
            Limits::from_config(Some(5.0), None);
        assert_eq!(cache.check("battery", &limits), Err(Invalid::ChainsChanged));
        let mut fewer = base.clone();
        fewer.chains.clear();
        assert_eq!(cache.check("battery", &fewer), Err(Invalid::ChainsChanged));

        let mut checked_again = base.clone();
        checked_again
            .observed
            .merge(&key("44025"), &checked(), &found(1), UnixTime(1));
        assert_eq!(
            cache.check("battery", &checked_again),
            Err(Invalid::CoverageChanged)
        );
        assert_eq!(
            cache.ranking("battery", &checked_again).coverage_revision,
            1
        );
        assert!(cache.check("battery", &checked_again).is_ok());

        let mut placed = checked_again.clone();
        placed.spot_placed = !checked_again.spot_placed;
        assert_eq!(
            cache.check("battery", &placed),
            Err(Invalid::HydrographyChanged)
        );
        let mut related = checked_again.clone();
        related.relations.insert(
            StationKey::new(SourceId::COOPS, "8518750"),
            crate::provenance::Relation::SameWaterBody,
        );
        assert_eq!(
            cache.check("battery", &related),
            Err(Invalid::HydrographyChanged)
        );

        cache.invalidate("battery");
        assert_eq!(cache.check("battery", &base), Err(Invalid::Missing));
    }

    #[test]
    fn a_ranking_with_relations_round_trips_through_json() {
        let mut cache = DiscoveryCache::new(3_600);
        let mut related = input();
        related.spot_placed = true;
        related.relations.insert(
            StationKey::new(SourceId::COOPS, "8518750"),
            crate::provenance::Relation::SameWaterBody,
        );
        cache.ranking("battery", &related);
        let json = serde_json::to_string(&cache).expect("a station key is not a map key");
        let back: DiscoveryCache = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cache);
        assert!(back.check("battery", &related).is_ok());
    }

    #[test]
    fn a_ranking_from_older_rules_is_redone() {
        let mut cache = DiscoveryCache::new(3_600);
        let base = input();
        cache.ranking("battery", &base);
        // A ranking stored before the rules were versioned decodes with version 0.
        let mut json = serde_json::to_value(&cache).unwrap();
        json["spots"]["battery"]
            .as_object_mut()
            .unwrap()
            .remove("rules");
        let old: DiscoveryCache = serde_json::from_value(json).unwrap();
        assert_eq!(old.check("battery", &base), Err(Invalid::RulesChanged));
    }

    #[test]
    fn cache_round_trips_through_json() {
        let mut cache = DiscoveryCache::default();
        let mut base = input();
        base.choices.exclude(key("44025"));
        base.observed
            .merge(&key("44025"), &checked(), &found(1), UnixTime(1));
        cache.ranking("battery", &base);
        let json = serde_json::to_string(&cache).unwrap();
        let back: DiscoveryCache = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cache);
        let observed: ObservedCoverage =
            serde_json::from_str(&serde_json::to_string(&base.observed).unwrap()).unwrap();
        assert_eq!(observed, base.observed);
    }
}
