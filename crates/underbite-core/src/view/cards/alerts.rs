//! Active marine alerts.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::AlertSeverity;
use crate::view::{CardChrome, Compact, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertsCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: AlertsFull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertOrigin {
    Marine,
    Weather,
}

/// One alert, safety text exactly as issued and never in Luna's voice.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertVm {
    pub id: String,
    pub origin: AlertOrigin,
    pub event: String,
    pub headline: Option<String>,
    pub severity: AlertSeverity,
    pub severity_label: String,
    pub onset: Option<String>,
    pub ends: Option<String>,
    pub expires: Option<String>,
    /// Zone names or ids the alert covers.
    pub zones: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct AlertsFull {
    /// Marine and weather alerts de-duplicated, most severe first.
    pub alerts: Vec<AlertVm>,
}

pub fn build(_input: &ViewInput) -> Option<AlertsCard> {
    None
}
