//! Solunar periods.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{SolunarCause, SolunarKind};
use crate::view::{CardChrome, Compact, Section, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: SolunarFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarPeriodVm {
    pub kind: SolunarKind,
    pub cause: SolunarCause,
    /// "Major" or "Minor" (the playful label is the voice's, set by the formatter).
    pub label: String,
    pub start_t: i64,
    pub end_t: i64,
    /// "6:10 to 8:10 AM"
    pub range: String,
    /// "Moon overhead", "Moonrise"
    pub cause_label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarDayVm {
    pub date: String,
    pub periods: Vec<SolunarPeriodVm>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SolunarFull {
    pub days: Vec<SolunarDayVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<SolunarCard> {
    None
}
