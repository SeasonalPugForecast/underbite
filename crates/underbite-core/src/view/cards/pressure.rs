//! Barometric pressure and its trend.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, TrendVm, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PressureCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: PressureFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PressureFull {
    pub value: Option<Quantity>,
    pub trend: Option<TrendVm>,
    /// The change over the trend window, signed.
    pub change: Option<Quantity>,
    /// Past observed and forecast pressure on one axis.
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<PressureCard> {
    None
}
