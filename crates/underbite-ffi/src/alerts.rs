//! Alert rules over the FFI: reading and writing them through the settings, the background
//! refresh's plan and fetches, their evaluation, and their words.
//!
//! The rules live in the settings (`[[alerts]]`); what each rule's last check found lives with
//! the app, as the bytes [`evaluate_alerts`] returns, passed back unchanged. A background
//! refresh has seconds, so it runs in three steps, none of which reads the clock:
//!
//! 1. [`plan_alerts`]: the steps the enabled rules need, most important first: one fetch per
//!    spot (a trip's rules name its places) asking only for the measurements its conditions
//!    read, and one ERDDAP request per gridded value a box rule watches.
//! 2. [`fetch_alert_spot`] for each spot step and the app's HTTP for each grid step, for as
//!    long as the time allows.
//! 3. [`evaluate_alerts`] with every spot's latest snapshot, the grid responses and the last
//!    state: the notifications due, worded, and the next state to keep.
//!
//! Coordinates leave the phone only rounded to `sources.coordinate_precision` (at most two
//! decimals): spot requests are rounded by the fetch, and a box is rounded outward. A
//! notification never carries coordinates, and names no place, rule or station when its rule
//! says not to. Every word comes from `underbite-core::view`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use underbite_config::export::to_json;
use underbite_config::schema::{Config, Severity};
use underbite_config::view_input::{river_ranges, trip_limits, window_params};
use underbite_core::alerts::{
    AlertMetric, AlertRule, AlertState, AlertTarget, BoxInputs, BoxValues, Condition, EvalInputs,
    FindingReason, RuleState, RuleStatus, SpotInputs, TripInputs, evaluate,
};
use underbite_core::map::Bbox;
use underbite_core::model::{Spot, SpotSnapshot, WaterType};
use underbite_core::offline::cache::merge_last;
use underbite_core::provenance::SourceId;
use underbite_core::time::{self, LocalDate, UnixTime, day_number};
use underbite_core::trip::gonogo::{DayInputs, DayVerdict, assess};
use underbite_core::trip::windows::{FishingWindow, Ranges, WindowInputs, compute};
use underbite_core::trip::{MAX_TRIP_DAYS, Trip};
use underbite_core::view::CardId;
use underbite_core::view::alerts::{
    self as words, PlaceNames, RuleInput, edit_threshold, parse_threshold, threshold_unit,
};
use underbite_core::view::cards::alerts::RuleStateKind;
use underbite_core::view::format::Formatter;
use underbite_fetch::store;
use underbite_providers::erddap::{self, GridTime};

use crate::fetch::{
    CachedResponse, FetchError, FetchHost, FetchOptions, HostRequest, HostResponse, Keys,
    SpotFetchOutcome, fetched,
};
use crate::map::formatter;
use crate::settings::{SettingsError, load};
use crate::view::SpotInfo;

/// The most cells a side of a box request asks for: a 1° box of a 0.01° grid is read every
/// second cell, some 2,500 values.
pub const MAX_BOX_CELLS_PER_SIDE: u32 = 50;

/// How long the app may reuse a box response when the server says nothing: the newest grid
/// time moves on daily.
const BOX_RESPONSE_AGE_S: u64 = 3_600;

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

/// Why an alerts call could not be done.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum AlertError {
    /// The settings could not be read or written.
    #[error("settings: {message}")]
    Settings {
        /// Why.
        message: String,
    },
    /// A rule, a metric, a trip or the alert state could not be read, or a rule cannot be
    /// saved as it is.
    #[error("invalid {what}: {message}")]
    Invalid {
        /// What was being read, or the path of the field at fault (`alerts[2].all_of`).
        what: String,
        /// Why.
        message: String,
    },
    /// A threshold typed in a field was refused; `message` is the field's message, in the
    /// user's units and voice.
    #[error("{message}")]
    Threshold {
        /// "Type a number, such as 12.5."
        message: String,
    },
}

impl From<SettingsError> for AlertError {
    fn from(e: SettingsError) -> Self {
        let SettingsError::Invalid { message } = e;
        Self::Settings { message }
    }
}

fn invalid(what: &str, message: &impl ToString) -> AlertError {
    AlertError::Invalid {
        what: what.to_owned(),
        message: message.to_string(),
    }
}

/// The alert state in `bytes`; none or empty is a first run.
fn read_state(bytes: Option<&[u8]>) -> Result<AlertState, AlertError> {
    match bytes.filter(|b| !b.is_empty()) {
        Some(b) => serde_json::from_slice(b).map_err(|e| invalid("alert state", &e)),
        None => Ok(AlertState::default()),
    }
}

fn metric(id: &str) -> Result<AlertMetric, AlertError> {
    serde_json::from_value(serde_json::Value::String(id.to_owned()))
        .map_err(|_| invalid("metric", &format!("`{id}` is not an alert metric")))
}

/// A serde enum's name (`"sea_surface_temp"`).
fn serde_name(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

fn water_type(text: &str) -> Option<WaterType> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
}

// ---------------------------------------------------------------------------------------------
// The rules: rows, words and editing
// ---------------------------------------------------------------------------------------------

/// A saved spot or trip by id, with the name the user gave it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct NamedPlace {
    /// The spot's or trip's id.
    pub id: String,
    /// Its name.
    pub name: String,
}

/// What a rule watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AlertTargetKind {
    /// A saved spot.
    Spot,
    /// A saved trip.
    Trip,
    /// An area on the map.
    Box,
}

/// How a rule's last check came out, for its styling; the words are beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AlertRuleState {
    /// Every condition holds.
    Met,
    /// A condition does not hold.
    NotMet,
    /// A condition's data was too old or too far, or its source failed.
    Unchecked,
    /// A condition had no data.
    NoData,
    /// Its spot or trip was deleted.
    Orphaned,
    /// Not checked since it was made.
    NotChecked,
    /// Turned off.
    Off,
}

