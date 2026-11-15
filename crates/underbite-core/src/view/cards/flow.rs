//! River flow and gage height against normal and the user's ranges.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::PercentileClass;
use crate::view::{CardChrome, ChartVm, Compact, Quantity, Section, TrendVm, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct FlowCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: FlowFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RangeStatus {
    Wadeable,
    Best,
    Fishable,
    BlownOut,
    Unknown,
}

/// Where the discharge sits against the user's own ranges.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct RangeStatusVm {
    pub status: RangeStatus,
    /// "Best range (400 to 900 cfs)"
    pub text: String,
}

/// "Higher than 80% of Nov 14 days since 1990".
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PercentileVm {
    pub text: String,
    pub class: Option<PercentileClass>,
    /// "Above normal"
    pub class_label: String,
    /// "Statistics API" or "Computed on this device from daily values"
    pub basis: String,
    /// "approved data only, 36 years"
    pub sample: String,
    /// Labelled thresholds, "10th percentile".
    pub thresholds: Vec<PercentileThresholdVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PercentileThresholdVm {
    pub label: String,
    pub value: Quantity,
}

/// A flood category: "Action", "Minor", "Moderate", "Major".
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct FloodCategoryVm {
    pub label: String,
    pub stage: Quantity,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct FlowFull {
    pub discharge: Option<Quantity>,
    pub trend: Option<TrendVm>,
    pub gage_height: Option<Quantity>,
    pub percentile: Option<PercentileVm>,
    pub range_status: Option<RangeStatusVm>,
    pub flood_categories: Vec<FloodCategoryVm>,
    /// Seven days of discharge, the NWPS forecast after now, user ranges and flood stages as rules.
    pub chart: Option<ChartVm>,
    /// A separate stage chart when the forecast is a stage hydrograph.
    pub stage_chart: Option<ChartVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<FlowCard> {
    None
}
