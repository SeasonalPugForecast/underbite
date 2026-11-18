//! Alert words: notification titles and bodies for alert events, rule descriptions for the
//! editor and threshold parsing in the user's units.
//!
//! Notifications are read on the lock screen. They name a place (the rule's own name, the spot
//! or trip, the station) only when the rule's `show_name` is on, else they say "Your spot" or
//! "Your trip"; they never carry coordinates. A notification that tells of a marine alert, a
//! storm or a hurricane is safety text: it reads the same in both voices.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use crate::alerts::{
    AlertEvent, AlertMetric, AlertRule, AlertTarget, CheckProblem, Condition, EventKind, Finding,
    FindingReason, RuleState, RuleStatus,
};
use crate::model::Measurement;
use crate::time::UnixTime;
use crate::units::{DischargeUnit, HeightUnit, PressureUnit, pressure_from_hpa, pressure_to_hpa};
use crate::view::cards::alerts::{AlertRuleVm, RuleStateKind, span};
use crate::view::format::Formatter;
use crate::view::strings::{Key, MapQuantity, SafetyKey, Strings, map_value};
use crate::view::{CardId, Voice};

/// One of the spot's alert rules with what its last check found, for the alerts card.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleInput {
    pub rule: AlertRule,
    /// The last check's state; `None` before the first check. A disabled rule has none.
    pub state: Option<RuleState>,
}

/// The names of the user's saved spots and trips, by id: what notifications and the editor call
/// them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaceNames {
    pub spots: BTreeMap<String, String>,
    pub trips: BTreeMap<String, String>,
}

/// The words of one notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct AlertNotification {
    pub title: String,
    /// What met the rule, one line per finding.
    pub body: String,
}

/// A rule in words, for the editor's list and its summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RuleDescription {
    /// What the rule watches: the spot's or trip's name, or "An area on the map" (never its
    /// coordinates).
    pub target: String,
    /// Each condition, in the rule's order: "River flow under 800 cfs".
    pub conditions: Vec<String>,
    /// The conditions on one line.
    pub summary: String,
    /// "Again every 12 h while met"; `None` when the rule waits for its conditions to stop.
    pub repeat: Option<String>,
    /// "Quiet hours 10:00 PM to 7:00 AM".
    pub quiet: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Values in the user's units
// ---------------------------------------------------------------------------------------------

/// A metric's value in canonical units as the user reads it, with its unit: "780 cfs".
pub fn metric_value(f: &Formatter, metric: AlertMetric, canonical: f64) -> String {
    let v = canonical as f32;
    match metric {
        AlertMetric::Discharge => f.discharge(Some(v)).full(),
        AlertMetric::GageHeight => f.stage_length(Some(v)).full(),
        AlertMetric::WaterTemperature | AlertMetric::SeaSurfaceTemp => {
            f.temperature(Some(v)).full()
        }
        AlertMetric::WaveHeight => f.wave_height(Some(v)).full(),
        AlertMetric::Wind | AlertMetric::Gusts => f.wind_speed(Some(v)).full(),
        AlertMetric::PressureTrend => {
            let unit = f.units.pressure;
            f.quantity(
                pressure_from_hpa(unit, v),
                pressure_decimals(unit),
                f.strings.pressure_unit(unit),
            )
            .full()
        }
        AlertMetric::TideHeight => f.length(Some(v)).full(),
        AlertMetric::Chlorophyll => map_value(f, MapQuantity::Chlorophyll, canonical),
    }
}

/// A pressure change keeps a decimal more than a pressure.
const fn pressure_decimals(unit: PressureUnit) -> u8 {
    match unit {
        PressureUnit::Hpa | PressureUnit::MmHg => 1,
        PressureUnit::InHg | PressureUnit::Kpa => 2,
    }
}

/// The unit a threshold of `metric` is typed in: "cfs", "°F", "ft", "µg/L".
pub fn threshold_unit(f: &Formatter, metric: AlertMetric) -> &'static str {
    let (s, u) = (f.strings, f.units);
    match metric {
        AlertMetric::Discharge => s.discharge_unit(u.discharge),
        AlertMetric::GageHeight => s.height_unit(u.stage),
        AlertMetric::WaterTemperature | AlertMetric::SeaSurfaceTemp => s.temp_unit(u.temperature),
        AlertMetric::WaveHeight => s.height_unit(u.waves),
        AlertMetric::Wind | AlertMetric::Gusts => s.speed_unit(u.wind_speed),
        AlertMetric::PressureTrend => s.pressure_unit(u.pressure),
        AlertMetric::TideHeight => s.height_unit(u.height),
        AlertMetric::Chlorophyll => s.get(Key::UnitMicrogramsPerLitre),
    }
}

/// A canonical value in the unit [`threshold_unit`] names.
fn to_display(f: &Formatter, metric: AlertMetric, canonical: f64) -> f32 {
    let (v, u) = (canonical as f32, f.units);
    match metric {
        AlertMetric::Discharge => u.discharge.from_canonical(v),
        AlertMetric::GageHeight => u.stage.from_canonical(v),
        AlertMetric::WaterTemperature | AlertMetric::SeaSurfaceTemp => {
            u.temperature.from_canonical(v)
        }
        AlertMetric::WaveHeight => u.waves.from_canonical(v),
        AlertMetric::Wind | AlertMetric::Gusts => u.wind_speed.from_canonical(v),
        AlertMetric::PressureTrend => pressure_from_hpa(u.pressure, v),
        AlertMetric::TideHeight => u.height.from_canonical(v),
        AlertMetric::Chlorophyll => v,
    }
}

/// A value typed in the unit [`threshold_unit`] names, in canonical units.
fn from_display(f: &Formatter, metric: AlertMetric, typed: f32) -> f64 {
    let u = f.units;
    f64::from(match metric {
        AlertMetric::Discharge => u.discharge.to_canonical(typed),
        AlertMetric::GageHeight => u.stage.to_canonical(typed),
        AlertMetric::WaterTemperature | AlertMetric::SeaSurfaceTemp => {
            u.temperature.to_canonical(typed)
        }
        AlertMetric::WaveHeight => u.waves.to_canonical(typed),
        AlertMetric::Wind | AlertMetric::Gusts => u.wind_speed.to_canonical(typed),
        AlertMetric::PressureTrend => pressure_to_hpa(u.pressure, typed),
        AlertMetric::TideHeight => u.height.to_canonical(typed),
        AlertMetric::Chlorophyll => typed,
    })
}

