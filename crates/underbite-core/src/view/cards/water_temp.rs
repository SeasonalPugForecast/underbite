//! Water temperature and its trend.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, TrendVm, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaterTempCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: WaterTempFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TempBasis {
    /// A gauge or buoy thermometer.
    Measured,
    /// Sea surface temperature from a model or satellite.
    SeaSurface,
}

/// The trout-stress guidance against the user's threshold.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct TroutStressVm {
    pub exceeded: bool,
    pub threshold: Quantity,
    /// "Consider not fishing for trout above 68 °F."
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct WaterTempFull {
    pub value: Option<Quantity>,
    pub basis: TempBasis,
    pub trend: Option<TrendVm>,
    pub trout_stress: Option<TroutStressVm>,
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<WaterTempCard> {
    None
}
