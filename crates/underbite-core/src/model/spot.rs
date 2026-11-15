//! The place a snapshot is about: coordinates, name, water type and water body.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::provenance::SourceId;

/// What kind of water a spot is on. It picks the default cards and source chains.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WaterType {
    RiverStream,
    LakeReservoir,
    TidalRiverEstuary,
    BayInshore,
    Surf,
    Nearshore,
    Offshore,
    GreatLakes,
    Pond,
    Ice,
}

/// A named water body in a source's own identifier space (for example a 3DHP feature).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WaterBodyRef {
    pub source: SourceId,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A saved place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Spot {
    pub id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub water_type: WaterType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_body: Option<WaterBodyRef>,
    /// IANA time-zone name, for example `America/New_York`.
    pub time_zone: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_type_names_are_snake_case() {
        assert_eq!(
            serde_json::to_string(&WaterType::TidalRiverEstuary).unwrap(),
            "\"tidal_river_estuary\""
        );
    }

    #[test]
    fn spot_without_water_body_is_compact() {
        let spot = Spot {
            id: "battery".into(),
            name: "The Battery".into(),
            latitude: 40.7003,
            longitude: -74.0142,
            water_type: WaterType::TidalRiverEstuary,
            water_body: None,
            time_zone: "America/New_York".into(),
        };
        let json = serde_json::to_value(&spot).unwrap();
        assert!(json.get("water_body").is_none());
        assert_eq!(serde_json::from_value::<Spot>(json).unwrap(), spot);
    }
}