/// The decimals a threshold field shows, in its unit.
fn field_decimals(f: &Formatter, metric: AlertMetric, shown: f32) -> u8 {
    let height = |unit: HeightUnit| match unit {
        HeightUnit::Ft => 1,
        HeightUnit::M => 2,
    };
    match metric {
        AlertMetric::Discharge => match f.units.discharge {
            DischargeUnit::Cfs => u8::from(shown.abs() < 100.0),
            DischargeUnit::M3s => 1 + u8::from(shown.abs() < 10.0),
        },
        AlertMetric::GageHeight => height(f.units.stage),
        AlertMetric::WaveHeight => height(f.units.waves),
        AlertMetric::TideHeight => height(f.units.height),
        AlertMetric::WaterTemperature
        | AlertMetric::SeaSurfaceTemp
        | AlertMetric::Wind
        | AlertMetric::Gusts => 1,
        AlertMetric::PressureTrend => pressure_decimals(f.units.pressure),
        AlertMetric::Chlorophyll => 2,
    }
}

/// The values a threshold may take, in canonical units: beyond them a rule could never fire,
/// or the number is a typing slip.
const fn threshold_bounds(metric: AlertMetric) -> (f64, f64) {
    match metric {
        AlertMetric::Discharge => (0.0, 100_000.0),
        AlertMetric::GageHeight => (-30.0, 300.0),
        AlertMetric::WaterTemperature | AlertMetric::SeaSurfaceTemp => (-5.0, 45.0),
        AlertMetric::WaveHeight => (0.0, 30.0),
        AlertMetric::Wind | AlertMetric::Gusts => (0.0, 100.0),
        AlertMetric::PressureTrend => (-50.0, 50.0),
        AlertMetric::TideHeight => (-10.0, 20.0),
        AlertMetric::Chlorophyll => (0.0, 1_000.0),
    }
}

/// Why a typed threshold was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ThresholdError {
    #[error("not a number")]
    NotANumber,
    #[error("out of range")]
    OutOfRange,
}

impl ThresholdError {
    /// The message for the field, in the user's units, voice and locale.
    #[must_use]
    pub fn text(self, f: &Formatter, metric: AlertMetric) -> String {
        let s = f.strings;
        match self {
            Self::NotANumber => s.fmt(
                Key::AlertThresholdNumber,
                &[("example", &s.number(12.5, 1))],
            ),
            Self::OutOfRange => {
                let (min, max) = threshold_bounds(metric);
                s.fmt(
                    Key::AlertThresholdRange,
                    &[
                        ("min", &metric_value(f, metric, min)),
                        ("max", &metric_value(f, metric, max)),
                    ],
                )
            }
        }
    }
}

/// A threshold typed in the user's unit for `metric` ([`threshold_unit`]), in canonical units;
/// blank text is no threshold. A leading minus is allowed (temperatures, pressure falls, tides
/// below datum); the locale's decimal mark, at most once; no thousands separators.
///
/// # Errors
/// [`ThresholdError::NotANumber`] for anything else, [`ThresholdError::OutOfRange`] beyond what
/// the metric can take.
pub fn parse_threshold(
    f: &Formatter,
    metric: AlertMetric,
    text: &str,
) -> Result<Option<f64>, ThresholdError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let mark = f.strings.locale.decimal_mark();
    let mut normalised = String::with_capacity(text.len() + 1);
    let mut rest = text;
    if let Some(r) = rest.strip_prefix(['-', '\u{2212}']) {
        normalised.push('-');
        rest = r;
    } else if let Some(r) = rest.strip_prefix('+') {
        rest = r;
    }
    let (mut digits, mut marked) = (false, false);
    for c in rest.chars() {
        match c {
            '0'..='9' => {
                digits = true;
                normalised.push(c);
            }
            c if c == mark && !marked => {
                marked = true;
                normalised.push('.');
            }
            _ => return Err(ThresholdError::NotANumber),
        }
    }
    if !digits {
        return Err(ThresholdError::NotANumber);
    }
    let typed: f64 = normalised.parse().map_err(|_| ThresholdError::NotANumber)?;
    if !typed.is_finite() || typed.abs() > f64::from(f32::MAX) {
        return Err(ThresholdError::OutOfRange);
    }
    let canonical = round_significant(from_display(f, metric, typed as f32));
    let (min, max) = threshold_bounds(metric);
    // A bound typed in another unit comes back a hair beyond it.
    let slack = |bound: f64| 1e-4 * bound.abs().max(1.0);
    if canonical < min - slack(min) || canonical > max + slack(max) {
        return Err(ThresholdError::OutOfRange);
    }
    Ok(Some(canonical.clamp(min, max)))
}

/// Six significant digits: what a typed number carries, without the noise of single precision.
fn round_significant(v: f64) -> f64 {
    if v.abs() < f64::MIN_POSITIVE || !v.is_finite() {
        return v;
    }
    let digits = 5 - v.abs().log10().floor() as i32;
    let scale = 10f64.powi(digits);
    (v * scale).round() / scale
}

/// A canonical threshold for the editor's field, in the user's unit, without trailing zeros and
/// with the locale's decimal mark: "800", "18.9".
pub fn edit_threshold(f: &Formatter, metric: AlertMetric, canonical: f64) -> String {
    let shown = to_display(f, metric, canonical);
    let text = f.strings.number(shown, field_decimals(f, metric, shown));
    let mark = f.strings.locale.decimal_mark();
    if text.contains(mark) {
        text.trim_end_matches('0').trim_end_matches(mark).to_owned()
    } else {
        text
    }
}

// ---------------------------------------------------------------------------------------------
// Rule descriptions
// ---------------------------------------------------------------------------------------------

/// One condition in words: "River flow under 800 cfs", "Small Craft Advisory in effect".
pub fn condition_text(f: &Formatter, condition: &Condition) -> String {
    let s = f.strings;
    match condition {
        Condition::Measurement {
            metric,
            below,
            above,
        } => {
            let what = s.alert_metric(*metric);
            let value = |v: f64| metric_value(f, *metric, v);
            match (below, above) {
                (Some(b), Some(a)) => s.fmt(
                    Key::AlertRuleBetween,
                    &[("what", what), ("low", &value(*a)), ("high", &value(*b))],
                ),
                (Some(b), None) => s.fmt(
                    Key::AlertRuleBelow,
                    &[("what", what), ("threshold", &value(*b))],
                ),
                (None, Some(a)) => s.fmt(
                    Key::AlertRuleAbove,
                    &[("what", what), ("threshold", &value(*a))],
                ),
                (None, None) => s.fmt(Key::AlertRuleNoThreshold, &[("what", what)]),
            }
        }
        Condition::MarineAlert { event } => {
            s.safety_fmt(SafetyKey::AlertRuleMarine, &[("alert", event)])
        }
        Condition::TripGo {} => s.get(Key::AlertRuleTripGo).to_owned(),
        Condition::WindowStarts { within_minutes } => s.fmt(
            Key::AlertRuleWindow,
            &[("duration", &f.duration(i64::from(*within_minutes) * 60))],
        ),
        Condition::StormAhead {} => s.safety(SafetyKey::AlertRuleStorm).to_owned(),
    }
}