impl From<RuleStateKind> for AlertRuleState {
    fn from(kind: RuleStateKind) -> Self {
        match kind {
            RuleStateKind::Met => Self::Met,
            RuleStateKind::NotMet => Self::NotMet,
            RuleStateKind::Unchecked => Self::Unchecked,
            RuleStateKind::NoData => Self::NoData,
            RuleStateKind::Orphaned => Self::Orphaned,
            RuleStateKind::NotChecked => Self::NotChecked,
            RuleStateKind::Off => Self::Off,
        }
    }
}

/// A rule in words, for the editor and the list.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RuleWords {
    /// What the rule watches: the spot's or trip's name, or "An area on the map" (never its
    /// coordinates).
    pub target: String,
    /// Each condition, in the rule's order: "River flow under 800 cfs".
    pub conditions: Vec<String>,
    /// The conditions on one line.
    pub summary: String,
    /// "Again every 12 h while met"; none when the rule waits for its conditions to stop.
    pub repeat: Option<String>,
    /// "Quiet hours 10:00 PM to 7:00 AM".
    pub quiet: Option<String>,
}

/// One rule of the list.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AlertRuleRow {
    /// The rule's id.
    pub id: String,
    /// The user's name for it.
    pub name: String,
    /// Whether it is checked at all.
    pub enabled: bool,
    /// What it watches.
    pub target: AlertTargetKind,
    /// The spot's or trip's id; none for a box.
    pub target_id: Option<String>,
    /// Its spot or trip no longer exists.
    pub orphaned: bool,
    /// The rule in words.
    pub words: RuleWords,
    /// How its last check came out.
    pub state: AlertRuleState,
    /// "Met since 6:10 AM", "River flow: couldn't check, data 5 h old".
    pub state_label: String,
    /// "Checked 6:10 AM"; none before the first check and when off.
    pub checked: Option<String>,
    /// The rule as the settings hold it (the core's `AlertRule`), for the editor.
    pub rule_json: String,
}

fn names(spots: &[NamedPlace], trips: &[NamedPlace]) -> PlaceNames {
    let by_id = |places: &[NamedPlace]| {
        places
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect()
    };
    PlaceNames {
        spots: by_id(spots),
        trips: by_id(trips),
    }
}

fn rule_words(f: &Formatter, rule: &AlertRule, names: &PlaceNames) -> RuleWords {
    let d = words::describe_rule(f, rule, names);
    RuleWords {
        target: d.target,
        conditions: d.conditions,
        summary: d.summary,
        repeat: d.repeat,
        quiet: d.quiet,
    }
}

/// The settings' rules for the list, in their order, each with its words in the settings'
/// units and voice and what its last check found (`state`, the bytes [`evaluate_alerts`] last
/// returned; none before the first run). A rule whose spot or trip is not among `spots` and
/// `trips` is orphaned. Times are written in `time_zone` (the phone's) at Unix time `now`.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn alert_rules(
    settings_json: String,
    spots: Vec<NamedPlace>,
    trips: Vec<NamedPlace>,
    state: Option<Vec<u8>>,
    time_zone: String,
    now: i64,
) -> Result<Vec<AlertRuleRow>, AlertError> {
    let (config, _) = load(&settings_json)?;
    let state = read_state(state.as_deref())?;
    let names = names(&spots, &trips);
    let f = formatter(&config, &time_zone);
    let now = UnixTime(now);
    config
        .alerts
        .iter()
        .map(|rule| {
            let orphaned = rule.is_orphaned(
                |id| names.spots.contains_key(id),
                |id| names.trips.contains_key(id),
            );
            let state = if orphaned && rule.enabled {
                Some(RuleState {
                    status: RuleStatus::Orphaned,
                    checked_at: now,
                    met_since: None,
                    notified_at: None,
                    pending: None,
                    marine: BTreeMap::new(),
                })
            } else {
                state.rules.get(&rule.id).cloned()
            };
            let row = words::rule_row(
                &f,
                &RuleInput {
                    rule: rule.clone(),
                    state,
                },
                now,
            );
            let (target, target_id) = match &rule.target {
                AlertTarget::Spot { id } => (AlertTargetKind::Spot, Some(id.clone())),
                AlertTarget::Trip { id } => (AlertTargetKind::Trip, Some(id.clone())),
                AlertTarget::Box { .. } => (AlertTargetKind::Box, None),
            };
            Ok(AlertRuleRow {
                id: rule.id.clone(),
                name: rule.name.clone(),
                enabled: rule.enabled,
                target,
                target_id,
                orphaned,
                words: rule_words(&f, rule, &names),
                state: row.state.into(),
                state_label: row.state_label,
                checked: if orphaned { None } else { row.checked },
                rule_json: serde_json::to_string(rule).map_err(|e| invalid("rule", &e))?,
            })
        })
        .collect()
}

fn read_rule(rule_json: &str) -> Result<AlertRule, AlertError> {
    serde_json::from_str(rule_json).map_err(|e| invalid("rule", &e))
}

/// A rule being edited (the core's `AlertRule` as JSON) in words, in the settings' units and
/// voice, naming its spot or trip from `spots` and `trips`.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn describe_alert_rule(
    rule_json: String,
    settings_json: String,
    spots: Vec<NamedPlace>,
    trips: Vec<NamedPlace>,
) -> Result<RuleWords, AlertError> {
    let (config, _) = load(&settings_json)?;
    let rule = read_rule(&rule_json)?;
    Ok(rule_words(
        &formatter(&config, "UTC"),
        &rule,
        &names(&spots, &trips),
    ))
}

/// The settings written back after an edit of their rules, refused when the rule at `index`
/// has an error (the first one, with its path).
fn save_rules(config: &Config, index: Option<usize>) -> Result<String, AlertError> {
    if let Some(i) = index {
        let prefix = format!("alerts[{i}]");
        let refused = config.validate().into_iter().find(|issue| {
            issue.severity == Severity::Error
                && (issue.path == prefix || issue.path.starts_with(&format!("{prefix}.")))
        });
        if let Some(issue) = refused {
            return Err(AlertError::Invalid {
                what: issue.path,
                message: issue.message,
            });
        }
    }
    to_json(config).map_err(|e| AlertError::Settings {
        message: e.to_string(),
    })
}

