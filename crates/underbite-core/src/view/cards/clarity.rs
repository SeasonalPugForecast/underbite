//! Turbidity, chlorophyll and clarity.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ClarityCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: ClarityFull,
}

/// The satellite diffuse attenuation coefficient; marked unavailable where there is none.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Kd490Vm {
    pub available: bool,
    pub value: Option<Quantity>,
    /// "No satellite clarity for this spot".
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ClarityFull {
    /// The unit symbol is the source's own: FNU, FTU or NTU, never converted.
    pub turbidity: Option<Quantity>,
    /// "Clear", "Slightly turbid", ...
    pub description: Option<String>,
    pub chlorophyll: Option<Quantity>,
    pub kd490: Kd490Vm,
    pub chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<ClarityCard> {
    None
}