/// A rule's conditions on one line: "River flow under 800 cfs · Water temperature under 66 °F".
pub fn conditions_summary(f: &Formatter, rule: &AlertRule) -> String {
    let s = f.strings;
    rule.all_of
        .iter()
        .map(|c| condition_text(f, c))
        .reduce(|a, b| s.fmt(Key::AlertRuleJoin, &[("a", &a), ("b", &b)]))
        .unwrap_or_else(|| s.get(Key::AlertRuleNoConditions).to_owned())
}

/// A rule in words for the editor, in the user's units. The editor is on the device, so the
/// spot or trip is named whatever `show_name` says; a box is never given as coordinates.
pub fn describe_rule(f: &Formatter, rule: &AlertRule, names: &PlaceNames) -> RuleDescription {
    let s = f.strings;
    let target = match &rule.target {
        AlertTarget::Spot { id } => names
            .spots
            .get(id)
            .cloned()
            .unwrap_or_else(|| s.get(Key::AlertTargetSpotGone).to_owned()),
        AlertTarget::Trip { id } => names
            .trips
            .get(id)
            .cloned()
            .unwrap_or_else(|| s.get(Key::AlertTargetTripGone).to_owned()),
        AlertTarget::Box { .. } => s.get(Key::AlertTargetBox).to_owned(),
    };
    let repeat = rule.repeat_after_hours.map(|h| {
        s.fmt(
            Key::AlertRuleRepeat,
            &[("duration", &f.duration(i64::from(h) * 3_600))],
        )
    });
    let quiet = rule.quiet_hours.map(|q| {
        let range = s.fmt(
            Key::Range,
            &[
                ("from", &minute_clock(f, q.start_minute)),
                ("to", &minute_clock(f, q.end_minute)),
            ],
        );
        s.fmt(Key::AlertRuleQuiet, &[("range", &range)])
    });
    RuleDescription {
        target,
        conditions: rule.all_of.iter().map(|c| condition_text(f, c)).collect(),
        summary: conditions_summary(f, rule),
        repeat,
        quiet,
    }
}

/// A minute of the day on the user's clock: "10:00 PM", "22:00".
fn minute_clock(f: &Formatter, minute: u16) -> String {
    Formatter::new(f.units, f.strings.voice, f.clock, "UTC")
        .with_locale(f.strings.locale)
        .clock(UnixTime(i64::from(minute) * 60))
}

/// A time today as a clock time, else with its date: "6:10 AM", "Nov 7, 6:10 AM".
pub(in crate::view) fn when_from(f: &Formatter, t: UnixTime, now: UnixTime) -> String {
    let time = f.clock(t);
    let date = f.date_of(t);
    if date == f.date_of(now) {
        time
    } else {
        f.strings
            .fmt(Key::AlertDayTime, &[("date", &date), ("time", &time)])
    }
}

// ---------------------------------------------------------------------------------------------
// Rule states on the alerts card
// ---------------------------------------------------------------------------------------------

/// What a condition is about, as a rule's state names it: "River flow", "Marine alerts".
fn condition_subject(s: Strings, condition: Option<&Condition>) -> &'static str {
    match condition {
        Some(Condition::Measurement { metric, .. }) => s.alert_metric(*metric),
        Some(Condition::MarineAlert { .. }) => s.measurement(Measurement::MarineAlerts),
        Some(Condition::TripGo {}) => s.card_title(CardId::GoNoGo),
        Some(Condition::WindowStarts { .. }) => s.card_title(CardId::Windows),
        Some(Condition::StormAhead {}) => s.card_title(CardId::StormAhead),
        None => s.get(Key::AlertRuleACondition),
    }
}

/// An age without "ago", rounded down: "45 min", "5 h", "2 d".
fn age_span(f: &Formatter, seconds: i64) -> String {
    let s = seconds.max(0);
    let (key, n) = if s < 3_600 {
        (Key::DurationMinutes, s / 60)
    } else if s < 86_400 {
        (Key::DurationHours, s / 3_600)
    } else {
        (Key::DurationDays, s / 86_400)
    };
    f.strings.fmt(key, &[("n", &n.to_string())])
}

/// A rule's state in words: "Met since 6:10 AM", "River flow: couldn't check, data 5 h old".
fn state_words(f: &Formatter, input: &RuleInput, now: UnixTime) -> (RuleStateKind, String) {
    let s = f.strings;
    let rule = &input.rule;
    let Some(state) = input.state.as_ref().filter(|_| rule.enabled) else {
        return if rule.enabled {
            (
                RuleStateKind::NotChecked,
                s.get(Key::AlertRuleNotChecked).to_owned(),
            )
        } else {
            (RuleStateKind::Off, s.get(Key::AlertRuleOff).to_owned())
        };
    };
    let what = |i: usize| condition_subject(s, rule.all_of.get(i));
    match state.status {
        RuleStatus::Met => (
            RuleStateKind::Met,
            state.met_since.map_or_else(
                || s.get(Key::AlertRuleMet).to_owned(),
                |t| s.fmt(Key::AlertRuleMetSince, &[("time", &when_from(f, t, now))]),
            ),
        ),
        RuleStatus::NotMet => (
            RuleStateKind::NotMet,
            s.get(Key::AlertRuleNotMet).to_owned(),
        ),
        RuleStatus::Unchecked { condition, problem } => {
            let what = what(condition);
            let text = match problem {
                CheckProblem::Stale { age_s, .. } => s.fmt(
                    Key::AlertRuleStale,
                    &[("what", what), ("age", &age_span(f, age_s))],
                ),
                CheckProblem::Far { distance_m, .. } => s.fmt(
                    Key::AlertRuleFar,
                    &[
                        ("what", what),
                        ("distance", &f.distance(Some(distance_m)).full()),
                    ],
                ),
                CheckProblem::SourceFailed => s.fmt(Key::AlertRuleFailed, &[("what", what)]),
            };
            (RuleStateKind::Unchecked, text)
        }
        RuleStatus::NoData { condition } => (
            RuleStateKind::NoData,
            s.fmt(Key::AlertRuleNoData, &[("what", what(condition))]),
        ),
        RuleStatus::Orphaned => (
            RuleStateKind::Orphaned,
            s.get(Key::AlertRuleOrphaned).to_owned(),
        ),
    }
}

/// A rule's row on the alerts card: its name, its conditions in the user's units and what its
/// last check found, at `now`. The card is on the device, so the rule's name is shown whatever
/// `show_name` says.
pub fn rule_row(f: &Formatter, input: &RuleInput, now: UnixTime) -> AlertRuleVm {
    let rule = &input.rule;
    let (state, state_label) = state_words(f, input, now);
    let checked = input.state.as_ref().filter(|_| rule.enabled).map(|st| {
        f.strings.fmt(
            Key::AlertRuleChecked,
            &[("time", &when_from(f, st.checked_at, now))],
        )
    });
    AlertRuleVm {
        id: rule.id.clone(),
        name: rule.name.clone(),
        summary: conditions_summary(f, rule),
        state,
        state_label,
        checked,
    }
}

// ---------------------------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------------------------