/// The settings with `rule_json` (the core's `AlertRule`) saved: in place of the rule with its
/// id, else after the others. A rule with an error (no name, no condition, a box over 1°, a
/// condition its target cannot have) is refused with the field's path; its spot or trip is not
/// looked up.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn put_alert_rule(settings_json: String, rule_json: String) -> Result<String, AlertError> {
    let (mut config, _) = load(&settings_json)?;
    let rule = read_rule(&rule_json)?;
    let index = if let Some(i) = config.alerts.iter().position(|r| r.id == rule.id) {
        config.alerts[i] = rule;
        i
    } else {
        config.alerts.push(rule);
        config.alerts.len() - 1
    };
    save_rules(&config, Some(index))
}

/// The settings without the rule `rule_id`; unchanged when there is none.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn remove_alert_rule(settings_json: String, rule_id: String) -> Result<String, AlertError> {
    let (mut config, _) = load(&settings_json)?;
    config.alerts.retain(|r| r.id != rule_id);
    save_rules(&config, None)
}

/// The settings with the rule `rule_id` turned on or off; unchanged when there is none.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn set_alert_rule_enabled(
    settings_json: String,
    rule_id: String,
    enabled: bool,
) -> Result<String, AlertError> {
    let (mut config, _) = load(&settings_json)?;
    for rule in config.alerts.iter_mut().filter(|r| r.id == rule_id) {
        rule.enabled = enabled;
    }
    save_rules(&config, None)
}

/// A value a measurement condition can watch, for the editor.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AlertMetricInfo {
    /// The metric as rules write it (`"discharge"`, `"sea_surface_temp"`, ...).
    pub metric: String,
    /// "River flow".
    pub title: String,
    /// The unit thresholds are typed in, from the settings: "cfs", "°F".
    pub unit: String,
    /// Whether a box rule can watch it (sea surface temperature and chlorophyll).
    pub in_boxes: bool,
}

/// Every metric a measurement condition can watch, with its title and the unit thresholds are
/// typed in under the settings.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn alert_metrics(settings_json: String) -> Result<Vec<AlertMetricInfo>, AlertError> {
    let (config, _) = load(&settings_json)?;
    let f = formatter(&config, "UTC");
    Ok(AlertMetric::ALL
        .into_iter()
        .map(|m| AlertMetricInfo {
            metric: serde_name(&m),
            title: f.strings.alert_metric(m).to_owned(),
            unit: threshold_unit(&f, m).to_owned(),
            in_boxes: m.in_boxes(),
        })
        .collect())
}

/// A threshold typed for `metric` in the unit [`alert_metrics`] names, as the canonical value
/// a rule holds; blank text is no threshold. A refused one comes back as
/// [`AlertError::Threshold`] with the field's message.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn parse_alert_threshold(
    metric: String,
    text: String,
    settings_json: String,
) -> Result<Option<f64>, AlertError> {
    let (config, _) = load(&settings_json)?;
    let m = self::metric(&metric)?;
    let f = formatter(&config, "UTC");
    parse_threshold(&f, m, &text).map_err(|e| AlertError::Threshold {
        message: e.text(&f, m),
    })
}

/// A rule's canonical threshold for `metric` as its field shows it, in the settings' unit:
/// "800", "18.9".
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn alert_threshold_text(
    metric: String,
    value: f64,
    settings_json: String,
) -> Result<String, AlertError> {
    let (config, _) = load(&settings_json)?;
    let m = self::metric(&metric)?;
    Ok(edit_threshold(&formatter(&config, "UTC"), m, value))
}

// ---------------------------------------------------------------------------------------------
// The dashboard's rules
// ---------------------------------------------------------------------------------------------

