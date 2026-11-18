//! Alert rule types: targets, conditions in canonical units, repeat and quiet hours.
//!
//! A rule names its spot or trip by id, never by coordinates; a box target is the one place a
//! rule holds coordinates, and the configuration export rounds them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}

/// One `[[alerts]]` rule: tell me when every condition holds at the target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertRule {
    /// Stable id, unique among the rules; evaluation state is kept under it.
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub target: AlertTarget,
    /// The rule fires when all of these hold on fresh data.
    pub all_of: Vec<Condition>,
    /// Fire again after this many hours while the rule stays true; without it, only after the
    /// conditions have stopped holding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_after_hours: Option<u32>,
    /// Hours in the target's time zone when a notification waits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<QuietHours>,
    /// Whether notifications name the place: the rule's own name, the spot or trip and the
    /// station. Without it they say "your spot" or "your trip". They never carry coordinates.
    #[serde(default = "yes")]
    pub show_name: bool,
}

impl AlertRule {
    /// Whether the spot or trip the rule watches no longer exists. Such a rule still decodes
    /// and is shown as orphaned; a box target is never orphaned.
    pub fn is_orphaned(
        &self,
        spot_exists: impl Fn(&str) -> bool,
        trip_exists: impl Fn(&str) -> bool,
    ) -> bool {
        match &self.target {
            AlertTarget::Spot { id } => !spot_exists(id),
            AlertTarget::Trip { id } => !trip_exists(id),
            AlertTarget::Box { .. } => false,
        }
    }
}

/// What a rule watches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AlertTarget {
    /// A saved spot, by its id.
    Spot { id: String },
    /// A saved trip, by its id.
    Trip { id: String },
    /// An area of gridded data, in degrees (WGS 84), at most 1° by 1°; only sea surface
    /// temperature and chlorophyll conditions apply.
    Box {
        south: f64,
        west: f64,
        north: f64,
        east: f64,
    },
}

// The conditions without fields are empty struct variants: serde ignores unknown fields on an
// internally tagged unit variant, and refuses them on a struct variant.
/// One condition of a rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    /// A value below `below`, above `above`, or between the two when both are given, in the
    /// metric's canonical unit.
    Measurement {
        metric: AlertMetric,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
    },
    /// A marine alert in force whose NWS event name starts with `event`, such as
    /// `Small Craft Advisory` or `Gale Warning`.
    MarineAlert { event: String },
    /// Go/no-go turns Go for a day of the trip (trip targets only).
    TripGo {},
    /// A fishing window starts within `within_minutes`.
    WindowStarts { within_minutes: u32 },
    /// A storm ahead, or a hurricane's forecast cone over the target (NHC).
    StormAhead {},
}

/// A value a measurement condition compares, with its canonical unit.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AlertMetric {
    /// River discharge, m³/s.
    Discharge,
    /// Gage height (stage), metres.
    GageHeight,
    /// Water temperature at a station, °C.
    WaterTemperature,
    /// Sea surface temperature, °C.
    SeaSurfaceTemp,
    /// Significant wave height, metres.
    WaveHeight,
    /// Sustained wind speed, m/s.
    Wind,
    /// Wind gusts, m/s.
    Gusts,
    /// Pressure change over the last 3 hours, hPa (negative when falling).
    PressureTrend,
    /// Tide height above the station's chart datum, metres.
    TideHeight,
    /// Chlorophyll a, µg/L (mg/m³).
    Chlorophyll,
}

impl AlertMetric {
    /// Every metric, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::Discharge,
        Self::GageHeight,
        Self::WaterTemperature,
        Self::SeaSurfaceTemp,
        Self::WaveHeight,
        Self::Wind,
        Self::Gusts,
        Self::PressureTrend,
        Self::TideHeight,
        Self::Chlorophyll,
    ];

    /// Whether a box target can watch it: only the gridded values a box rule reads.
    pub const fn in_boxes(self) -> bool {
        matches!(self, Self::SeaSurfaceTemp | Self::Chlorophyll)
    }
}