/// Marine alerts, storms and hurricanes: safety text, never in Luna's voice.
const fn is_safety(reason: FindingReason) -> bool {
    matches!(
        reason,
        FindingReason::AlertNew
            | FindingReason::AlertStronger
            | FindingReason::AlertExtended
            | FindingReason::AlertInForce
            | FindingReason::StormAhead
            | FindingReason::Hurricane
    )
}

/// The notification for `event` of `rule`, in the user's units, clock and voice; `f` is in the
/// target's time zone. With `show_name` off it names no place, rule or station. A notification
/// with a marine alert, storm or hurricane in it is written plainly in every voice.
pub fn notification(
    f: &Formatter,
    event: &AlertEvent,
    rule: &AlertRule,
    names: &PlaceNames,
) -> AlertNotification {
    let mut f = f.clone();
    if event.findings.iter().any(|x| is_safety(x.reason)) {
        f.strings.voice = Voice::Plain;
    }
    let f = &f;
    let s = f.strings;
    let name = if rule.show_name {
        rule.name.clone()
    } else {
        s.get(match rule.target {
            AlertTarget::Spot { .. } => Key::AlertNameSpot,
            AlertTarget::Trip { .. } => Key::AlertNameTrip,
            AlertTarget::Box { .. } => Key::AlertNameArea,
        })
        .to_owned()
    };
    let title = match event.kind {
        EventKind::Met => name,
        EventKind::Update => s.fmt(Key::AlertNotifyUpdate, &[("rule", &name)]),
        EventKind::Repeat => s.fmt(Key::AlertNotifyRepeat, &[("rule", &name)]),
    };
    let body = event
        .findings
        .iter()
        .map(|x| finding_line(f, x, rule, names, event.found_at))
        .collect::<Vec<_>>()
        .join("\n");
    AlertNotification { title, body }
}

/// Where a finding is, as a notification may say it: the saved spot or trip by name when the
/// rule shows names, else "Your spot", "Your trip" or "Your area".
fn place(s: Strings, finding: &Finding, rule: &AlertRule, names: &PlaceNames) -> String {
    if rule.show_name {
        let saved = finding
            .spot_id
            .as_ref()
            .and_then(|id| names.spots.get(id))
            .or_else(|| match &rule.target {
                AlertTarget::Spot { id } => names.spots.get(id),
                AlertTarget::Trip { id } => names.trips.get(id),
                AlertTarget::Box { .. } => None,
            });
        if let Some(name) = saved {
            return name.clone();
        }
    }
    s.get(match rule.target {
        AlertTarget::Spot { .. } => Key::AlertPlaceSpot,
        AlertTarget::Trip { .. } => Key::AlertPlaceTrip,
        AlertTarget::Box { .. } => Key::AlertPlaceArea,
    })
    .to_owned()
}

/// Where a value comes from: the station by name when the rule shows names, else the source by
/// name, else (a grid the table has no name for) the place.
fn value_source(s: Strings, finding: &Finding, rule: &AlertRule, names: &PlaceNames) -> String {
    if let Some(station) = finding.station.as_ref().filter(|_| rule.show_name) {
        return station.name.clone();
    }
    finding
        .source
        .as_ref()
        .or_else(|| finding.station.as_ref().map(|st| &st.source))
        .map(|id| (s.source_name(id), id))
        .filter(|(name, id)| name != id.as_str())
        .map_or_else(|| place(s, finding, rule, names), |(name, _)| name)
}

/// A value against the rule's threshold: "Croton R at New Croton Dam: River flow 780 cfs, under
/// your 800 cfs". `None` when the rule no longer has that measurement.
fn threshold_line(
    f: &Formatter,
    finding: &Finding,
    rule: &AlertRule,
    names: &PlaceNames,
) -> Option<String> {
    let strings = f.strings;
    let (
        Some(Condition::Measurement {
            metric,
            below,
            above,
        }),
        Some(measured),
    ) = (rule.all_of.get(finding.condition), finding.measured)
    else {
        return None;
    };
    let value = |v: f64| metric_value(f, *metric, v);
    let mut args = vec![
        ("place", value_source(strings, finding, rule, names)),
        ("what", strings.alert_metric(*metric).to_owned()),
        ("value", value(measured)),
    ];
    let key = match (below, above) {
        (Some(high), Some(low)) => {
            args.extend([("low", value(*low)), ("high", value(*high))]);
            Key::AlertNotifyBetween
        }
        (Some(threshold), None) => {
            args.push(("threshold", value(*threshold)));
            Key::AlertNotifyBelow
        }
        (None, Some(threshold)) => {
            args.push(("threshold", value(*threshold)));
            Key::AlertNotifyAbove
        }
        (None, None) => Key::AlertNotifyValue,
    };
    let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
    Some(strings.fmt(key, &args))
}

/// A fishing window: "Harbor weekend: fishing window 6:10 to 8:10 AM", or "now, until 8:10 AM"
/// once it has started. `None` without its start.
fn window_line(
    f: &Formatter,
    finding: &Finding,
    place: &str,
    found_at: UnixTime,
) -> Option<String> {
    let strings = f.strings;
    let start = finding.at?;
    Some(match finding.until {
        Some(end) if start <= found_at => strings.fmt(
            Key::AlertNotifyWindowNow,
            &[("place", place), ("time", &when_from(f, end, found_at))],
        ),
        Some(end) if f.date_of(start) == f.date_of(found_at) => strings.fmt(
            Key::AlertNotifyWindow,
            &[("place", place), ("range", &f.clock_range(start, end))],
        ),
        _ => strings.fmt(
            Key::AlertNotifyWindow,
            &[("place", place), ("range", &when_from(f, start, found_at))],
        ),
    })
}

/// A marine alert, storm or hurricane: safety text, with when it is in effect.
fn safety_line(f: &Formatter, finding: &Finding, place: &str) -> String {
    let strings = f.strings;
    let subject = |fallback: SafetyKey| {
        finding
            .subject
            .clone()
            .unwrap_or_else(|| strings.safety(fallback).to_owned())
    };
    let text = match finding.reason {
        FindingReason::StormAhead => {
            strings.safety_fmt(SafetyKey::AlertNotifyStorm, &[("place", place)])
        }
        FindingReason::Hurricane => strings.safety_fmt(
            SafetyKey::AlertNotifyHurricane,
            &[
                ("place", place),
                ("storm", &subject(SafetyKey::AlertNameCyclone)),
            ],
        ),
        reason => {
            let key = match reason {
                FindingReason::AlertNew => SafetyKey::AlertNotifyNew,
                FindingReason::AlertStronger => SafetyKey::AlertNotifyStronger,
                FindingReason::AlertExtended => SafetyKey::AlertNotifyExtended,
                _ => SafetyKey::AlertNotifyInForce,
            };
            strings.safety_fmt(
                key,
                &[
                    ("place", place),
                    ("alert", &subject(SafetyKey::AlertNameMarine)),
                ],
            )
        }
    };
    match span(f, finding.at, finding.until) {
        Some(span) => strings.safety_fmt(
            SafetyKey::AlertNotifySpan,
            &[("text", &text), ("span", &span)],
        ),
        None => text,
    }
}

