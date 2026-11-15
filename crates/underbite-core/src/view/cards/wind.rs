//! Wind and gusts.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: WindFull,
}

/// Where the wind figure comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindBasis {
    /// A station or buoy.
    Observed,
    /// The forecast model.
    Forecast,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindHourVm {
    pub t: i64,
    pub time: String,
    pub speed: Quantity,
    pub gust: Option<Quantity>,
    /// "NW"
    pub direction: Option<String>,
    /// Degrees the wind blows from, for drawing arrows.
    pub from_deg: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WindFull {
    pub basis: WindBasis,
    pub speed: Option<Quantity>,
    pub gust: Option<Quantity>,
    pub direction: Option<String>,
    pub from_deg: Option<f32>,
    pub hours: Vec<WindHourVm>,
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<WindCard> {
    None
}