/// The rules the alerts card of `spot_id` lists, with their states: the spot's own and those
/// of the trips (their JSON) that include it, in the settings' order. Unreadable state bytes
/// or trips are passed over: the page still shows.
pub(crate) fn view_rules(
    config: &Config,
    spot_id: &str,
    trips_json: &[String],
    state: Option<&[u8]>,
) -> Vec<RuleInput> {
    if config.alerts.is_empty() {
        return Vec::new();
    }
    let state = read_state(state).unwrap_or_default();
    let trips: BTreeSet<String> = trips_json
        .iter()
        .filter_map(|json| serde_json::from_str::<Trip>(json).ok())
        .filter(|t| t.places.iter().any(|p| p.spot_id == spot_id))
        .map(|t| t.id)
        .collect();
    config
        .alerts
        .iter()
        .filter(|rule| match &rule.target {
            AlertTarget::Spot { id } => id == spot_id,
            AlertTarget::Trip { id } => trips.contains(id),
            AlertTarget::Box { .. } => false,
        })
        .map(|rule| RuleInput {
            rule: rule.clone(),
            state: state.rules.get(&rule.id).cloned(),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------------------------

/// A spot to fetch for the rules that watch it, with [`fetch_alert_spot`].
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AlertFetch {
    /// The spot as the app holds it. Its coordinates stay on the phone: requests carry them
    /// rounded to the settings' precision.
    pub spot: SpotInfo,
    /// The ids of the rules that read it, in the settings' order.
    pub rules: Vec<String>,
    /// What the fetch asks for (`"discharge"`, `"marine_alerts"`, ...); the weather and the
    /// storm watch come with every fetch.
    pub measurements: Vec<String>,
    /// 0 for storms and marine alerts, 1 for measurements, 2 for fishing windows, 3 for a
    /// trip's go/no-go: the most important rule it serves.
    pub priority: u32,
}

/// One ERDDAP request for a box rule: send it (pacing its `node` as the map does) and hand
/// the response to [`evaluate_alerts`] under its key.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AlertGridRequest {
    /// Hand the response back under it.
    pub key: String,
    /// The rule's id.
    pub rule: String,
    /// `"sea_surface_temp"` or `"chlorophyll"`.
    pub metric: String,
    /// The server, for the map's pacer: `pfeg` or `coastwatch`.
    pub node: String,
    /// The request: the newest grid over the box rounded outward.
    pub request: HostRequest,
    /// As [`AlertFetch::priority`].
    pub priority: u32,
}

/// One step of a background check.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum AlertStep {
    /// Fetch a spot.
    Spot {
        /// What to fetch.
        fetch: AlertFetch,
    },
    /// Send one grid request.
    Grid {
        /// What to send.
        grid: AlertGridRequest,
    },
}

/// How much a rule matters when time runs short: safety first, a trip's verdict last.
fn rule_priority(rule: &AlertRule) -> u32 {
    rule.all_of
        .iter()
        .map(|c| match c {
            Condition::StormAhead {} | Condition::MarineAlert { .. } => 0,
            Condition::Measurement { .. } => 1,
            Condition::WindowStarts { .. } => 2,
            Condition::TripGo {} => 3,
        })
        .min()
        .unwrap_or(4)
}

/// The cards whose data a condition reads: a fetch asks for their measurements on the spot's
/// water, as the dashboard does. Storms and the weather come with every fetch.
fn condition_cards(condition: &Condition) -> &'static [CardId] {
    use CardId as C;
    match condition {
        Condition::Measurement { metric, .. } => match metric {
            AlertMetric::Discharge | AlertMetric::GageHeight => &[C::Flow],
            AlertMetric::WaterTemperature | AlertMetric::SeaSurfaceTemp => &[C::WaterTemp],
            AlertMetric::WaveHeight => &[C::Waves],
            AlertMetric::Wind | AlertMetric::Gusts => &[C::Wind],
            AlertMetric::PressureTrend => &[C::Pressure],
            AlertMetric::TideHeight => &[C::Tide],
            // Gridded only: no spot fetch has it.
            AlertMetric::Chlorophyll => &[],
        },
        Condition::MarineAlert { .. } => &[C::Alerts],
        // What go/no-go judges: wind, waves, currents, river flow and the marine alerts.
        Condition::TripGo {} => &[C::GoNoGo, C::Waves, C::Current, C::Flow],
        // What the windows weigh: tides, currents, pressure, water temperature and flow.
        Condition::WindowStarts { .. } => &[
            C::Windows,
            C::Tide,
            C::Current,
            C::Pressure,
            C::WaterTemp,
            C::Flow,
        ],
        Condition::StormAhead {} => &[],
    }
}

/// The settings a fetch for `rules` runs with: the user's, with only the cards the rules'
/// conditions read, in the catalogue's order, on every water type.
fn narrowed<'a>(config: &Config, rules: impl IntoIterator<Item = &'a AlertRule>) -> Config {
    let wanted: BTreeSet<CardId> = rules
        .into_iter()
        .flat_map(|r| r.all_of.iter())
        .flat_map(|c| condition_cards(c).iter().copied())
        .collect();
    let mut out = config.clone();
    let cards = &mut out.cards;
    cards.order = CardId::ALL
        .into_iter()
        .filter(|c| wanted.contains(c))
        .collect();
    cards.hidden.clear();
    for layout in [
        &mut cards.river,
        &mut cards.lake,
        &mut cards.tidal,
        &mut cards.bay,
        &mut cards.surf,
        &mut cards.nearshore,
        &mut cards.offshore,
        &mut cards.great_lakes,
        &mut cards.pond,
        &mut cards.ice,
    ] {
        *layout = None;
    }
    out
}

/// The measurements a fetch with `config` asks for on `water`, by name.
fn measurement_names(config: &Config, water: WaterType) -> Vec<String> {
    underbite_fetch::requirements(config, water)
        .measurements
        .iter()
        .map(serde_name)
        .collect()
}

/// The box rounded outward to `decimals`, inside the globe.
fn rounded_box(area: Bbox, decimals: u8) -> Bbox {
    let scale = 10f64.powi(i32::from(decimals));
    let down = |v: f64| (v * scale).floor() / scale;
    let up = |v: f64| (v * scale).ceil() / scale;
    Bbox {
        west: down(area.west).max(-180.0),
        south: down(area.south).max(-90.0),
        east: up(area.east).min(180.0),
        north: up(area.north).min(90.0),
    }
}

/// The grid layer a box metric is read from.
fn box_layer(metric: AlertMetric) -> Option<&'static str> {
    match metric {
        AlertMetric::SeaSurfaceTemp => Some("sst"),
        AlertMetric::Chlorophyll => Some("chlorophyll"),
        _ => None,
    }
}

fn grid_source(metric: AlertMetric) -> Option<underbite_core::map::GridSource> {
    let layer = crate::map::layer(box_layer(metric)?).ok()?;
    Some(layer.grid()?.primary.source())
}

fn bbox_of(target: &AlertTarget) -> Option<Bbox> {
    match *target {
        AlertTarget::Box {
            south,
            west,
            north,
            east,
        } => Some(Bbox {
            west,
            south,
            east,
            north,
        }),
        _ => None,
    }
}

/// The key a box rule's grid response comes back under.
fn grid_key(rule: &str, metric: AlertMetric) -> String {
    format!("{rule}/{}", serde_name(&metric))
}

/// The grid requests of a box rule: one per gridded metric its conditions watch.
fn grid_requests(rule: &AlertRule, decimals: u8, priority: u32) -> Vec<AlertGridRequest> {
    let Some(area) = bbox_of(&rule.target) else {
        return Vec::new();
    };
    let metrics: BTreeSet<AlertMetric> = rule
        .all_of
        .iter()
        .filter_map(|c| match c {
            Condition::Measurement { metric, .. } if metric.in_boxes() => Some(*metric),
            _ => None,
        })
        .collect();
    let area = rounded_box(area, decimals);
    metrics
        .into_iter()
        .filter_map(|metric| {
            let source = grid_source(metric)?;
            let side = (area.north - area.south).max(area.east - area.west);
            let cells = (side / source.native_step_deg).ceil().max(1.0) as u32;
            let stride = cells.div_ceil(MAX_BOX_CELLS_PER_SIDE).max(1);
            let request = erddap::grid_request(&source, GridTime::Last, area, stride).ok()?;
            Some(AlertGridRequest {
                key: grid_key(&rule.id, metric),
                rule: rule.id.clone(),
                metric: serde_name(&metric),
                node: crate::map::node_name(request.node),
                request: crate::map::get(request.url, BOX_RESPONSE_AGE_S),
                priority,
            })
        })
        .collect()
}

