//! What a fetch keeps between fetches through [`FetchHost::load`](crate::FetchHost::load) and
//! [`FetchHost::store`](crate::FetchHost::store): the namespaces and keys, and the JSON shape
//! of a stored value. Neither names nor values carry coordinates or keys.

use serde::{Deserialize, Serialize};
use underbite_core::discover::cache::StationChoices;
use underbite_core::discover::rank::StationKey;
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;

/// Station lists, CO-OPS's and NDBC's whole and the others per rounded area.
pub const STATIONS: &str = "stations";
/// Rankings per spot, and the coverage observed at stations ([`COVERAGE_KEY`]).
pub const DISCOVERY: &str = "discovery";
/// The stations pinned and excluded per spot.
pub const CHOICES: &str = "choices";
/// The call ledger ([`LEDGER_KEY`]).
pub const USAGE: &str = "usage";
/// The last snapshot per spot.
pub const SPOT: &str = "spot";
/// The key of the call ledger.
pub const LEDGER_KEY: &str = "ledger";
/// The key of the observed coverage.
pub const COVERAGE_KEY: &str = "coverage";

/// How long a CO-OPS station list stays valid, seconds.
pub const COOPS_STATION_LIST_TTL_S: i64 = 7 * 86_400;
/// How long the other stations lists stay valid, seconds.
pub const STATION_LIST_TTL_S: i64 = 86_400;

/// A value with when it was stored: the JSON of every stored value, the ledger's included.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stored<T> {
    /// When it was stored, Unix seconds.
    pub saved_at: i64,
    /// The value.
    pub value: T,
}

/// The key of a spot: a hash of its id, never its coordinates.
pub fn spot_key(spot_id: &str) -> String {
    format!("spot-{:016x}", fnv1a(spot_id.as_bytes()))
}

/// FNV-1a, 64 bit: a stable name for a key (std's hashers are not stable across releases).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

// ---------------------------------------------------------------------------------------------
// Pins and exclusions
// ---------------------------------------------------------------------------------------------

/// Why a station key could not be read.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum StationKeyError {
    /// Not `source:id` or `source:id:bin`.
    #[error("{0:?} is not `source:id` or `source:id:bin`")]
    Shape(String),
    /// The bin is not a number.
    #[error("bin of {0:?} is not a number")]
    Bin(String),
}

/// Parses `source:id` or `source:id:bin`, with the id in the case its source writes it (see
/// [`canonical_station_id`]). The command line and the app read pins the same way.
pub fn parse_station_key(text: &str) -> Result<StationKey, StationKeyError> {
    let shape = || StationKeyError::Shape(text.to_owned());
    let mut parts = text.splitn(3, ':');
    let (Some(source), Some(id)) = (parts.next(), parts.next()) else {
        return Err(shape());
    };
    if source.is_empty() || id.is_empty() {
        return Err(shape());
    }
    let source = source.to_ascii_lowercase();
    let key = StationKey::new(SourceId::new(&source), canonical_station_id(&source, id));
    match parts.next() {
        Some(bin) => Ok(key.with_bin(
            bin.parse()
                .map_err(|_| StationKeyError::Bin(text.to_owned()))?,
        )),
        None => Ok(key),
    }
}

/// A station id in the case its source writes it, so `ndbc:BGNN6` and `ndbc:bgnn6` are one
/// station: NDBC ids are lower case, CO-OPS and NWPS ids upper case, and USGS ids are
/// `USGS-<number>` (a bare number is accepted).
pub fn canonical_station_id(source: &str, id: &str) -> String {
    let id = id.trim();
    match source {
        "ndbc" => id.to_ascii_lowercase(),
        "coops" | "coops-harmonics" | "nwps" => id.to_ascii_uppercase(),
        "usgs" => {
            let number = match id.split_at_checked(5) {
                Some((prefix, rest)) if prefix.eq_ignore_ascii_case("usgs-") => rest,
                _ => id,
            };
            format!("USGS-{number}")
        }
        _ => id.to_owned(),
    }
}

/// What to do with a station in a spot's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Always use it.
    Pin,
    /// Never use it.
    Exclude,
    /// Forget the pin or exclusion.
    Clear,
}

