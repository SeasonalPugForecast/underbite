//! The marine zone forecast text.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, Compact, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MarineForecastCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: MarineForecastFull,
}

/// One period of the forecast, text exactly as issued (never truncated).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ForecastPeriodVm {
    pub name: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MarineForecastFull {
    pub zone_id: String,
    pub zone_name: String,
    /// "Issued 4:09 AM"
    pub issued: String,
    /// "Coastal waters forecast"
    pub product: String,
    /// The first period, for the compact form.
    pub headline: Option<String>,
    pub periods: Vec<ForecastPeriodVm>,
}

pub fn build(_input: &ViewInput) -> Option<MarineForecastCard> {
    None
}
