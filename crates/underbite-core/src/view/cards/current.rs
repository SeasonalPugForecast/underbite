//! Tidal current speed, direction and slack times.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CurrentCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: CurrentFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurrentPhase {
    Flood,
    Ebb,
    Slack,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CurrentEventKind {
    MaxFlood,
    MaxEbb,
    Slack,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CurrentEventVm {
    pub t: i64,
    pub time: String,
    pub kind: CurrentEventKind,
    /// "Max flood", "Slack", ...
    pub label: String,
    /// None for slack.
    pub speed: Option<Quantity>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CurrentFull {
    pub phase: CurrentPhase,
    /// "Flooding", "Ebbing", "Slack"
    pub phase_label: String,
    pub speed_now: Option<Quantity>,
    /// Where the water is heading now, "NNE 025°".
    pub direction_now: Option<String>,
    pub next_slack: Option<CurrentEventVm>,
    pub events: Vec<CurrentEventVm>,
    pub flood_direction: Option<String>,
    pub ebb_direction: Option<String>,
    /// "Bin 3 at 12 ft": the depth the station measures at.
    pub bin_depth: Option<String>,
    /// Speed over time with slack windows highlighted.
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<CurrentCard> {
    None
}