/// The stored choices of a spot (`stored` is what the host holds under [`CHOICES`] and
/// [`spot_key`], if anything) with `choice` applied to `key`, as the JSON to store. A stored
/// value that does not read back is replaced.
pub fn apply_choice(
    stored: Option<&str>,
    choice: Choice,
    key: StationKey,
    now: UnixTime,
) -> serde_json::Result<String> {
    let mut choices = stored
        .and_then(|text| serde_json::from_str::<Stored<StationChoices>>(text).ok())
        .map(|s| s.value)
        .unwrap_or_default();
    match choice {
        Choice::Pin => choices.pin(key),
        Choice::Exclude => choices.exclude(key),
        Choice::Clear => choices.clear(&key),
    }
    serde_json::to_string(&Stored {
        saved_at: now.0,
        value: choices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_keys_hide_the_id_and_stay_stable() {
        let key = spot_key("40.7033,-74.0170");
        assert_eq!(key, spot_key("40.7033,-74.0170"));
        assert_ne!(key, spot_key("41.2250,-73.8589"));
        assert!(!key.contains("40.7"), "{key}");
        // The name existing caches use.
        assert_eq!(spot_key(""), "spot-cbf29ce484222325");
    }

    #[test]
    fn keys_parse_with_and_without_a_bin() {
        assert_eq!(
            parse_station_key("coops:8518750").unwrap(),
            StationKey::new(SourceId::COOPS, "8518750")
        );
        assert_eq!(
            parse_station_key("coops:NYH1927:13").unwrap(),
            StationKey::new(SourceId::COOPS, "NYH1927").with_bin(13)
        );
        assert!(parse_station_key("coops").is_err());
        assert!(parse_station_key("coops:").is_err());
        assert!(parse_station_key("coops:x:y").is_err());
    }

    #[test]
    fn ids_take_the_case_of_their_source() {
        let key = |text: &str| parse_station_key(text).unwrap();
        assert_eq!(key("ndbc:BGNN6"), StationKey::new(SourceId::NDBC, "bgnn6"));
        assert_eq!(key("NDBC:44025"), StationKey::new(SourceId::NDBC, "44025"));
        assert_eq!(
            key("coops:nyh1927:13"),
            StationKey::new(SourceId::COOPS, "NYH1927").with_bin(13)
        );
        assert_eq!(key("nwps:ktnn6"), StationKey::new(SourceId::NWPS, "KTNN6"));
        assert_eq!(
            key("usgs:01375000"),
            StationKey::new(SourceId::USGS, "USGS-01375000")
        );
        assert_eq!(key("usgs:usgs-01375000"), key("usgs:01375000"));
        // A source this version does not know keeps its id as written.
        assert_eq!(key("other:AbC").id, "AbC");
    }

    #[test]
    fn choices_change_one_station_at_a_time() {
        let now = UnixTime(1_794_744_000);
        let read = |json: &str| {
            serde_json::from_str::<Stored<StationChoices>>(json)
                .unwrap()
                .value
        };
        let battery = parse_station_key("coops:8518750").unwrap();
        let buoy = parse_station_key("ndbc:44025").unwrap();
        let pinned = apply_choice(None, Choice::Pin, battery.clone(), now).unwrap();
        let both = apply_choice(Some(&pinned), Choice::Exclude, buoy.clone(), now).unwrap();
        let choices = read(&both);
        assert!(choices.pinned.contains(&battery) && choices.excluded.contains(&buoy));
        // Excluding lifts the pin; clearing lifts the exclusion.
        let swapped = apply_choice(Some(&both), Choice::Exclude, battery.clone(), now).unwrap();
        assert!(read(&swapped).pinned.is_empty());
        let cleared = apply_choice(Some(&swapped), Choice::Clear, battery, now).unwrap();
        assert_eq!(read(&cleared).excluded.len(), 1);
        // A value that does not read back is replaced.
        let fresh = apply_choice(Some("not json"), Choice::Pin, buoy.clone(), now).unwrap();
        assert!(read(&fresh).pinned.contains(&buoy));
    }
}
