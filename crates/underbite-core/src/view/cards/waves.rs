//! Waves and swell.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WavesCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: WavesFull,
}

/// One wave train (wind waves or a swell).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaveTrainVm {
    /// "Wind waves", "Swell 1"
    pub label: String,
    pub height: Option<Quantity>,
    pub period: Option<Quantity>,
    /// "SE 135°", the direction the waves come from.
    pub direction: Option<String>,
}

/// Waves at one moment: the latest observation or the forecast for a time.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaveNowVm {
    pub t: i64,
    pub time: String,
    pub significant_height: Option<Quantity>,
    pub dominant_period: Option<Quantity>,
    pub mean_period: Option<Quantity>,
    pub direction: Option<String>,
    pub wind_waves: Option<WaveTrainVm>,
    pub swells: Vec<WaveTrainVm>,
    /// "Steep", when the source says.
    pub steepness: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WavesFull {
    pub observed: Option<WaveNowVm>,
    /// The forecast for now (or the next hour).
    pub forecast: Option<WaveNowVm>,
    /// Observed and forecast height on one axis.
    pub height_chart: Option<ChartVm>,
    pub period_chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<WavesCard> {
    None
}