/// A daily span, in minutes after midnight in the target's time zone. It crosses midnight when
/// `start_minute` is after `end_minute`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuietHours {
    pub start_minute: u16,
    pub end_minute: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(target: AlertTarget, all_of: Vec<Condition>) -> AlertRule {
        AlertRule {
            id: "r1".to_owned(),
            name: "Croton low and cool".to_owned(),
            enabled: true,
            target,
            all_of,
            repeat_after_hours: None,
            quiet_hours: None,
            show_name: true,
        }
    }

    #[test]
    fn every_kind_round_trips_through_json() {
        let r = AlertRule {
            repeat_after_hours: Some(12),
            show_name: false,
            quiet_hours: Some(QuietHours {
                start_minute: 22 * 60,
                end_minute: 7 * 60,
            }),
            ..rule(
                AlertTarget::Trip {
                    id: "trip-1".to_owned(),
                },
                vec![
                    Condition::Measurement {
                        metric: AlertMetric::Discharge,
                        below: Some(22.65),
                        above: None,
                    },
                    Condition::Measurement {
                        metric: AlertMetric::WaterTemperature,
                        below: Some(18.9),
                        above: Some(10.0),
                    },
                    Condition::MarineAlert {
                        event: "Small Craft Advisory".to_owned(),
                    },
                    Condition::TripGo {},
                    Condition::WindowStarts { within_minutes: 30 },
                    Condition::StormAhead {},
                ],
            )
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["target"], json!({ "kind": "trip", "id": "trip-1" }));
        assert_eq!(
            v["all_of"][0],
            json!({ "kind": "measurement", "metric": "discharge", "below": 22.65 })
        );
        assert_eq!(v["all_of"][3], json!({ "kind": "trip_go" }));
        assert_eq!(
            v["all_of"][4],
            json!({ "kind": "window_starts", "within_minutes": 30 })
        );
        assert_eq!(v["quiet_hours"]["start_minute"], 1320);
        assert_eq!(v["show_name"], false);
        let back: AlertRule = serde_json::from_value(v).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn enabled_and_show_name_default_to_true_and_options_to_none() {
        let r: AlertRule = serde_json::from_value(json!({
            "id": "r", "name": "Gale", "target": { "kind": "spot", "id": "battery" },
            "all_of": [{ "kind": "marine_alert", "event": "Gale Warning" }]
        }))
        .unwrap();
        assert!(r.enabled && r.show_name);
        assert_eq!((r.repeat_after_hours, r.quiet_hours), (None, None));
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("repeat_after_hours").is_none() && v.get("quiet_hours").is_none());
        // Always written, so a file says what its notifications show.
        assert_eq!(v["show_name"], true);
    }

    #[test]
    fn unknown_fields_and_kinds_are_refused() {
        for v in [
            json!({ "kind": "spot", "id": "a", "lat": 40.7 }),
            json!({ "kind": "box", "south": 40, "west": -74, "north": 40.5, "east": -73.5, "zoom": 1 }),
            json!({ "kind": "circle", "radius": 1 }),
        ] {
            assert!(
                serde_json::from_value::<AlertTarget>(v.clone()).is_err(),
                "{v}"
            );
        }
        for v in [
            json!({ "kind": "measurement", "metric": "discharge", "below": 1, "unit": "cfs" }),
            json!({ "kind": "measurement", "metric": "rainfall", "below": 1 }),
            json!({ "kind": "storm_ahead", "radius_km": 10 }),
        ] {
            assert!(
                serde_json::from_value::<Condition>(v.clone()).is_err(),
                "{v}"
            );
        }
    }

    #[test]
    fn only_spot_and_trip_targets_can_be_orphaned() {
        let spots = ["battery"];
        let trips: [&str; 0] = [];
        let orphaned = |target| {
            rule(target, vec![Condition::StormAhead {}])
                .is_orphaned(|id| spots.contains(&id), |id| trips.contains(&id))
        };
        assert!(!orphaned(AlertTarget::Spot {
            id: "battery".to_owned()
        }));
        assert!(orphaned(AlertTarget::Spot {
            id: "gone".to_owned()
        }));
        assert!(orphaned(AlertTarget::Trip {
            id: "gone".to_owned()
        }));
        assert!(!orphaned(AlertTarget::Box {
            south: 40.0,
            west: -73.5,
            north: 40.5,
            east: -73.0
        }));
    }

    #[test]
    fn boxes_watch_only_gridded_values() {
        let gridded: Vec<_> = AlertMetric::ALL
            .into_iter()
            .filter(|m| m.in_boxes())
            .collect();
        assert_eq!(
            gridded,
            [AlertMetric::SeaSurfaceTemp, AlertMetric::Chlorophyll]
        );
    }
}