/// What one spot of the plan gathers.
struct Gathered {
    spot: SpotInfo,
    rules: Vec<String>,
    priority: u32,
    /// The place in the settings of the first rule of its priority: the tie-break.
    first: usize,
}

/// The steps a background check takes for the enabled rules at Unix time `now`, most
/// important first (safety, then measurements, fishing windows and trips' go/no-go; then the
/// settings' order), so that a check cut short has done what matters most:
///
/// - one fetch per spot a rule watches, shared by its rules, asking only for the measurements
///   their conditions read (see [`AlertFetch`]); a trip's rules fetch its places while a
///   forecast can reach one of its days not over yet;
/// - one ERDDAP request per gridded value a box rule watches, the box rounded outward to the
///   settings' precision and read at most [`MAX_BOX_CELLS_PER_SIDE`] cells a side.
///
/// `spots` are the saved spots and `trips` the saved trips (their JSON); a rule whose spot or
/// trip is not among them has nothing to fetch, and an unreadable trip is passed over.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn plan_alerts(
    settings_json: String,
    spots: Vec<SpotInfo>,
    trips: Vec<String>,
    now: i64,
) -> Result<Vec<AlertStep>, AlertError> {
    let (config, _) = load(&settings_json)?;
    let decimals = config
        .sources
        .coordinate_precision
        .min(underbite_config::sources::MAX_COORDINATE_PRECISION);
    let saved: BTreeMap<&str, &SpotInfo> = spots.iter().map(|s| (s.id.as_str(), s)).collect();
    let trips: BTreeMap<String, Vec<SpotInfo>> = trips
        .iter()
        .filter_map(|json| {
            let trip: Trip = serde_json::from_str(json).ok()?;
            let places = crate::trip::trip_spots(json.clone(), now).ok()?;
            let reached = places
                .into_iter()
                .filter(|p| p.forecast)
                .filter_map(|p| match saved.get(p.key.as_str()) {
                    Some(s) => Some((*s).clone()),
                    None => p.spot,
                })
                .collect();
            Some((trip.id, reached))
        })
        .collect();
    let mut gathered: BTreeMap<String, Gathered> = BTreeMap::new();
    let mut steps = Vec::new();
    for (index, rule) in config.alerts.iter().enumerate().filter(|(_, r)| r.enabled) {
        let priority = rule_priority(rule);
        let places: Vec<SpotInfo> = match &rule.target {
            AlertTarget::Spot { id } => saved
                .get(id.as_str())
                .map(|s| (*s).clone())
                .into_iter()
                .collect(),
            AlertTarget::Trip { id } => trips.get(id).cloned().unwrap_or_default(),
            AlertTarget::Box { .. } => {
                steps.extend(
                    grid_requests(rule, decimals, priority)
                        .into_iter()
                        .map(|grid| (priority, index, AlertStep::Grid { grid })),
                );
                Vec::new()
            }
        };
        for spot in places {
            let entry = gathered.entry(spot.id.clone()).or_insert_with(|| Gathered {
                spot,
                rules: Vec::new(),
                priority,
                first: index,
            });
            if !entry.rules.contains(&rule.id) {
                entry.rules.push(rule.id.clone());
            }
            if priority < entry.priority {
                entry.priority = priority;
                entry.first = index;
            }
        }
    }
    for g in gathered.into_values() {
        let rules: Vec<&AlertRule> = config
            .alerts
            .iter()
            .filter(|r| g.rules.contains(&r.id))
            .collect();
        let measurements = water_type(&g.spot.water_type)
            .map(|w| measurement_names(&narrowed(&config, rules), w))
            .unwrap_or_default();
        steps.push((
            g.priority,
            g.first,
            AlertStep::Spot {
                fetch: AlertFetch {
                    spot: g.spot,
                    rules: g.rules,
                    measurements,
                    priority: g.priority,
                },
            },
        ));
    }
    // Stable: grid requests of one rule keep their order, after the spot fetches of their rank.
    steps.sort_by_key(|(priority, first, step)| {
        (*priority, *first, matches!(step, AlertStep::Grid { .. }))
    });
    Ok(steps.into_iter().map(|(_, _, step)| step).collect())
}

// ---------------------------------------------------------------------------------------------
// The fetch
// ---------------------------------------------------------------------------------------------

/// The spot's last snapshot as the fetch stored it.
fn stored_snapshot(host: &Arc<dyn FetchHost>, spot_id: &str) -> Option<SpotSnapshot> {
    let text = host.load(store::SPOT.to_owned(), store::spot_key(spot_id))?;
    let stored: store::Stored<SpotSnapshot> = serde_json::from_str(&text).ok()?;
    Some(stored.value)
}

