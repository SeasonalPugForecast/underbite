//! Per-spot discovery results with the user's pinned and excluded stations.
//!
//! [`StationChoices`] are a spot's pins and exclusions. [`ObservedCoverage`] holds what
//! fetches found at stations (NDBC columns that hold values, USGS series that are live), merged
//! in after each fetch.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::rank::StationKey;
use crate::model::{Coverage, Measurement};
use crate::provenance::SourceId;
use crate::time::UnixTime;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
