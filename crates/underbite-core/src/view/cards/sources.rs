//! The sources behind every value, with their credits.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, CardState, Compact, CreditVm, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SourcesCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: SourcesFull,
}

/// A station that supplied a value.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct StationUseVm {
    /// What it supplies: "Water temperature".
    pub measurement: String,
    pub station: String,
    pub station_id: String,
    pub source: String,
    /// "0.4 mi"
    pub distance: Option<String>,
    /// "6 min ago"
    pub age: String,
    /// "Same water body", "Upstream"
    pub relation: String,
    /// "Observed", "Predicted", "Modelled"
    pub kind: String,
    pub state: CardState,
}

/// A source that did not deliver.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SourceErrorVm {
    pub source: String,
    pub measurement: Option<String>,
    /// "Rate limited", "Timed out"
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SourcesFull {
    pub credits: Vec<CreditVm>,
    pub stations: Vec<StationUseVm>,
    pub errors: Vec<SourceErrorVm>,
}

pub fn build(_input: &ViewInput) -> Option<SourcesCard> {
    None
}