/// Fetches one spot of an [`AlertFetch`] at Unix time `now` through `host`, as `fetch_spot`
/// does but asking only for what its rules read (the weather and the storm watch included),
/// without inferring the water type. The values it does not ask for come from the spot's last
/// stored snapshot with their own times, so the snapshot kept and returned stays whole for the
/// dashboard; the fresh values replace the old ones. The spot's station rankings are left as
/// the last full fetch kept them, so the station choices still list every measurement.
/// Requests carry the spot's coordinates rounded to the settings' precision.
/// `cached_metadata` is as for `fetch_spot`.
#[uniffi::export(default(cached_metadata = []))]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub async fn fetch_alert_spot(
    fetch: AlertFetch,
    settings_json: String,
    keys: Keys,
    allow_metered: bool,
    now: i64,
    cached_metadata: Vec<CachedResponse>,
    host: Arc<dyn FetchHost>,
) -> Result<SpotFetchOutcome, FetchError> {
    let (config, _) = load(&settings_json)?;
    let rules = config.alerts.iter().filter(|r| fetch.rules.contains(&r.id));
    let settings = to_json(&narrowed(&config, rules)).map_err(|e| FetchError::Settings {
        message: e.to_string(),
    })?;
    let key = store::spot_key(&fetch.spot.id);
    let previous = stored_snapshot(&host, &fetch.spot.id);
    let rankings = host.load(store::DISCOVERY.to_owned(), key.clone());
    let options = FetchOptions {
        allow_metered,
        infer_water_type: false,
    };
    let out = Box::pin(crate::fetch::fetch_spot(
        fetch.spot.clone(),
        settings,
        keys,
        options,
        now,
        cached_metadata,
        Arc::clone(&host),
    ))
    .await;
    // The fetch ranked the stations of its few measurements only: the full ranking stays.
    host.store(
        store::DISCOVERY.to_owned(),
        key.clone(),
        rankings.unwrap_or_else(|| store::FORGOTTEN.to_owned()),
    );
    let mut out = out?;
    if let Some(last) = previous {
        let fresh: SpotSnapshot =
            serde_json::from_str(&out.snapshot_json).map_err(|e| FetchError::Invalid {
                what: "snapshot".into(),
                message: e.to_string(),
            })?;
        let merged = merge_last(&last, fresh);
        let json = serde_json::to_string(&merged).map_err(|e| FetchError::Invalid {
            what: "snapshot".into(),
            message: e.to_string(),
        })?;
        if let Ok(stored) = serde_json::to_string(&store::Stored {
            saved_at: now,
            value: &merged,
        }) {
            host.store(store::SPOT.to_owned(), key, stored);
        }
        out.snapshot_json = json;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------------------------

/// A saved spot with its latest snapshot, for [`evaluate_alerts`].
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AlertSpotData {
    /// The spot as the app holds it (its name and water type win over the snapshot's).
    pub spot: SpotInfo,
    /// Its latest snapshot (`fetch_spot`'s or `fetch_alert_spot`'s `snapshot_json`), if any.
    pub snapshot_json: Option<String>,
}

/// The response to an [`AlertGridRequest`]. A request with no response is left out.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AlertGridResponse {
    /// The request's key.
    pub key: String,
    /// What came back, any status.
    pub response: HostResponse,
}

/// One notification to post, worded by the core.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AlertNotice {
    /// The rule's id: one notification per rule, a newer one replacing it.
    pub rule_id: String,
    /// The title: the rule's name, or "Your spot" when it hides names.
    pub title: String,
    /// What met the rule, one line per finding; never coordinates.
    pub body: String,
    /// A storm or a hurricane: post it time-sensitive.
    pub time_sensitive: bool,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct AlertRun {
    /// The notifications due now, in the settings' order.
    pub notifications: Vec<AlertNotice>,
    /// The state to keep and pass to the next check, and to `alert_rules` and `view_model`.
    pub state: Vec<u8>,
}

/// A spot as the core holds it, from the app's.
fn core_spot(info: &SpotInfo) -> Option<Spot> {
    Some(Spot {
        id: info.id.clone(),
        name: info.name.clone(),
        latitude: info.latitude,
        longitude: info.longitude,
        water_type: water_type(&info.water_type)?,
        water_body: None,
        time_zone: info.time_zone.clone(),
    })
}

/// The spot's snapshot with the spot as the app has it, or an empty one when it has none (or
/// none that can be read): every rule on it then has no data.
fn spot_snapshot(data: &AlertSpotData) -> Option<SpotSnapshot> {
    let spot = core_spot(&data.spot)?;
    let read = data
        .snapshot_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<SpotSnapshot>(json).ok());
    Some(match read {
        Some(mut snapshot) => {
            snapshot.spot.name = spot.name;
            snapshot.spot.water_type = spot.water_type;
            if !spot.time_zone.is_empty() {
                snapshot.spot.time_zone = spot.time_zone;
            }
            snapshot
        }
        None => SpotSnapshot::new(spot, UnixTime(0)),
    })
}

/// The fishing windows of the local days `first` to `last` that end after `now`, with the
/// settings and the spot's best river flow.
fn windows_between(
    config: &Config,
    snapshot: &SpotSnapshot,
    first: LocalDate,
    last: LocalDate,
    now: UnixTime,
) -> Vec<FishingWindow> {
    let flow = river_ranges(&config.rivers, snapshot)
        .and_then(|r| Some([f64::from(r.best_min_m3s?), f64::from(r.best_max_m3s?)]));
    let ranges = Ranges {
        water_temp_c: None,
        flow_m3s: flow,
    };
    let inputs = WindowInputs::from_snapshot(snapshot, first, last, ranges);
    compute(&inputs, &window_params(&config.windows), now)
}

/// A spot's windows today and tomorrow, local to it.
fn spot_windows(config: &Config, snapshot: &SpotSnapshot, now: UnixTime) -> Vec<FishingWindow> {
    let Ok(tz) = time::zone(&snapshot.spot.time_zone) else {
        return Vec::new();
    };
    let Ok(today) = time::local_date(now, &tz) else {
        return Vec::new();
    };
    let tomorrow = time::local_date_bounds(today, &tz)
        .ok()
        .and_then(|(_, end)| time::local_date(end, &tz).ok())
        .unwrap_or(today);
    windows_between(config, snapshot, today, tomorrow, now)
}

/// The dates from `first` to `last`; none when reversed or longer than a trip may be.
fn dates(first: LocalDate, last: LocalDate) -> Vec<LocalDate> {
    let (a, b) = (day_number(first), day_number(last));
    if b < a || b - a >= MAX_TRIP_DAYS {
        return Vec::new();
    }
    (a..=b)
        .map(|n| {
            let (year, month, day) = whether_core::time::civil_from_days(n);
            LocalDate { year, month, day }
        })
        .collect()
}

