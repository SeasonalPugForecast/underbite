//! The current conditions at a glance.

use schemars::JsonSchema;
use serde::Serialize;

use crate::view::{CardChrome, CardId, CardState, Compact, Quantity, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct NowCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: NowFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct NowFull {
    pub items: Vec<NowItem>,
}

/// The headline of another card.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct NowItem {
    pub card: CardId,
    pub label: String,
    pub headline: Quantity,
    pub line: Option<String>,
    pub state: CardState,
}

pub fn build(_input: &ViewInput) -> Option<NowCard> {
    None
}
