//! Sun and moon times and the moon's phase.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::MoonPhase;
use crate::view::{CardChrome, Compact, Quantity, Section, TimedLabel, ViewInput};

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SunMoonCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: SunMoonFull,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MoonPhaseVm {
    pub phase: MoonPhase,
    /// "Waxing gibbous"
    pub label: String,
    /// The lit fraction as a percentage.
    pub illumination: Quantity,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MoonVm {
    pub phase: MoonPhaseVm,
    pub rise: Option<TimedLabel>,
    pub set: Option<TimedLabel>,
    pub upper_transit: Option<TimedLabel>,
    pub lower_transit: Option<TimedLabel>,
    /// The nearest new, quarter or full moon.
    pub nearest_principal: Option<TimedLabel>,
}

/// One local day.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SunMoonDayVm {
    /// "Sat Nov 14"
    pub date: String,
    /// Dawn, sunrise, golden hour, sunset, dusk in time order, only those that occur.
    pub sun: Vec<TimedLabel>,
    /// "10 h 12 min"
    pub day_length: Option<String>,
    /// "Sun stays up all day" or "Sun stays down all day".
    pub polar: Option<String>,
    pub moon: MoonVm,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct SunMoonFull {
    /// Today first, then the days the snapshot has.
    pub days: Vec<SunMoonDayVm>,
    pub sections: Vec<Section>,
}

pub fn build(_input: &ViewInput) -> Option<SunMoonCard> {
    None
}