/// What a trip's rules read: its places, its zone, and, when a rule asks, the go/no-go of
/// each day at each place with a snapshot and the windows of its days still to come.
fn trip_inputs(
    config: &Config,
    trip: &Trip,
    snapshots: &BTreeMap<String, SpotSnapshot>,
    days: bool,
    windows: bool,
    now: UnixTime,
) -> TripInputs {
    let limits = trip
        .limits
        .clone()
        .or_else(|| config.limits.first().map(trip_limits))
        .unwrap_or_default();
    let mut verdicts: Vec<DayVerdict> = Vec::new();
    let mut found: Vec<FishingWindow> = Vec::new();
    let dates = dates(trip.first, trip.last);
    for place in &trip.places {
        let Some(snapshot) = snapshots.get(&place.spot_id) else {
            continue;
        };
        let river = river_ranges(&config.rivers, snapshot).unwrap_or_default();
        if days {
            verdicts.extend(
                dates
                    .iter()
                    .map(|&d| assess(d, &DayInputs::new(snapshot).with_river(river), &limits, now)),
            );
        }
        let today = time::zone(&snapshot.spot.time_zone)
            .ok()
            .and_then(|tz| time::local_date(now, &tz).ok());
        if windows && let Some(today) = today {
            let first = if day_number(today) > day_number(trip.first) {
                today
            } else {
                trip.first
            };
            if day_number(first) <= day_number(trip.last) {
                found.extend(windows_between(config, snapshot, first, trip.last, now));
            }
        }
    }
    found.sort_by_key(|w| (w.start, w.end));
    TripInputs {
        places: trip.places.iter().map(|p| p.spot_id.clone()).collect(),
        time_zone: trip
            .places
            .first()
            .map(|p| p.time_zone.clone())
            .unwrap_or_default(),
        days: verdicts,
        windows: found,
    }
}

/// The finite values of a grid response inside a box rule's box, or none when the response
/// is not a grid.
fn box_values(rule: &AlertRule, metric: AlertMetric, response: HostResponse) -> Option<BoxValues> {
    let area = bbox_of(&rule.target)?;
    let source = grid_source(metric)?;
    let fetched = fetched(response).ok()?;
    let body = erddap::check_response(&fetched.as_response())
        .ok()?
        .to_vec();
    let grid = erddap::parse_grid(&source, &body).ok()?;
    Some(BoxValues {
        source: SourceId::new(source.dataset.clone()),
        time: grid.time,
        cells: grid.finite_values_in(area),
    })
}

fn time_sensitive(reason: FindingReason) -> bool {
    matches!(reason, FindingReason::StormAhead | FindingReason::Hurricane)
}

/// Checks the enabled rules at Unix time `now` and words the notifications due.
///
/// - `spots`: every saved spot (and every trip place fetched for a rule) with its latest
///   snapshot; a spot without one has no data, and a rule whose spot is missing is orphaned.
/// - `trips`: the saved trips (their JSON); an unreadable one is passed over.
/// - `grids`: the responses to the plan's grid requests that came back.
/// - `state`: the bytes the last check returned, none for the first; unreadable bytes are an
///   error (pass none to start over, which may notify again what was told).
/// - `time_zone`: the phone's IANA zone, for a box rule's quiet hours and its notification's
///   times; a spot's or trip's own zone is used for theirs.
///
/// A rule fires on fresh data only, once, and again after it has stopped holding or after its
/// repeat interval; quiet hours hold a notification until they end. The words are in the
/// settings' units and voice; storms, hurricanes and marine alerts are always plain.
#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI passes arguments by value
pub fn evaluate_alerts(
    settings_json: String,
    spots: Vec<AlertSpotData>,
    trips: Vec<String>,
    grids: Vec<AlertGridResponse>,
    state: Option<Vec<u8>>,
    time_zone: String,
    now: i64,
) -> Result<AlertRun, AlertError> {
    let (config, _) = load(&settings_json)?;
    let state = read_state(state.as_deref())?;
    let now = UnixTime(now);
    let rules = &config.alerts;
    let (inputs, names) = eval_inputs(&config, &spots, &trips, grids, &time_zone, now);
    let (events, next) = evaluate(rules, &inputs, &state, now);
    let notifications = events
        .iter()
        .filter_map(|event| {
            let rule = rules.iter().find(|r| r.id == event.rule_id)?;
            let zone = match &rule.target {
                AlertTarget::Spot { id } => inputs
                    .spots
                    .get(id)
                    .map_or(time_zone.as_str(), |s| s.snapshot.spot.time_zone.as_str()),
                AlertTarget::Trip { id } => inputs
                    .trips
                    .get(id)
                    .map_or(time_zone.as_str(), |t| t.time_zone.as_str()),
                AlertTarget::Box { .. } => time_zone.as_str(),
            };
            let n = words::notification(&formatter(&config, zone), event, rule, &names);
            Some(AlertNotice {
                rule_id: event.rule_id.clone(),
                title: n.title,
                body: n.body,
                time_sensitive: event.findings.iter().any(|f| time_sensitive(f.reason)),
            })
        })
        .collect();
    Ok(AlertRun {
        notifications,
        state: serde_json::to_vec(&next).map_err(|e| invalid("alert state", &e))?,
    })
}