/// One line of a notification body: what met one condition.
fn finding_line(
    f: &Formatter,
    finding: &Finding,
    rule: &AlertRule,
    names: &PlaceNames,
    found_at: UnixTime,
) -> String {
    let strings = f.strings;
    let here = place(strings, finding, rule, names);
    let line = match finding.reason {
        FindingReason::Threshold => threshold_line(f, finding, rule, names),
        FindingReason::WindowStarts => window_line(f, finding, &here, found_at),
        FindingReason::TripGo => finding.at.map(|day| {
            strings.fmt(
                Key::AlertNotifyTripGo,
                &[("place", &here), ("day", &f.date_of(day))],
            )
        }),
        FindingReason::AlertNew
        | FindingReason::AlertStronger
        | FindingReason::AlertExtended
        | FindingReason::AlertInForce
        | FindingReason::StormAhead
        | FindingReason::Hurricane => Some(safety_line(f, finding, &here)),
    };
    // A finding the rule, edited since, no longer describes.
    line.unwrap_or_else(|| strings.fmt(Key::AlertNotifyMet, &[("place", &here)]))
}

#[cfg(test)]
pub(in crate::view) mod tests {
    use super::*;
    use crate::alerts::{Condition as C, QuietHours};
    use crate::provenance::{SourceId, StationKind, StationRef, UsgsSiteType};
    use crate::time::parse_rfc3339;
    use crate::view::cards::flow::fixtures::{CFS, metric, us};
    use crate::view::locale::{Locale, NumberStyle};
    use crate::view::{DisplayUnits, Voice};
    use whether_core::presentation::TimeFormat;

    pub(in crate::view) fn at(iso: &str) -> UnixTime {
        parse_rfc3339(iso).unwrap()
    }

    pub(in crate::view) fn formatter(units: DisplayUnits, voice: Voice) -> Formatter {
        Formatter::new(units, voice, TimeFormat::H12, "America/New_York")
    }

    /// 800 cfs and 66 °F in canonical units.
    pub(in crate::view) const CROTON_LOW_M3S: f64 = 800.0 * CFS as f64;
    pub(in crate::view) const COOL_C: f64 = (66.0 - 32.0) / 1.8;

    pub(in crate::view) fn rule(id: &str, target: AlertTarget, all_of: Vec<C>) -> AlertRule {
        AlertRule {
            id: id.into(),
            name: "Croton low and cool".into(),
            enabled: true,
            target,
            all_of,
            repeat_after_hours: None,
            quiet_hours: None,
            show_name: true,
        }
    }

    pub(in crate::view) fn spot(id: &str) -> AlertTarget {
        AlertTarget::Spot { id: id.into() }
    }

    pub(in crate::view) fn below(metric: AlertMetric, v: f64) -> C {
        C::Measurement {
            metric,
            below: Some(v),
            above: None,
        }
    }

    pub(in crate::view) fn croton_rule() -> AlertRule {
        rule(
            "croton-low",
            spot("croton"),
            vec![
                below(AlertMetric::Discharge, CROTON_LOW_M3S),
                below(AlertMetric::WaterTemperature, COOL_C),
            ],
        )
    }

    pub(in crate::view) fn names() -> PlaceNames {
        PlaceNames {
            spots: [
                ("croton".to_owned(), "Croton River".to_owned()),
                ("battery".to_owned(), "The Battery".to_owned()),
            ]
            .into(),
            trips: [("trip-1".to_owned(), "Harbor weekend".to_owned())].into(),
        }
    }

    fn croton_station() -> StationRef {
        StationRef {
            source: SourceId::USGS,
            id: "USGS-01375000".into(),
            name: "Croton R at New Croton Dam".into(),
            kind: StationKind::UsgsSite(UsgsSiteType::Stream),
            latitude: 41.2251,
            longitude: -73.8579,
            bin: None,
            reference_id: None,
        }
    }

    fn finding(condition: usize, reason: FindingReason) -> Finding {
        Finding {
            condition,
            reason,
            measured: None,
            at: None,
            until: None,
            spot_id: None,
            source: None,
            station: None,
            subject: None,
        }
    }

    fn croton_event() -> AlertEvent {
        let now = at("2026-06-20T10:10:00Z");
        let reading = |condition, v: f64| Finding {
            measured: Some(v),
            at: Some(at("2026-06-20T10:00:00Z")),
            spot_id: Some("croton".into()),
            source: Some(SourceId::USGS),
            station: Some(croton_station()),
            ..finding(condition, FindingReason::Threshold)
        };
        AlertEvent {
            rule_id: "croton-low".into(),
            kind: EventKind::Met,
            found_at: now,
            findings: vec![reading(0, 780.0 * f64::from(CFS)), reading(1, 16.5)],
        }
    }

    /// The recorded ANZ338 small craft advisory, as evaluation reports it for a trip.
    fn advisory_event(reason: FindingReason) -> (AlertEvent, AlertRule) {
        let rule = AlertRule {
            name: "Harbor advisory".into(),
            ..rule(
                "sca",
                AlertTarget::Trip {
                    id: "trip-1".into(),
                },
                vec![C::MarineAlert {
                    event: "Small Craft Advisory".into(),
                }],
            )
        };
        let event = AlertEvent {
            rule_id: "sca".into(),
            kind: EventKind::Met,
            found_at: at("2026-11-08T14:30:00Z"),
            findings: vec![Finding {
                at: Some(at("2026-11-08T06:48:00Z")),
                until: Some(at("2026-11-09T23:00:00Z")),
                spot_id: Some("battery".into()),
                source: Some(SourceId::NWS),
                subject: Some("Small Craft Advisory".into()),
                ..finding(0, reason)
            }],
        };
        (event, rule)
    }

    fn storm_event() -> (AlertEvent, AlertRule) {
        let rule = AlertRule {
            name: "Storms at the Battery".into(),
            ..rule("storm", spot("battery"), vec![C::StormAhead {}])
        };
        let event = AlertEvent {
            rule_id: "storm".into(),
            kind: EventKind::Met,
            found_at: at("2026-11-08T14:30:00Z"),
            findings: vec![
                Finding {
                    at: Some(at("2026-11-09T18:00:00Z")),
                    until: Some(at("2026-11-10T12:00:00Z")),
                    spot_id: Some("battery".into()),
                    source: Some(SourceId::new("open-meteo")),
                    ..finding(0, FindingReason::StormAhead)
                },
                Finding {
                    at: Some(at("2026-11-10T00:00:00Z")),
                    until: Some(at("2026-11-11T06:00:00Z")),
                    spot_id: Some("battery".into()),
                    source: Some(SourceId::new("nhc")),
                    subject: Some("Hurricane Melissa".into()),
                    ..finding(0, FindingReason::Hurricane)
                },
            ],
        };
        (event, rule)
    }

