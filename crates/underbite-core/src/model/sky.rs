//! Sun, moon and solunar periods per local day.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::time::{LocalDate, UnixTime};

/// Why a day has no sunrise or sunset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Polar {
    /// The sun stays up all day (midnight sun).
    AlwaysUp,
    /// The sun stays down all day (polar night).
    AlwaysDown,
}

/// Sun times on one local day. Each is absent when the sun never crosses that altitude.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SunTimes {
    /// Set on days with no sunrise or sunset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polar: Option<Polar>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sunrise: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sunset: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civil_dawn: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civil_dusk: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nautical_dawn: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nautical_dusk: Option<UnixTime>,
    /// End of the morning golden hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden_morning_end: Option<UnixTime>,
    /// Start of the evening golden hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden_evening_start: Option<UnixTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MoonPhase {
    New,
    WaxingCrescent,
    FirstQuarter,
    WaxingGibbous,
    Full,
    WaningGibbous,
    LastQuarter,
    WaningCrescent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalPhase {
    New,
    FirstQuarter,
    Full,
    LastQuarter,
}

/// A principal phase and the instant it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PrincipalPhaseTime {
    pub phase: PrincipalPhase,
    pub time: UnixTime,
}

/// The moon on one local day.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MoonDay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rise: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_transit: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lower_transit: Option<UnixTime>,
    /// The moon's apparent longitude minus the sun's, 0 to 360 degrees: 0 new, 90 first quarter,
    /// 180 full, 270 last quarter, so it also tells waxing from waning.
    pub elongation_deg: f32,
    /// Illuminated fraction, 0 to 1.
    pub illuminated_fraction: f32,
    pub phase: MoonPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nearest_principal: Option<PrincipalPhaseTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SolunarKind {
    Major,
    Minor,
}

/// What a solunar period is centred on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SolunarCause {
    UpperTransit,
    LowerTransit,
    Rise,
    Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SolunarPeriod {
    pub kind: SolunarKind,
    pub start: UnixTime,
    pub end: UnixTime,
    pub cause: SolunarCause,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SkyDay {
    pub date: LocalDate,
    pub sun: SunTimes,
    pub moon: MoonDay,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solunar: Vec<SolunarPeriod>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SkyConditions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<SkyDay>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sky_serializes_to_an_empty_object() {
        assert_eq!(
            serde_json::to_string(&SkyConditions::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn polar_day_round_trips() {
        let t = SunTimes {
            polar: Some(Polar::AlwaysDown),
            ..SunTimes::default()
        };
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "{\"polar\":\"always_down\"}");
        assert_eq!(serde_json::from_str::<SunTimes>(&json).unwrap(), t);
    }
}
