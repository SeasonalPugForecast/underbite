//! The catch log: entries with the conditions at the time, exports, and what worked.
//!
//! The log is the user's private data. It is stored on the device as one JSON document
//! ([`LogBook`]) and leaves it only in what the user exports or shares. Every record has a stable
//! id and a modification time, and a deletion is kept as a tombstone, so a later sync can merge
//! two devices' books without any change of identity. A log is never part of a `.underbite` file.

pub mod entry;
pub mod export;
pub mod insights;

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

pub use entry::{
    CatchConditions, CatchEntry, CatchLocation, Fate, Light, PhotoRef, PhotoRefError, Sourced,
};

use crate::time::UnixTime;

/// The format version this build writes and the newest it reads.
pub const LOG_VERSION: u32 = 1;

/// A log that could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogError {
    /// The document is not a valid log.
    #[error("the catch log is not valid: {0}")]
    Parse(String),
    /// The document was written by a newer version of the app.
    #[error("the catch log is format {found}, newer than this app reads ({supported})")]
    Newer {
        /// The version in the document.
        found: u32,
        /// The newest version this build reads.
        supported: u32,
    },
}

#[derive(Deserialize)]
struct Header {
    v: u32,
}

#[derive(Deserialize)]
struct RawBook {
    v: u32,
    #[serde(default)]
    entries: Vec<CatchEntry>,
}

/// All of a user's catches, with tombstones, in a stable order.
///
/// Entries are ordered by `caught_at`, then `id`. There is at most one record per id.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LogBook {
    v: u32,
    entries: Vec<CatchEntry>,
}

impl Default for LogBook {
    fn default() -> Self {
        Self::new()
    }
}

fn order(a: &CatchEntry, b: &CatchEntry) -> Ordering {
    a.caught_at.cmp(&b.caught_at).then_with(|| a.id.cmp(&b.id))
}

/// Whether `new` replaces `old`, two records of one id.
///
/// The later `modified_at` wins. On a tie a tombstone wins over a live entry, and between two
/// records of the same kind the greater JSON text wins: arbitrary, but the same on every device,
/// so merging in either order gives the same book.
fn replaces(new: &CatchEntry, old: &CatchEntry) -> bool {
    match new.modified_at.cmp(&old.modified_at) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal if new.deleted != old.deleted => new.deleted,
        Ordering::Equal => {
            let text = |e: &CatchEntry| serde_json::to_string(e).unwrap_or_default();
            text(new) > text(old)
        }
    }
}