    fn window_and_go_event(started: bool) -> (AlertEvent, AlertRule) {
        let rule = AlertRule {
            name: "Weekend bite".into(),
            ..rule(
                "bite",
                AlertTarget::Trip {
                    id: "trip-1".into(),
                },
                vec![C::TripGo {}, C::WindowStarts { within_minutes: 45 }],
            )
        };
        let found_at = at("2026-11-14T10:30:00Z");
        let start = if started {
            at("2026-11-14T10:10:00Z")
        } else {
            at("2026-11-14T11:10:00Z")
        };
        let event = AlertEvent {
            rule_id: "bite".into(),
            kind: EventKind::Met,
            found_at,
            findings: vec![
                Finding {
                    at: Some(at("2026-11-14T05:00:00Z")),
                    ..finding(0, FindingReason::TripGo)
                },
                Finding {
                    at: Some(start),
                    until: Some(at("2026-11-14T13:10:00Z")),
                    ..finding(1, FindingReason::WindowStarts)
                },
            ],
        };
        (event, rule)
    }

    fn sst_box_event() -> (AlertEvent, AlertRule) {
        let rule = AlertRule {
            name: "Warm water off the Hudson Canyon".into(),
            ..rule(
                "warm",
                AlertTarget::Box {
                    south: 39.4123,
                    west: -72.3456,
                    north: 39.9876,
                    east: -71.8765,
                },
                vec![C::Measurement {
                    metric: AlertMetric::SeaSurfaceTemp,
                    below: None,
                    above: Some(20.0),
                }],
            )
        };
        let event = AlertEvent {
            rule_id: "warm".into(),
            kind: EventKind::Repeat,
            found_at: at("2026-08-02T12:00:00Z"),
            findings: vec![Finding {
                measured: Some(21.4),
                at: Some(at("2026-08-01T12:00:00Z")),
                source: Some(SourceId::new("mur-sst")),
                ..finding(0, FindingReason::Threshold)
            }],
        };
        (event, rule)
    }

    /// Every kind of event, with its rule.
    pub(in crate::view) fn events() -> Vec<(AlertEvent, AlertRule)> {
        vec![
            (croton_event(), croton_rule()),
            advisory_event(FindingReason::AlertNew),
            advisory_event(FindingReason::AlertStronger),
            advisory_event(FindingReason::AlertExtended),
            advisory_event(FindingReason::AlertInForce),
            storm_event(),
            window_and_go_event(false),
            window_and_go_event(true),
            sst_box_event(),
        ]
    }

    fn hidden(mut rule: AlertRule) -> AlertRule {
        rule.show_name = false;
        rule
    }

    #[test]
    fn a_croton_notification_reads_in_the_users_units_with_the_station() {
        let rule = croton_rule();
        let event = croton_event();
        let us_plain = notification(&formatter(us(), Voice::Plain), &event, &rule, &names());
        assert_eq!(us_plain.title, "Croton low and cool");
        assert_eq!(
            us_plain.body,
            "Croton R at New Croton Dam: River flow 780 cfs, under your 800 cfs\n\
             Croton R at New Croton Dam: Water temperature 62 °F, under your 66 °F"
        );
        let metric_plain =
            notification(&formatter(metric(), Voice::Plain), &event, &rule, &names());
        assert_eq!(
            metric_plain.body,
            "Croton R at New Croton Dam: River flow 22 m³/s, under your 23 m³/s\n\
             Croton R at New Croton Dam: Water temperature 16.5 °C, under your 18.9 °C"
        );
        let mut all = BTreeMap::new();
        for (units, u) in [("us", us()), ("metric", metric())] {
            for voice in [Voice::Luna, Voice::Plain] {
                all.insert(
                    format!("{units} {voice:?}"),
                    notification(&formatter(u, voice), &event, &rule, &names()),
                );
            }
        }
        insta::assert_json_snapshot!("alert_notification_croton", all);
    }

    #[test]
    fn without_names_a_notification_names_no_place_rule_or_station() {
        let event = croton_event();
        let n = notification(
            &formatter(us(), Voice::Luna),
            &event,
            &hidden(croton_rule()),
            &names(),
        );
        assert_eq!(n.title, "Alert for your spot");
        assert_eq!(
            n.body,
            "USGS: River flow 780 cfs, under your 800 cfs\n\
             USGS: Water temperature 62 °F, under your 66 °F"
        );
        let (event, rule) = advisory_event(FindingReason::AlertNew);
        let n = notification(
            &formatter(us(), Voice::Plain),
            &event,
            &hidden(rule),
            &names(),
        );
        assert_eq!(n.title, "Alert for your trip");
        assert_eq!(
            n.body,
            "Your trip: Small Craft Advisory in effect, Nov 8, 1:48 AM – Nov 9, 6:00 PM"
        );
        let (event, rule) = window_and_go_event(false);
        let n = notification(
            &formatter(us(), Voice::Luna),
            &event,
            &hidden(rule),
            &names(),
        );
        assert_eq!(n.title, "Alert for your trip");
        assert!(
            n.body.starts_with("Your trip: Luna approves of Nov 14"),
            "{}",
            n.body
        );
        let (event, rule) = sst_box_event();
        let n = notification(
            &formatter(us(), Voice::Plain),
            &event,
            &hidden(rule),
            &names(),
        );
        assert_eq!(n.title, "Alert for your area: still met");
        // Every word that could place the spot is gone.
        for (event, rule) in [
            (croton_event(), croton_rule()),
            advisory_event(FindingReason::AlertNew),
            storm_event(),
            window_and_go_event(true),
            sst_box_event(),
        ] {
            for voice in [Voice::Luna, Voice::Plain] {
                let n = notification(
                    &formatter(us(), voice),
                    &event,
                    &hidden(rule.clone()),
                    &names(),
                );
                let text = format!("{}\n{}", n.title, n.body);
                for private in [
                    rule.name.as_str(),
                    "Croton",
                    "Battery",
                    "Harbor weekend",
                    "Hudson",
                ] {
                    assert!(!text.contains(private), "{private} in {text}");
                }
            }
        }
    }

    #[test]
    fn notifications_never_carry_coordinates() {
        for (event, rule) in [
            (croton_event(), croton_rule()),
            advisory_event(FindingReason::AlertExtended),
            storm_event(),
            window_and_go_event(false),
            sst_box_event(),
        ] {
            for show in [true, false] {
                let rule = AlertRule {
                    show_name: show,
                    ..rule.clone()
                };
                let n = notification(&formatter(metric(), Voice::Plain), &event, &rule, &names());
                let text = format!("{} {}", n.title, n.body);
                for coordinate in ["41.2", "73.8", "39.4", "72.3", "39.9", "71.8"] {
                    assert!(!text.contains(coordinate), "{coordinate} in {text}");
                }
            }
        }
    }

