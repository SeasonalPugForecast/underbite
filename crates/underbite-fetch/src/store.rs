//! What a fetch keeps between fetches through [`FetchHost::load`](crate::FetchHost::load) and
//! [`FetchHost::store`](crate::FetchHost::store): the namespaces and keys, and the JSON shape
//! of a stored value. Neither names nor values carry coordinates or keys.

use serde::{Deserialize, Serialize};

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
}