/// What evaluation reads, and the names notifications may use: the spots with their
/// snapshots (and their windows when a rule asks), the trips (with their days' go/no-go and
/// windows when a rule asks), and the boxes with the values of the grid responses.
fn eval_inputs(
    config: &Config,
    spots: &[AlertSpotData],
    trips: &[String],
    grids: Vec<AlertGridResponse>,
    time_zone: &str,
    now: UnixTime,
) -> (EvalInputs, PlaceNames) {
    let rules = &config.alerts;
    let enabled = || rules.iter().filter(|r| r.enabled);
    let asks = |target: &dyn Fn(&AlertTarget) -> bool, wanted: fn(&Condition) -> bool| {
        enabled().any(|r| target(&r.target) && r.all_of.iter().any(wanted))
    };
    let is_window = |c: &Condition| matches!(c, Condition::WindowStarts { .. });
    let is_go = |c: &Condition| matches!(c, Condition::TripGo {});

    let mut names = PlaceNames::default();
    let mut snapshots: BTreeMap<String, SpotSnapshot> = BTreeMap::new();
    for data in spots {
        names
            .spots
            .insert(data.spot.id.clone(), data.spot.name.clone());
        if let Some(snapshot) = spot_snapshot(data) {
            snapshots.insert(data.spot.id.clone(), snapshot);
        }
    }
    let trips: Vec<Trip> = trips
        .iter()
        .filter_map(|json| serde_json::from_str(json).ok())
        .collect();

    let mut inputs = EvalInputs {
        limits: underbite_config::view_input::measurement_limits(&config.sources),
        ..EvalInputs::default()
    };
    for (id, snapshot) in &snapshots {
        let here = |t: &AlertTarget| matches!(t, AlertTarget::Spot { id: s } if s == id);
        let windows = if asks(&here, is_window) {
            spot_windows(config, snapshot, now)
        } else {
            Vec::new()
        };
        inputs.spots.insert(
            id.clone(),
            SpotInputs {
                snapshot: snapshot.clone(),
                windows,
            },
        );
    }
    for trip in &trips {
        names.trips.insert(trip.id.clone(), trip.name.clone());
        let this = |t: &AlertTarget| matches!(t, AlertTarget::Trip { id } if *id == trip.id);
        let days = asks(&this, is_go);
        let windows = asks(&this, is_window);
        inputs.trips.insert(
            trip.id.clone(),
            trip_inputs(config, trip, &snapshots, days, windows, now),
        );
    }
    for rule in enabled().filter(|r| bbox_of(&r.target).is_some()) {
        inputs.boxes.insert(
            rule.id.clone(),
            BoxInputs {
                time_zone: time_zone.to_owned(),
                values: BTreeMap::new(),
            },
        );
    }
    for g in grids {
        let Some((rule_id, metric_id)) = g.key.rsplit_once('/') else {
            continue;
        };
        let (Some(rule), Ok(metric)) = (
            enabled().find(|r| r.id == rule_id && bbox_of(&r.target).is_some()),
            self::metric(metric_id),
        ) else {
            continue;
        };
        if let (Some(values), Some(entry)) = (
            box_values(rule, metric, g.response),
            inputs.boxes.get_mut(rule_id),
        ) {
            entry.values.insert(metric, values);
        }
    }
    (inputs, names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use underbite_core::alerts::QuietHours;

    fn rule(id: &str, target: AlertTarget, all_of: Vec<Condition>) -> AlertRule {
        AlertRule {
            id: id.into(),
            name: id.into(),
            enabled: true,
            target,
            all_of,
            repeat_after_hours: None,
            quiet_hours: None::<QuietHours>,
            show_name: true,
        }
    }

    fn flow_below(threshold: f64) -> Condition {
        Condition::Measurement {
            metric: AlertMetric::Discharge,
            below: Some(threshold),
            above: None,
        }
    }

    fn spot(id: &str) -> AlertTarget {
        AlertTarget::Spot { id: id.into() }
    }

    /// The narrowed settings survive the trip through JSON that the fetch takes them on.
    fn reloaded(config: &Config) -> Config {
        crate::settings::load(&to_json(config).unwrap()).unwrap().0
    }

    #[test]
    fn a_flow_rule_asks_only_for_the_flow_on_a_river() {
        let rules = [rule("r", spot("croton"), vec![flow_below(20.0)])];
        let config = reloaded(&narrowed(&Config::default(), &rules));
        assert_eq!(
            measurement_names(&config, WaterType::RiverStream),
            [
                "discharge",
                "gage_height",
                "flood_categories",
                "flow_forecast"
            ]
        );
    }

    #[test]
    fn a_storm_rule_asks_for_no_measurement_inland_and_the_marine_safety_ones_on_salt() {
        let rules = [rule("r", spot("s"), vec![Condition::StormAhead {}])];
        let config = reloaded(&narrowed(&Config::default(), &rules));
        assert_eq!(
            measurement_names(&config, WaterType::RiverStream),
            Vec::<String>::new()
        );
        assert_eq!(
            measurement_names(&config, WaterType::BayInshore),
            ["marine_forecast", "marine_alerts"]
        );
    }

    #[test]
    fn hidden_cards_and_layouts_do_not_hide_what_a_rule_reads() {
        let mut config = Config::default();
        config.cards.hidden.push(CardId::Flow);
        let rules = [rule("r", spot("croton"), vec![flow_below(20.0)])];
        let narrowed = reloaded(&narrowed(&config, &rules));
        assert!(measurement_names(&narrowed, WaterType::RiverStream).contains(&"discharge".into()));
        // Fewer than the whole dashboard asks for.
        assert!(
            measurement_names(&narrowed, WaterType::RiverStream).len()
                < measurement_names(&Config::default(), WaterType::RiverStream).len()
        );
    }

    #[test]
    fn a_box_is_rounded_outward() {
        let b = rounded_box(
            Bbox {
                west: -73.987,
                south: 40.123,
                east: -73.501,
                north: 40.991,
            },
            1,
        );
        assert_eq!(
            (b.west, b.south, b.east, b.north),
            (-74.0, 40.1, -73.5, 41.0)
        );
    }

    #[test]
    fn safety_comes_first_and_a_trips_verdict_last() {
        let storm = rule("a", spot("s"), vec![Condition::StormAhead {}]);
        let go = rule("b", spot("s"), vec![Condition::TripGo {}]);
        let both = rule(
            "c",
            spot("s"),
            vec![flow_below(1.0), Condition::StormAhead {}],
        );
        assert_eq!(rule_priority(&storm), 0);
        assert_eq!(rule_priority(&go), 3);
        assert_eq!(rule_priority(&both), 0);
    }

    #[test]
    fn unreadable_state_is_refused_and_none_is_a_first_run() {
        assert_eq!(read_state(None).unwrap(), AlertState::default());
        assert_eq!(read_state(Some(&[])).unwrap(), AlertState::default());
        assert!(matches!(
            read_state(Some(b"not json")),
            Err(AlertError::Invalid { .. })
        ));
    }
}
