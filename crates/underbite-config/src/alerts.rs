//! The `[[alerts]]` rules in the configuration file.
//!
//! The rule types are the core's. Here they are checked, their unknown kinds dropped on import,
//! and their box targets rounded for export: a spot or trip target is an id, so a box is the
//! only place a rule holds coordinates.

use serde_json::Value;

pub use underbite_core::alerts::{AlertMetric, AlertRule, AlertTarget, Condition, QuietHours};

use crate::schema::Issues;

/// The largest side of a box target, in degrees of latitude and of longitude.
pub const MAX_BOX_DEGREES: f64 = 1.0;

/// Slack for boxes whose rounded sides come out a hair over [`MAX_BOX_DEGREES`].
const BOX_EPSILON: f64 = 1e-9;

const MINUTES_PER_DAY: u16 = 24 * 60;

/// The target kinds this version reads.
pub const TARGET_KINDS: [&str; 3] = ["spot", "trip", "box"];

/// The condition kinds this version reads.
pub const CONDITION_KINDS: [&str; 5] = [
    "measurement",
    "marine_alert",
    "trip_go",
    "window_starts",
    "storm_ahead",
];

/// Every semantic problem in the rules. Spot and trip ids are not looked up: a rule whose spot
/// was deleted still loads, and the app shows it as orphaned.
pub(crate) fn validate(rules: &[AlertRule], issues: &mut Issues) {
    for (i, rule) in rules.iter().enumerate() {
        let path = format!("alerts[{i}]");
        if rule.id.trim().is_empty() {
            issues.error(format!("{path}.id"), "a rule needs an id");
        } else if rules[..i].iter().any(|r| r.id == rule.id) {
            issues.error(format!("{path}.id"), "another rule has this id");
        }
        if rule.name.trim().is_empty() {
            issues.error(format!("{path}.name"), "a rule needs a name");
        }
        validate_target(&rule.target, &format!("{path}.target"), issues);
        if rule.all_of.is_empty() {
            issues.error(
                format!("{path}.all_of"),
                "a rule needs at least one condition",
            );
        }
        for (j, condition) in rule.all_of.iter().enumerate() {
            validate_condition(
                &rule.target,
                condition,
                &format!("{path}.all_of[{j}]"),
                issues,
            );
        }
        if rule.repeat_after_hours == Some(0) {
            issues.error(
                format!("{path}.repeat_after_hours"),
                "must be greater than 0; leave it out to notify once until the rule stops holding",
            );
        }
        if let Some(q) = rule.quiet_hours {
            validate_quiet_hours(q, &format!("{path}.quiet_hours"), issues);
        }
    }
}

fn validate_target(target: &AlertTarget, path: &str, issues: &mut Issues) {
    match target {
        AlertTarget::Spot { id } | AlertTarget::Trip { id } => {
            if id.trim().is_empty() {
                issues.error(format!("{path}.id"), "names no spot or trip");
            }
        }
        &AlertTarget::Box {
            south,
            west,
            north,
            east,
        } => {
            let lat = |v: f64| v.is_finite() && (-90.0..=90.0).contains(&v);
            let lon = |v: f64| v.is_finite() && (-180.0..=180.0).contains(&v);
            if !(lat(south) && lat(north) && south < north) {
                issues.error(
                    format!("{path}.north"),
                    "expected latitudes from -90 to 90 with `south` below `north`",
                );
            } else if north - south > MAX_BOX_DEGREES + BOX_EPSILON {
                issues.error(
                    format!("{path}.north"),
                    "a box spans at most 1° of latitude",
                );
            }
            if !(lon(west) && lon(east) && west < east) {
                issues.error(
                    format!("{path}.east"),
                    "expected longitudes from -180 to 180 with `west` below `east` (a box cannot cross the 180th meridian)",
                );
            } else if east - west > MAX_BOX_DEGREES + BOX_EPSILON {
                issues.error(
                    format!("{path}.east"),
                    "a box spans at most 1° of longitude",
                );
            }
        }
    }
}