    #[test]
    fn advisories_storms_and_hurricanes_read_the_same_in_both_voices() {
        let mut all = BTreeMap::new();
        for reason in [
            FindingReason::AlertNew,
            FindingReason::AlertStronger,
            FindingReason::AlertExtended,
            FindingReason::AlertInForce,
        ] {
            let (event, rule) = advisory_event(reason);
            let luna = notification(&formatter(us(), Voice::Luna), &event, &rule, &names());
            let plain = notification(&formatter(us(), Voice::Plain), &event, &rule, &names());
            assert_eq!(luna, plain, "{reason:?}");
            assert!(!luna.body.contains("Luna"));
            all.insert(format!("{reason:?}"), plain);
        }
        assert_eq!(
            all["AlertNew"].body,
            "The Battery: Small Craft Advisory in effect, Nov 8, 1:48 AM – Nov 9, 6:00 PM"
        );
        let (event, rule) = storm_event();
        let luna = notification(&formatter(us(), Voice::Luna), &event, &rule, &names());
        let plain = notification(&formatter(us(), Voice::Plain), &event, &rule, &names());
        assert_eq!(luna, plain);
        assert_eq!(
            plain.body,
            "The Battery: a significant storm is forecast, Nov 9, 1:00 PM – Nov 10, 7:00 AM\n\
             The Battery: in the forecast cone of Hurricane Melissa, Nov 9, 7:00 PM – Nov 11, 1:00 AM"
        );
        all.insert("storm".into(), plain);
        // A window or a go day in the same notification is plain too.
        let (mut event, mut rule) = window_and_go_event(false);
        let (advisory, _) = advisory_event(FindingReason::AlertNew);
        event
            .findings
            .extend(advisory.findings.iter().map(|x| Finding {
                condition: 2,
                ..x.clone()
            }));
        rule.all_of.push(C::MarineAlert {
            event: "Small Craft Advisory".into(),
        });
        let luna = notification(&formatter(us(), Voice::Luna), &event, &rule, &names());
        assert_eq!(
            luna,
            notification(&formatter(us(), Voice::Plain), &event, &rule, &names())
        );
        assert!(!luna.body.contains("Luna") && !luna.body.contains("snack"));
        insta::assert_json_snapshot!("alert_notification_safety", all);
    }

    #[test]
    fn windows_and_go_days_follow_the_voice() {
        let mut all = BTreeMap::new();
        for started in [false, true] {
            let (event, rule) = window_and_go_event(started);
            for voice in [Voice::Luna, Voice::Plain] {
                all.insert(
                    format!("started {started} {voice:?}"),
                    notification(&formatter(us(), voice), &event, &rule, &names()),
                );
            }
        }
        assert_eq!(
            all["started false Plain"].body,
            "Harbor weekend: Nov 14 looks good\n\
             Harbor weekend: fishing window 6:10 to 8:10 AM"
        );
        assert_eq!(
            all["started true Luna"].body,
            "Harbor weekend: Luna approves of Nov 14\n\
             Harbor weekend: snack time now, until 8:10 AM"
        );
        let (event, rule) = sst_box_event();
        let n = notification(&formatter(us(), Voice::Plain), &event, &rule, &names());
        assert_eq!(n.title, "Warm water off the Hudson Canyon: still met");
        assert_eq!(
            n.body,
            "Your area: Sea surface temperature 71 °F, over your 68 °F"
        );
        all.insert("box".into(), n);
        insta::assert_json_snapshot!("alert_notification_windows", all);
    }

    #[test]
    fn a_finding_the_rule_no_longer_has_still_reads() {
        let mut event = croton_event();
        event.findings[1].condition = 7;
        let n = notification(
            &formatter(us(), Voice::Plain),
            &event,
            &croton_rule(),
            &names(),
        );
        assert_eq!(
            n.body.lines().nth(1),
            Some("Croton River: your rule is met")
        );
    }

    #[test]
    fn rules_are_described_in_the_users_units_and_voice() {
        let mut rule = rule(
            "all",
            AlertTarget::Trip {
                id: "trip-1".into(),
            },
            vec![
                below(AlertMetric::Discharge, CROTON_LOW_M3S),
                C::Measurement {
                    metric: AlertMetric::WaterTemperature,
                    below: Some(COOL_C),
                    above: Some(10.0),
                },
                C::Measurement {
                    metric: AlertMetric::PressureTrend,
                    below: Some(-2.0),
                    above: None,
                },
                C::Measurement {
                    metric: AlertMetric::Gusts,
                    below: None,
                    above: Some(10.0),
                },
                C::MarineAlert {
                    event: "Small Craft Advisory".into(),
                },
                C::TripGo {},
                C::WindowStarts { within_minutes: 30 },
                C::StormAhead {},
            ],
        );
        rule.repeat_after_hours = Some(12);
        rule.quiet_hours = Some(QuietHours {
            start_minute: 22 * 60,
            end_minute: 7 * 60,
        });
        let d = describe_rule(&formatter(us(), Voice::Plain), &rule, &names());
        assert_eq!(d.target, "Harbor weekend");
        assert_eq!(
            d.conditions[..4],
            [
                "River flow under 800 cfs",
                "Water temperature between 50 °F and 66 °F",
                "Pressure change in 3 h under -0.06 inHg",
                "Gusts over 22 mph",
            ]
        );
        assert_eq!(d.conditions[4], "Small Craft Advisory in effect");
        assert_eq!(d.repeat.as_deref(), Some("Again every 12 h while met"));
        assert_eq!(d.quiet.as_deref(), Some("Quiet hours 10:00 PM to 7:00 AM"));
        let luna = describe_rule(&formatter(us(), Voice::Luna), &rule, &names());
        assert_eq!(luna.conditions[6], "A snack time within 30 min");
        // Safety conditions read the same in both voices.
        assert_eq!(luna.conditions[4], d.conditions[4]);
        assert_eq!(luna.conditions[7], d.conditions[7]);
        let mut all = BTreeMap::new();
        for (units, u) in [("us", us()), ("metric", metric())] {
            for voice in [Voice::Luna, Voice::Plain] {
                all.insert(
                    format!("{units} {voice:?}"),
                    describe_rule(&formatter(u, voice), &rule, &names()),
                );
            }
        }
        insta::assert_json_snapshot!("alert_rule_descriptions", all);
        // A target that is gone, and a box, are named without coordinates.
        let gone = describe_rule(
            &formatter(us(), Voice::Plain),
            &croton_rule(),
            &PlaceNames::default(),
        );
        assert_eq!(gone.target, "A deleted spot");
        let (_, sst) = sst_box_event();
        let d = describe_rule(&formatter(metric(), Voice::Plain), &sst, &names());
        assert_eq!(d.target, "An area on the map");
        assert_eq!(d.summary, "Sea surface temperature over 20.0 °C");
        assert_eq!(d.repeat, None);
        let none = AlertRule {
            all_of: Vec::new(),
            ..croton_rule()
        };
        assert_eq!(
            conditions_summary(&formatter(us(), Voice::Plain), &none),
            "No conditions"
        );
        assert_eq!(
            conditions_summary(&formatter(us(), Voice::Plain), &croton_rule()),
            "River flow under 800 cfs · Water temperature under 66 °F"
        );
    }

