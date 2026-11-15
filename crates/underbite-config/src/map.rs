//! Map defaults.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `[map]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MapConfig {
    /// The layer preset shown first, by name.
    pub preset: String,
    /// Most storage offline packs may use, in megabytes.
    pub offline_max_mb: u32,
}

impl Default for MapConfig {
    fn default() -> Self {
        Self {
            preset: "Inshore flats".to_owned(),
            offline_max_mb: 1500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_missing_keys() {
        let m: MapConfig = toml::from_str("offline_max_mb = 500").unwrap();
        assert_eq!(
            (m.preset.as_str(), m.offline_max_mb),
            ("Inshore flats", 500)
        );
    }
}