fn validate_condition(
    target: &AlertTarget,
    condition: &Condition,
    path: &str,
    issues: &mut Issues,
) {
    let in_box = matches!(target, AlertTarget::Box { .. });
    match condition {
        Condition::Measurement {
            metric,
            below,
            above,
        } => {
            if in_box && !metric.in_boxes() {
                issues.error(
                    format!("{path}.metric"),
                    "a box rule watches only sea surface temperature or chlorophyll",
                );
            }
            match (below, above) {
                (None, None) => issues.error(path, "give `below`, `above` or both"),
                (Some(b), Some(a)) if a.is_finite() && b.is_finite() && a >= b => issues.error(
                    format!("{path}.above"),
                    "the range is empty: `above` must be less than `below`",
                ),
                _ => {}
            }
            for (key, v) in [("below", below), ("above", above)] {
                if v.is_some_and(|v| !v.is_finite()) {
                    issues.error(format!("{path}.{key}"), "must be a number");
                }
            }
        }
        Condition::MarineAlert { event } => {
            if in_box {
                issues.error(
                    path,
                    "a box rule watches only sea surface temperature or chlorophyll",
                );
            }
            if event.trim().is_empty() {
                issues.error(
                    format!("{path}.event"),
                    "name the alert, such as `Small Craft Advisory`",
                );
            }
        }
        Condition::TripGo {} => {
            if !matches!(target, AlertTarget::Trip { .. }) {
                issues.error(path, "go/no-go applies only to a trip");
            }
        }
        Condition::WindowStarts { .. } | Condition::StormAhead {} => {
            if in_box {
                issues.error(
                    path,
                    "a box rule watches only sea surface temperature or chlorophyll",
                );
            }
        }
    }
}

fn validate_quiet_hours(q: QuietHours, path: &str, issues: &mut Issues) {
    for (key, v) in [
        ("start_minute", q.start_minute),
        ("end_minute", q.end_minute),
    ] {
        if v >= MINUTES_PER_DAY {
            issues.error(
                format!("{path}.{key}"),
                "minutes after midnight, from 0 to 1439",
            );
        }
    }
    if q.start_minute == q.end_minute {
        issues.warning(
            path,
            "quiet hours start and end at the same minute, so they never apply",
        );
    }
}

/// Rounds every box target to `decimals` places, as coordinates sent to providers are. A box
/// that would shrink to nothing keeps one step of the grid from its rounded-down south or west
/// side; a side of at most 1° stays at most 1°.
pub(crate) fn round_boxes(rules: &mut [AlertRule], decimals: u8) {
    let scale = 10f64.powi(i32::from(decimals));
    let step = 1.0 / scale;
    let round = |v: f64| (v * scale).round() / scale;
    let floor = |v: f64| (v * scale).floor() / scale;
    // The two sides, rounded; a collapsed span starts from the low side rounded down.
    let pair = |lo: f64, hi: f64| {
        let (rlo, rhi) = (round(lo), round(hi));
        if lo < hi && rhi <= rlo {
            let start = floor(lo);
            (start, start + step)
        } else {
            (rlo, rhi)
        }
    };
    for rule in rules {
        if let AlertTarget::Box {
            south,
            west,
            north,
            east,
        } = &mut rule.target
        {
            (*south, *north) = pair(*south, *north);
            (*west, *east) = pair(*west, *east);
        }
    }
}

/// Drops rules whose target kind, condition kind or metric this version does not know, with a
/// warning for each: such a rule cannot be judged, and leaving out one of its conditions would
/// change what it means. Values of the wrong type are left for the typed decode to report.
pub(crate) fn tidy(doc: &mut Value, issues: &mut Issues) {
    let Some(rules) = doc.get_mut("alerts").and_then(Value::as_array_mut) else {
        return;
    };
    let mut kept = Vec::with_capacity(rules.len());
    for (i, rule) in rules.drain(..).enumerate() {
        match unknown_part(&rule) {
            Some((at, what)) => issues.warning(
                format!("alerts[{i}]{at}"),
                format!("{what}; the rule was dropped"),
            ),
            None => kept.push(rule),
        }
    }
    *rules = kept;
}