    #[test]
    fn quiet_hours_follow_the_24_hour_clock() {
        let rule = AlertRule {
            quiet_hours: Some(QuietHours {
                start_minute: 22 * 60 + 30,
                end_minute: 6 * 60 + 5,
            }),
            ..croton_rule()
        };
        let f = Formatter::new(metric(), Voice::Plain, TimeFormat::H24, "Europe/Berlin");
        assert_eq!(
            describe_rule(&f, &rule, &names()).quiet.as_deref(),
            Some("Quiet hours 22:30 to 06:05")
        );
    }

    #[test]
    fn thresholds_are_typed_in_the_users_units() {
        let fu = formatter(us(), Voice::Plain);
        let fm = formatter(metric(), Voice::Plain);
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-4 * b.abs().max(1.0), "{a} {b}");
        close(
            parse_threshold(&fu, AlertMetric::Discharge, "800")
                .unwrap()
                .unwrap(),
            CROTON_LOW_M3S,
        );
        close(
            parse_threshold(&fu, AlertMetric::WaterTemperature, " 66 ")
                .unwrap()
                .unwrap(),
            COOL_C,
        );
        close(
            parse_threshold(&fm, AlertMetric::WaterTemperature, "18.9")
                .unwrap()
                .unwrap(),
            18.9,
        );
        close(
            parse_threshold(&fu, AlertMetric::Gusts, "25")
                .unwrap()
                .unwrap(),
            25.0 * 0.447_04,
        );
        close(
            parse_threshold(&fm, AlertMetric::Gusts, "36")
                .unwrap()
                .unwrap(),
            10.0,
        );
        close(
            parse_threshold(&fm, AlertMetric::PressureTrend, "-2")
                .unwrap()
                .unwrap(),
            -2.0,
        );
        close(
            parse_threshold(&fu, AlertMetric::PressureTrend, "\u{2212}0.06")
                .unwrap()
                .unwrap(),
            -2.031_83,
        );
        close(
            parse_threshold(&fu, AlertMetric::TideHeight, "-1.5")
                .unwrap()
                .unwrap(),
            -0.4572,
        );
        close(
            parse_threshold(&fu, AlertMetric::Chlorophyll, "+2.5")
                .unwrap()
                .unwrap(),
            2.5,
        );
        assert_eq!(parse_threshold(&fu, AlertMetric::Discharge, "  "), Ok(None));
        // The noise of single precision is not kept.
        assert_eq!(
            parse_threshold(&fm, AlertMetric::WaveHeight, "1.2"),
            Ok(Some(1.2))
        );
        for bad in ["1,200", "12.5.1", "-", ".", "12 cfs", "1e3", "--3", "abc"] {
            assert_eq!(
                parse_threshold(&fu, AlertMetric::Discharge, bad),
                Err(ThresholdError::NotANumber),
                "{bad}"
            );
        }
        for out in ["-1", "4000000", "9".repeat(60).as_str()] {
            assert_eq!(
                parse_threshold(&fu, AlertMetric::Discharge, out),
                Err(ThresholdError::OutOfRange),
                "{out}"
            );
        }
        // The bounds are inside the range, typed in any unit.
        assert!(parse_threshold(&fu, AlertMetric::WaterTemperature, "113").is_ok());
        assert!(parse_threshold(&fu, AlertMetric::WaterTemperature, "23").is_ok());
        assert_eq!(
            parse_threshold(&fu, AlertMetric::WaterTemperature, "114"),
            Err(ThresholdError::OutOfRange)
        );
        // A comma-decimal locale takes a comma and refuses a point.
        let comma = Locale {
            numbers: NumberStyle::Comma,
            ..Locale::ENGLISH
        };
        let de = formatter(metric(), Voice::Plain).with_locale(comma);
        close(
            parse_threshold(&de, AlertMetric::WaveHeight, "1,5")
                .unwrap()
                .unwrap(),
            1.5,
        );
        assert_eq!(
            parse_threshold(&de, AlertMetric::WaveHeight, "1.5"),
            Err(ThresholdError::NotANumber)
        );
        assert_eq!(edit_threshold(&de, AlertMetric::WaveHeight, 1.5), "1,5");
    }

    #[test]
    fn threshold_fields_and_errors_read_in_the_users_units() {
        let fu = formatter(us(), Voice::Plain);
        let fm = formatter(metric(), Voice::Luna);
        assert_eq!(threshold_unit(&fu, AlertMetric::Discharge), "cfs");
        assert_eq!(threshold_unit(&fu, AlertMetric::GageHeight), "ft");
        assert_eq!(threshold_unit(&fm, AlertMetric::Gusts), "km/h");
        assert_eq!(threshold_unit(&fm, AlertMetric::PressureTrend), "hPa");
        assert_eq!(threshold_unit(&fu, AlertMetric::Chlorophyll), "µg/L");
        assert_eq!(
            edit_threshold(&fu, AlertMetric::Discharge, CROTON_LOW_M3S),
            "800"
        );
        assert_eq!(
            edit_threshold(&fu, AlertMetric::Discharge, 12.4 * f64::from(CFS)),
            "12.4"
        );
        assert_eq!(
            edit_threshold(&fu, AlertMetric::WaterTemperature, COOL_C),
            "66"
        );
        assert_eq!(
            edit_threshold(&fm, AlertMetric::WaterTemperature, COOL_C),
            "18.9"
        );
        assert_eq!(edit_threshold(&fm, AlertMetric::PressureTrend, -2.0), "-2");
        assert_eq!(
            edit_threshold(&fu, AlertMetric::PressureTrend, -2.0),
            "-0.06"
        );
        // What is typed comes back as it was typed.
        for (f, m, typed) in [
            (&fu, AlertMetric::Discharge, "800"),
            (&fu, AlertMetric::WaterTemperature, "66"),
            (&fu, AlertMetric::TideHeight, "5.5"),
            (&fm, AlertMetric::Wind, "18"),
            (&fm, AlertMetric::GageHeight, "1.25"),
        ] {
            let v = parse_threshold(f, m, typed).unwrap().unwrap();
            assert_eq!(edit_threshold(f, m, v), typed, "{m:?}");
        }
        assert_eq!(
            ThresholdError::NotANumber.text(&fu, AlertMetric::Discharge),
            "Enter a number, such as 12.5"
        );
        assert_eq!(
            ThresholdError::OutOfRange.text(&fu, AlertMetric::WaterTemperature),
            "Enter a value from 23 °F to 113 °F"
        );
        assert_eq!(
            ThresholdError::OutOfRange.text(&fm, AlertMetric::PressureTrend),
            "Enter a value from -50.0 hPa to 50.0 hPa"
        );
    }
}