impl LogBook {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self {
            v: LOG_VERSION,
            entries: Vec::new(),
        }
    }

    /// Reads a log from its JSON text. Fields this version does not know are ignored; records of
    /// one id are merged as [`upsert`](Self::upsert) would.
    ///
    /// # Errors
    /// [`LogError::Newer`] when the document's `v` is newer than [`LOG_VERSION`], checked before
    /// anything else is read so a newer shape is never half-parsed; [`LogError::Parse`] otherwise
    /// when it is not a log.
    pub fn from_json(text: &str) -> Result<Self, LogError> {
        let header: Header =
            serde_json::from_str(text).map_err(|e| LogError::Parse(e.to_string()))?;
        if header.v > LOG_VERSION {
            return Err(LogError::Newer {
                found: header.v,
                supported: LOG_VERSION,
            });
        }
        let raw: RawBook =
            serde_json::from_str(text).map_err(|e| LogError::Parse(e.to_string()))?;
        let mut book = Self::new();
        for entry in raw.entries {
            book.upsert(entry);
        }
        debug_assert!(raw.v <= LOG_VERSION);
        Ok(book)
    }

    /// The log as JSON text. The same book always gives the same text.
    ///
    /// # Errors
    /// [`LogError::Parse`] if serialization fails, which it does not for these types.
    pub fn to_json(&self) -> Result<String, LogError> {
        serde_json::to_string(self).map_err(|e| LogError::Parse(e.to_string()))
    }

    /// Adds `entry` or replaces the record of its id when `entry` is the later write (see
    /// [`LogBook`]'s ordering rules: last writer wins on `modified_at`, ties settled the same way
    /// on every device). Returns whether the book changed.
    pub fn upsert(&mut self, entry: CatchEntry) -> bool {
        match self.entries.iter().position(|e| e.id == entry.id) {
            Some(i) => {
                if !replaces(&entry, &self.entries[i]) {
                    return false;
                }
                self.entries[i] = entry;
            }
            None => self.entries.push(entry),
        }
        self.entries.sort_by(order);
        true
    }

    /// Deletes the entry `id`: its record becomes a tombstone (`deleted`, `modified_at` set to
    /// `now`, never earlier than its last edit) that is kept so a sync can carry the deletion. The
    /// tombstone holds no position, notes or photos.
    ///
    /// Returns the photo references the entry had, so the host can delete the files, or `None`
    /// when there is no live entry with that id.
    pub fn delete(&mut self, id: &str, now: UnixTime) -> Option<Vec<PhotoRef>> {
        let i = self.entries.iter().position(|e| e.id == id && !e.deleted)?;
        let tombstone = self.entries[i].tombstone(now);
        let photos = std::mem::replace(&mut self.entries[i], tombstone).photos;
        Some(photos)
    }

    /// Merges another book into this one, record by record. Merging is commutative and
    /// idempotent. Returns how many records changed.
    pub fn merge(&mut self, other: LogBook) -> usize {
        other
            .entries
            .into_iter()
            .filter(|e| self.upsert(e.clone()))
            .count()
    }

    /// Every record including tombstones, in stable order.
    #[must_use]
    pub fn entries(&self) -> &[CatchEntry] {
        &self.entries
    }

    /// The record of `id`, a tombstone if it was deleted.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&CatchEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The entries that are not deleted, in stable order.
    pub fn live(&self) -> impl Iterator<Item = &CatchEntry> {
        self.entries.iter().filter(|e| !e.deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datum::{Height, VerticalDatum};
    use crate::derive::pressure::Trend;
    use crate::log::entry::{CurrentSet, PressureTrend, TideDirection, TurnKind, WindCondition};
    use crate::model::{
        SolunarKind, Spot, SpotSnapshot, TideEvent, TideKind, TideSeries, TideSourceKind,
        TurbidityUnit, WaterType,
    };
    use crate::provenance::{
        Provenance, Quality, Relation, SourceId, StationKind, StationRef, ValueKind,
    };
    use crate::series::{Sample, Series};
    use crate::time::parse_coops;
    use crate::trip::SpeciesId;

    fn entry(id: &str, caught: i64, modified: i64) -> CatchEntry {
        CatchEntry::new(
            id,
            UnixTime(caught),
            "America/New_York",
            CatchLocation::new(40.7003, -74.0142),
            UnixTime(modified),
        )
    }

    fn ids(book: &LogBook) -> Vec<&str> {
        book.entries().iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn empty_book_is_versioned() {
        assert_eq!(LogBook::new().to_json().unwrap(), r#"{"v":1,"entries":[]}"#);
        assert_eq!(LogBook::from_json(r#"{"v":1}"#).unwrap(), LogBook::new());
    }

    #[test]
    fn full_entry_round_trips() {
        let mut e = entry("0001", 1_794_000_000, 1_794_000_100);
        e.location.spot_id = Some("battery".into());
        e.location.area = Some("Lower Hudson".into());
        e.species = Some(SpeciesId::new("morone-saxatilis").unwrap());
        e.species_text = Some("schoolie".into());
        e.length_m = Some(0.71);
        e.weight_kg = Some(3.9);
        e.fate = Fate::KeptForSashimi;
        e.lure = Some("white bucktail".into());
        e.method = Some("jigging".into());
        e.notes = Some("first of the incoming".into());
        e.photos = vec![PhotoRef::new("0001-a.heic").unwrap()];
        e.conditions = Some(CatchConditions {
            at: e.caught_at,
            water_temperature_c: Some(Sourced {
                value: 16.0,
                source: SourceId::COOPS,
                station: Some("8518750".into()),
                station_name: Some("The Battery".into()),
                age_s: 120,
            }),
            light: Some(Light::Dawn),
            ..CatchConditions::default()
        });
        let mut book = LogBook::new();
        book.upsert(e.clone());
        book.upsert(entry("0002", 1_793_000_000, 5));
        let json = book.to_json().unwrap();
        let back = LogBook::from_json(&json).unwrap();
        assert_eq!(back, book);
        assert_eq!(back.get("0001"), Some(&e));
        // Deterministic: re-serializing gives the same text.
        assert_eq!(back.to_json().unwrap(), json);
        assert!(json.contains(r#""fate":"kept_for_sashimi""#));
        assert!(json.contains(r#""species":"morone-saxatilis""#));
    }

    #[test]
    fn entries_are_ordered_by_catch_time_then_id() {
        let mut book = LogBook::new();
        book.upsert(entry("b", 200, 1));
        book.upsert(entry("c", 100, 1));
        book.upsert(entry("a", 200, 1));
        assert_eq!(ids(&book), ["c", "a", "b"]);
        let back = LogBook::from_json(&book.to_json().unwrap()).unwrap();
        assert_eq!(ids(&back), ["c", "a", "b"]);
        // Moving a catch in time reorders it.
        let mut moved = entry("c", 300, 2);
        moved.notes = Some("fixed the date".into());
        assert!(book.upsert(moved));
        assert_eq!(ids(&book), ["a", "b", "c"]);
    }

    #[test]
    fn last_writer_wins_on_modified_at() {
        let mut book = LogBook::new();
        let mut old = entry("a", 100, 10);
        old.notes = Some("old".into());
        let mut new = entry("a", 100, 20);
        new.notes = Some("new".into());
        assert!(book.upsert(new.clone()));
        assert!(!book.upsert(old));
        assert_eq!(book.get("a").unwrap().notes.as_deref(), Some("new"));
        assert!(!book.upsert(new), "an identical record changes nothing");
        assert_eq!(book.entries().len(), 1);
    }

    #[test]
    fn equal_times_settle_the_same_way_in_either_order() {
        let mut x = entry("a", 100, 10);
        x.notes = Some("from the phone".into());
        let mut y = entry("a", 100, 10);
        y.notes = Some("from the tablet".into());
        let mut gone = x.tombstone(UnixTime(10));
        gone.modified_at = UnixTime(10);

        for (p, q) in [(&x, &y), (&y, &x), (&x, &gone), (&gone, &x)] {
            let mut one = LogBook::new();
            one.upsert(p.clone());
            let mut two = LogBook::new();
            two.upsert(q.clone());
            let mut ab = one.clone();
            ab.merge(two.clone());
            let mut ba = two;
            ba.merge(one);
            assert_eq!(ab, ba);
        }
        // A tombstone beats a live entry written at the same second.
        let mut book = LogBook::new();
        book.upsert(x);
        assert!(book.upsert(gone));
        assert!(book.get("a").unwrap().deleted);
    }

    #[test]
    fn delete_keeps_a_tombstone_and_hides_the_entry() {
        let mut book = LogBook::new();
        let mut e = entry("a", 100, 10);
        e.notes = Some("private".into());
        e.photos = vec![PhotoRef::new("a.heic").unwrap()];
        book.upsert(e);
        book.upsert(entry("b", 200, 10));

        let photos = book.delete("a", UnixTime(50)).unwrap();
        assert_eq!(photos, vec![PhotoRef::new("a.heic").unwrap()]);
        assert_eq!(
            book.live().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
        assert_eq!(ids(&book), ["a", "b"], "the tombstone is kept");
        let t = book.get("a").unwrap();
        assert!(t.deleted);
        assert_eq!(t.modified_at, UnixTime(50));
        assert_eq!(t.caught_at, UnixTime(100));
        assert!(t.notes.is_none() && t.photos.is_empty());

        // Deleting twice, or something that is not there, does nothing.
        assert_eq!(book.delete("a", UnixTime(60)), None);
        assert_eq!(book.delete("zzz", UnixTime(60)), None);

        // The tombstone survives a round trip and is not resurrected by an older edit.
        let mut back = LogBook::from_json(&book.to_json().unwrap()).unwrap();
        assert!(!back.upsert(entry("a", 100, 40)));
        assert!(back.live().all(|e| e.id != "a"));
        // A later edit does bring it back (an undo on another device).
        assert!(back.upsert(entry("a", 100, 70)));
        assert!(back.live().any(|e| e.id == "a"));
    }

    #[test]
    fn a_tombstone_is_never_dated_before_the_entry() {
        let mut book = LogBook::new();
        book.upsert(entry("a", 100, 500));
        book.delete("a", UnixTime(300));
        assert_eq!(book.get("a").unwrap().modified_at, UnixTime(500));
        assert!(book.get("a").unwrap().deleted);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let text = r#"{"v":1,"future":{"x":1},"entries":[
            {"id":"a","created_at":1,"modified_at":2,"caught_at":3,"time_zone":"UTC",
             "location":{"latitude":1.5,"longitude":2.5,"datum":"wgs84"},
             "fate":"kept","mood":"smug","conditions":{"at":3,"tide_height_ft":4}}]}"#;
        let book = LogBook::from_json(text).unwrap();
        let e = book.get("a").unwrap();
        assert_eq!(e.fate, Fate::Kept);
        assert_eq!(e.location, CatchLocation::new(1.5, 2.5));
        assert!(e.conditions.as_ref().unwrap().is_empty());
        assert!(!e.deleted);
    }

    #[test]
    fn a_newer_version_is_rejected_before_its_shape_is_read() {
        let err = LogBook::from_json(r#"{"v":2,"entries":[{"shape":"unknown"}]}"#).unwrap_err();
        assert_eq!(
            err,
            LogError::Newer {
                found: 2,
                supported: 1
            }
        );
        assert!(err.to_string().contains("newer"));
        assert!(matches!(
            LogBook::from_json(r#"{"entries":[]}"#),
            Err(LogError::Parse(_))
        ));
        assert!(matches!(
            LogBook::from_json("nope"),
            Err(LogError::Parse(_))
        ));
        assert!(matches!(
            LogBook::from_json(r#"{"v":1,"entries":[{"id":"a"}]}"#),
            Err(LogError::Parse(_))
        ));
    }

    #[test]
    fn duplicate_ids_in_a_file_merge_to_the_later_write() {
        let mut a = entry("a", 100, 10);
        a.notes = Some("one".into());
        let mut b = entry("a", 100, 20);
        b.notes = Some("two".into());
        let text = serde_json::to_string(&serde_json::json!({"v":1,"entries":[b, a]})).unwrap();
        let book = LogBook::from_json(&text).unwrap();
        assert_eq!(book.entries().len(), 1);
        assert_eq!(book.get("a").unwrap().notes.as_deref(), Some("two"));
    }

    #[test]
    fn merge_is_idempotent() {
        let mut a = LogBook::new();
        a.upsert(entry("x", 1, 1));
        a.upsert(entry("y", 2, 5));
        let mut b = LogBook::new();
        b.upsert(entry("y", 2, 9));
        b.upsert(entry("z", 3, 1));
        let mut m = a.clone();
        assert_eq!(m.merge(b.clone()), 2);
        assert_eq!(m.merge(b), 0);
        assert_eq!(ids(&m), ["x", "y", "z"]);
        assert_eq!(m.get("y").unwrap().modified_at, UnixTime(9));
    }

    // ---- conditions from recorded fixtures -------------------------------------------------

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/../../tests/fixtures/raw/coops/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap()
    }

    fn prov(kind: ValueKind) -> Provenance {
        Provenance {
            source: SourceId::COOPS,
            station: Some(StationRef {
                source: SourceId::COOPS,
                id: "8518750".into(),
                name: "The Battery".into(),
                kind: StationKind::TideHarmonic,
                latitude: 40.7006,
                longitude: -74.0142,
                bin: None,
                reference_id: None,
            }),
            distance_m: Some(450.0),
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(0),
            kind,
            quality: Quality::Approved,
        }
    }

    fn data_series(name: &str, kind: ValueKind) -> Series<f32> {
        let doc = fixture(name);
        let samples = doc["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                Sample::new(
                    parse_coops(p["t"].as_str().unwrap()).unwrap(),
                    p["v"].as_str().unwrap().parse().unwrap(),
                )
            })
            .collect();
        Series::new(prov(kind), samples)
    }

    fn battery_snapshot() -> SpotSnapshot {
        let spot = Spot {
            id: "battery".into(),
            name: "The Battery".into(),
            latitude: 40.7003,
            longitude: -74.0142,
            water_type: WaterType::TidalRiverEstuary,
            water_body: None,
            time_zone: "America/New_York".into(),
        };
        let mut snap = SpotSnapshot::new(spot, UnixTime(0));
        let events = fixture("coops-8518750-hilo-2026-11-08.json")["predictions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| TideEvent {
                time: parse_coops(p["t"].as_str().unwrap()).unwrap(),
                kind: if p["type"] == "H" {
                    TideKind::High
                } else {
                    TideKind::Low
                },
                height_m: Some(p["v"].as_str().unwrap().parse().unwrap()),
            })
            .collect();
        snap.marine.tide = Some(TideSeries {
            source_kind: TideSourceKind::HarmonicStation,
            datum: VerticalDatum::Mllw,
            events,
            curve: None,
            observed: None,
            provenance: prov(ValueKind::Predicted),
        });
        snap.water.temperature_c = Some(data_series(
            "coops-8518750-water-temperature-2026-11-07.json",
            ValueKind::Observed,
        ));
        snap.marine.observed_pressure_hpa = Some(data_series(
            "coops-8518750-air-pressure-2026-11-07.json",
            ValueKind::Observed,
        ));
        snap
    }

    #[test]
    fn conditions_carry_the_tide_stage_and_its_source() {
        let snap = battery_snapshot();
        // 2026-11-08 09:00 UTC: between the 05:59 low and the 12:07 high.
        let at = parse_coops("2026-11-08 09:00").unwrap();
        let c = CatchConditions::from_snapshot(&snap, at);
        let tide = c.tide.expect("tide stage");
        assert_eq!(tide.value.direction, TideDirection::Rising);
        assert_eq!(tide.value.previous, TurnKind::Low);
        assert_eq!(tide.value.next, TurnKind::High);
        assert!((tide.value.hours_since_previous - 3.02).abs() < 0.01);
        assert!((tide.value.hours_to_next - 3.12).abs() < 0.01);
        assert_eq!(tide.source, SourceId::COOPS);
        assert_eq!(tide.station.as_deref(), Some("8518750"));
        assert_eq!(tide.station_name.as_deref(), Some("The Battery"));
        // The temperature and pressure samples end a day earlier: left out.
        assert!(c.water_temperature_c.is_none());
        assert!(c.pressure_hpa.is_none() && c.pressure_trend.is_none());
        assert_eq!(c.at, at);
    }

    #[test]
    fn conditions_carry_water_temperature_and_pressure_trend() {
        let snap = battery_snapshot();
        let at = parse_coops("2026-11-07 12:03").unwrap();
        let c = CatchConditions::from_snapshot(&snap, at);
        let t = c.water_temperature_c.expect("water temperature");
        assert_eq!(t.value, 15.9);
        assert_eq!(t.source, SourceId::COOPS);
        assert_eq!(t.station.as_deref(), Some("8518750"));
        assert_eq!(t.age_s, 180);
        let p = c.pressure_hpa.expect("pressure");
        assert!(p.value > 900.0 && p.value < 1100.0);
        assert_eq!(p.source, SourceId::COOPS);
        let trend = c.pressure_trend.expect("pressure trend");
        let series = snap.marine.observed_pressure_hpa.as_ref().unwrap();
        let expected = crate::derive::pressure::observed_trend(series, at).unwrap();
        assert_eq!(trend.value, PressureTrend::from(expected));
        assert!(matches!(
            expected,
            Trend::Rising | Trend::Falling | Trend::Steady
        ));
        // No predictions cover 11-07 midday.
        assert!(c.tide.is_none());
        // A catch two hours after the last reading has no readings.
        let late = CatchConditions::from_snapshot(&snap, parse_coops("2026-11-08 02:00").unwrap());
        assert!(late.water_temperature_c.is_none() && late.pressure_hpa.is_none());
    }

    #[test]
    fn conditions_from_the_sample_snapshot() {
        use crate::model::snapshot::testdata::sample_snapshot;
        let snap = sample_snapshot();
        let mut snap = snap;
        let noon = snap.sky.days[0].sun.sunrise.expect("sunrise").0 + 3 * 3_600;
        // The sample's sky day is labelled for another date: label it for the catch's own date.
        let tz = crate::time::zone(&snap.spot.time_zone).unwrap();
        snap.sky.days[0].date = crate::time::local_date(UnixTime(noon), &tz).unwrap();
        let day = snap.sky.days[0].clone();
        let c = CatchConditions::from_snapshot(&snap, UnixTime(noon));
        assert_eq!(c.light, Some(Light::Day));
        assert_eq!(c.moon.map(|m| m.phase), Some(day.moon.phase));
        assert_eq!(
            c.moon.map(|m| m.illuminated_fraction),
            Some(day.moon.illuminated_fraction)
        );
        // Far from every sample the water values drop out, the sky values stay.
        // Five hours before the sample sky day's sunrise the catch is in the dark, a few minutes
        // after the samples: every water and weather value is present.
        let at = UnixTime(1_794_000_400);
        let near = CatchConditions::from_snapshot(&snap, at);
        let current = near.current.as_ref().expect("current");
        assert_eq!(current.value.speed_ms, 0.5);
        assert_eq!(current.value.set, Some(CurrentSet::Ebb));
        assert_eq!(current.value.toward_deg, Some(190.0));
        let flow = near.flow.as_ref().expect("flow");
        assert_eq!(flow.value.discharge_m3s, 0.34);
        assert_eq!(flow.source, SourceId::USGS);
        assert_eq!(flow.age_s, 40);
        assert_eq!(
            flow.value.percentile, None,
            "the percentile is for another date"
        );
        assert_eq!(
            near.gage_height.as_ref().map(|g| g.value.height),
            Some(Height {
                datum: VerticalDatum::Navd88,
                metres: 1.21
            })
        );
        assert_eq!(
            near.water_temperature_c.as_ref().map(|t| t.value),
            Some(14.6)
        );
        assert_eq!(
            near.turbidity.as_ref().map(|t| t.value.unit),
            Some(TurbidityUnit::Fnu)
        );
        let pressure = near.pressure_hpa.as_ref().expect("pressure");
        assert_eq!((pressure.value, pressure.source.as_str()), (1013.2, "ndbc"));
        let wind = near.wind.as_ref().expect("wind");
        assert_eq!(
            wind.value,
            WindCondition {
                speed_ms: 6.0,
                gust_ms: Some(8.0),
                from_deg: Some(250.0)
            }
        );
        assert_eq!(near.light, Some(Light::Dawn));
        assert!(near.solunar.is_none());

        // The percentile counts only on its own date.
        let mut dated = snap.clone();
        let tz = crate::time::zone(&dated.spot.time_zone).unwrap();
        let d = crate::time::local_date(at, &tz).unwrap();
        dated
            .water
            .discharge_percentile_for_date
            .as_mut()
            .unwrap()
            .day_of_year = format!("{:02}-{:02}", d.month, d.day);
        let flow = CatchConditions::from_snapshot(&dated, at).flow.unwrap();
        assert!(flow.value.percentile.is_some());

        // Inside the sample's major period, in daylight.
        let major = CatchConditions::from_snapshot(&snap, UnixTime(1_794_000_000 + 400 * 60));
        assert_eq!(major.solunar.map(|p| p.kind), Some(SolunarKind::Major));
        assert_eq!(major.light, Some(Light::Day));

        let far = CatchConditions::from_snapshot(&snap, UnixTime(noon + 400 * 86_400));
        assert!(far.tide.is_none() && far.flow.is_none() && far.wind.is_none());
        assert!(far.water_temperature_c.is_none() && far.pressure_hpa.is_none());
        assert!(far.light.is_none() && far.moon.is_none());
    }

    #[test]
    fn conditions_json_leaves_out_what_is_unknown() {
        let c = CatchConditions {
            at: UnixTime(5),
            ..CatchConditions::default()
        };
        assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"at":5}"#);
        assert!(c.is_empty());
    }
}