/// Where in a rule an unknown kind or metric sits, and what it is.
fn unknown_part(rule: &Value) -> Option<(String, String)> {
    let kind = |v: &Value| v.get("kind").and_then(Value::as_str).map(str::to_owned);
    if let Some(k) = rule.get("target").and_then(kind)
        && !TARGET_KINDS.contains(&k.as_str())
    {
        return Some((
            ".target.kind".to_owned(),
            format!("`{k}` is not a known target"),
        ));
    }
    let conditions = rule.get("all_of").and_then(Value::as_array)?;
    conditions.iter().enumerate().find_map(|(j, c)| {
        if let Some(k) = kind(c)
            && !CONDITION_KINDS.contains(&k.as_str())
        {
            return Some((
                format!(".all_of[{j}].kind"),
                format!("`{k}` is not a known condition"),
            ));
        }
        let metric = c.get("metric").and_then(Value::as_str)?;
        serde_json::from_value::<AlertMetric>(Value::from(metric))
            .is_err()
            .then(|| {
                (
                    format!(".all_of[{j}].metric"),
                    format!("`{metric}` is not a known measurement"),
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Config, Severity};
    use serde_json::json;

    fn spot_rule(id: &str, all_of: Vec<Condition>) -> AlertRule {
        AlertRule {
            id: id.to_owned(),
            name: "Croton low and cool".to_owned(),
            enabled: true,
            target: AlertTarget::Spot {
                id: "croton".to_owned(),
            },
            all_of,
            repeat_after_hours: None,
            quiet_hours: None,
        }
    }

    fn sst_box(south: f64, west: f64, north: f64, east: f64) -> AlertRule {
        AlertRule {
            target: AlertTarget::Box {
                south,
                west,
                north,
                east,
            },
            ..spot_rule(
                &format!("box {south} {west} {north} {east}"),
                vec![Condition::Measurement {
                    metric: AlertMetric::SeaSurfaceTemp,
                    below: None,
                    above: Some(20.0),
                }],
            )
        }
    }

    fn paths(rules: &[AlertRule]) -> Vec<(Severity, String)> {
        let mut issues = Issues::default();
        validate(rules, &mut issues);
        issues.0.into_iter().map(|i| (i.severity, i.path)).collect()
    }

    fn errors(rules: &[AlertRule]) -> Vec<String> {
        paths(rules)
            .into_iter()
            .filter(|(s, _)| *s == Severity::Error)
            .map(|(_, p)| p)
            .collect()
    }

    const TOML: &str = r#"
[[alerts]]
id = "croton-low"
name = "Croton low and cool"
target = { kind = "spot", id = "croton" }
all_of = [
    { kind = "measurement", metric = "discharge", below = 22.65 },
    { kind = "measurement", metric = "water_temperature", below = 18.9 },
]
quiet_hours = { start_minute = 1320, end_minute = 420 }

[[alerts]]
id = "sca"
name = "Small craft advisory on the trip"
enabled = false
target = { kind = "trip", id = "trip-1" }
all_of = [{ kind = "marine_alert", event = "Small Craft Advisory" }, { kind = "trip_go" }]
repeat_after_hours = 12

[[alerts]]
id = "canyon"
name = "Warm water in the box"
target = { kind = "box", south = 39.5, west = -72.5, north = 40.5, east = -71.5 }
all_of = [{ kind = "measurement", metric = "sea_surface_temp", above = 20.0 }]
"#;

    #[test]
    fn alerts_parse_from_toml_and_validate_clean() {
        let c = Config::from_toml(TOML).unwrap();
        assert_eq!(c.alerts.len(), 3);
        assert_eq!(c.validate(), []);
        assert!(c.alerts[0].enabled);
        assert!(!c.alerts[1].enabled);
        assert_eq!(c.alerts[1].repeat_after_hours, Some(12));
        assert_eq!(
            c.alerts[0].quiet_hours,
            Some(QuietHours {
                start_minute: 1320,
                end_minute: 420
            })
        );
        assert_eq!(
            c.alerts[0].all_of[0],
            Condition::Measurement {
                metric: AlertMetric::Discharge,
                below: Some(22.65),
                above: None
            }
        );
    }

    #[test]
    fn alerts_round_trip_through_toml_and_json() {
        let c = Config::from_toml(TOML).unwrap();
        let text = c.to_toml().unwrap();
        assert!(text.contains("[[alerts]]"), "{text}");
        assert_eq!(Config::from_toml(&text).unwrap(), c);
        let json: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(json, c);
    }

    #[test]
    fn alerts_default_to_none() {
        assert_eq!(Config::default().alerts, []);
        assert_eq!(
            Config::from_toml("[units]\nspeed = \"kn\"").unwrap().alerts,
            []
        );
    }

    #[test]
    fn alerts_ids_names_and_conditions_are_required() {
        let mut nameless = spot_rule(" ", vec![]);
        nameless.name = String::new();
        let twin = spot_rule("a", vec![Condition::StormAhead {}]);
        assert_eq!(
            errors(&[nameless, twin.clone(), twin]),
            [
                "alerts[0].id",
                "alerts[0].name",
                "alerts[0].all_of",
                "alerts[2].id"
            ]
        );
        let mut idless = spot_rule("a", vec![Condition::StormAhead {}]);
        idless.target = AlertTarget::Trip { id: String::new() };
        assert_eq!(errors(&[idless]), ["alerts[0].target.id"]);
    }

    #[test]
    fn alerts_measurements_need_a_bound_and_a_non_empty_range() {
        let m = |below, above| Condition::Measurement {
            metric: AlertMetric::WaveHeight,
            below,
            above,
        };
        assert_eq!(
            errors(&[spot_rule(
                "a",
                vec![
                    m(None, None),
                    m(Some(1.0), Some(2.0)),
                    m(Some(f64::NAN), None),
                    m(Some(2.0), Some(1.0)),
                ],
            )]),
            [
                "alerts[0].all_of[0]",
                "alerts[0].all_of[1].above",
                "alerts[0].all_of[2].below"
            ]
        );
    }

    #[test]
    fn alerts_boxes_are_small_and_watch_only_gridded_values() {
        assert_eq!(
            errors(&[sst_box(40.0, -74.0, 41.0, -73.0)]),
            Vec::<String>::new()
        );
        assert_eq!(
            errors(&[
                sst_box(40.0, -74.0, 41.5, -73.0),
                sst_box(40.0, -74.0, 40.5, -72.5),
                sst_box(41.0, 179.5, 40.0, -179.5),
            ]),
            [
                "alerts[0].target.north",
                "alerts[1].target.east",
                "alerts[2].target.north",
                "alerts[2].target.east"
            ]
        );
        let mut r = sst_box(40.0, -74.0, 40.5, -73.5);
        r.all_of = vec![
            Condition::Measurement {
                metric: AlertMetric::Chlorophyll,
                below: None,
                above: Some(1.0),
            },
            Condition::Measurement {
                metric: AlertMetric::Discharge,
                below: Some(1.0),
                above: None,
            },
            Condition::MarineAlert {
                event: "Gale Warning".to_owned(),
            },
            Condition::StormAhead {},
            Condition::WindowStarts { within_minutes: 30 },
            Condition::TripGo {},
        ];
        assert_eq!(
            errors(&[r]),
            [
                "alerts[0].all_of[1].metric",
                "alerts[0].all_of[2]",
                "alerts[0].all_of[3]",
                "alerts[0].all_of[4]",
                "alerts[0].all_of[5]"
            ]
        );
    }

    #[test]
    fn alerts_other_conditions_are_checked() {
        let mut r = spot_rule(
            "a",
            vec![
                Condition::TripGo {},
                Condition::MarineAlert {
                    event: String::new(),
                },
            ],
        );
        r.repeat_after_hours = Some(0);
        r.quiet_hours = Some(QuietHours {
            start_minute: 1440,
            end_minute: 1440,
        });
        assert_eq!(
            paths(&[r]),
            [
                (Severity::Error, "alerts[0].all_of[0]".to_owned()),
                (Severity::Error, "alerts[0].all_of[1].event".to_owned()),
                (Severity::Error, "alerts[0].repeat_after_hours".to_owned()),
                (
                    Severity::Error,
                    "alerts[0].quiet_hours.start_minute".to_owned()
                ),
                (
                    Severity::Error,
                    "alerts[0].quiet_hours.end_minute".to_owned()
                ),
                (Severity::Warning, "alerts[0].quiet_hours".to_owned()),
            ]
        );
    }

    #[test]
    fn alerts_boxes_round_to_the_grid_and_stay_valid() {
        let mut rules = vec![
            sst_box(40.123_456, -72.987_654, 41.123_4, -71.987_6),
            sst_box(40.001, -72.004, 40.003, -72.001),
            sst_box(89.996, 179.991, 89.999, 179.999),
            spot_rule("spot", vec![Condition::StormAhead {}]),
        ];
        let spot = rules[3].clone();
        round_boxes(&mut rules, 2);
        let sides = |r: &AlertRule| match r.target {
            AlertTarget::Box {
                south,
                west,
                north,
                east,
            } => [south, west, north, east],
            _ => unreachable!(),
        };
        let close = |a: [f64; 4], b: [f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(close(sides(&rules[0]), [40.12, -72.99, 41.12, -71.99]));
        assert!(close(sides(&rules[1]), [40.0, -72.01, 40.01, -72.0]));
        assert!(close(sides(&rules[2]), [89.99, 179.99, 90.0, 180.0]));
        assert_eq!(rules[3], spot);
        assert_eq!(errors(&rules), Vec::<String>::new());
        let mut coarse = vec![sst_box(40.3, -73.8, 40.6, -73.4)];
        round_boxes(&mut coarse, 0);
        assert!(close(sides(&coarse[0]), [40.0, -74.0, 41.0, -73.0]));
        assert_eq!(errors(&coarse), Vec::<String>::new());
    }

    #[test]
    fn alerts_with_unknown_kinds_are_dropped_with_a_warning() {
        let mut doc = json!({ "alerts": [
            { "id": "a", "name": "A", "target": { "kind": "circle", "radius": 1 }, "all_of": [] },
            { "id": "b", "name": "B", "target": { "kind": "spot", "id": "s" },
              "all_of": [{ "kind": "storm_ahead" }, { "kind": "rainbow" }] },
            { "id": "c", "name": "C", "target": { "kind": "spot", "id": "s" },
              "all_of": [{ "kind": "measurement", "metric": "rainfall", "above": 1 }] },
            { "id": "d", "name": "D", "target": { "kind": "spot", "id": "s" },
              "all_of": [{ "kind": "measurement", "metric": "gusts", "above": 10 }] },
        ]});
        let mut issues = Issues::default();
        tidy(&mut doc, &mut issues);
        let found: Vec<_> = issues.0.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            found,
            [
                "alerts[0].target.kind",
                "alerts[1].all_of[1].kind",
                "alerts[2].all_of[0].metric"
            ]
        );
        assert!(issues.0.iter().all(|i| i.severity == Severity::Warning));
        assert_eq!(doc["alerts"].as_array().unwrap().len(), 1);
        assert_eq!(doc["alerts"][0]["id"], "d");
    }

    #[test]
    fn alerts_kind_lists_match_the_types() {
        let conditions = [
            Condition::Measurement {
                metric: AlertMetric::Wind,
                below: Some(1.0),
                above: None,
            },
            Condition::MarineAlert {
                event: "Gale Warning".to_owned(),
            },
            Condition::TripGo {},
            Condition::WindowStarts { within_minutes: 0 },
            Condition::StormAhead {},
        ];
        let kinds: Vec<_> = conditions
            .iter()
            .map(|c| serde_json::to_value(c).unwrap()["kind"].clone())
            .collect();
        assert_eq!(serde_json::to_value(CONDITION_KINDS).unwrap(), json!(kinds));
        let targets = [
            AlertTarget::Spot { id: "s".into() },
            AlertTarget::Trip { id: "t".into() },
            sst_box(0.0, 0.0, 1.0, 1.0).target,
        ];
        let kinds: Vec<_> = targets
            .iter()
            .map(|t| serde_json::to_value(t).unwrap()["kind"].clone())
            .collect();
        assert_eq!(serde_json::to_value(TARGET_KINDS).unwrap(), json!(kinds));
    }
}
