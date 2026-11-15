//! Tide heights, highs and lows, and the stage now.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::cards::sun_moon::MoonPhaseVm;
use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, TimedLabel, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: TideFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TideStage {
    Rising,
    Falling,
    HighSlack,
    LowSlack,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TideEventKind {
    High,
    Low,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideEventVm {
    /// Unix seconds.
    pub t: i64,
    pub time: String,
    pub kind: TideEventKind,
    /// "High" or "Low".
    pub label: String,
    /// None when the source gives times only (modelled sea level).
    pub height: Option<Quantity>,
}

/// One local day of the tide table.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideDayVm {
    /// "Sat Nov 14"
    pub date: String,
    pub events: Vec<TideEventVm>,
    pub sunrise: Option<TimedLabel>,
    pub sunset: Option<TimedLabel>,
    pub moon: Option<MoonPhaseVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TideFull {
    pub stage: TideStage,
    /// "Rising", "Falling", ...
    pub stage_label: String,
    /// The height now, interpolated; None without heights.
    pub height_now: Option<Quantity>,
    /// The next high and the next low, soonest first.
    pub next: Vec<TideEventVm>,
    /// "above MLLW"; None when heights are not shown.
    pub datum_label: Option<String>,
    /// False for modelled sea level: relative curve, no heights.
    pub heights_shown: bool,
    /// Prediction with the observed level over it, sunrise and sunset shading, moon phase.
    pub chart: Option<ChartVm>,
    pub days: Vec<TideDayVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<TideCard> {
    None
}
