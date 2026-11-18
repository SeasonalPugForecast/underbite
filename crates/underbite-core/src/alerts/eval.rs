//! Rule evaluation: which rules fire for a snapshot, and when they may fire again.
//!
//! [`evaluate`] judges every enabled rule against what the caller gathered ([`EvalInputs`]) and
//! the state the last run left ([`AlertState`]), and returns the notifications due now with the
//! next state. Nothing here performs I/O or reads the clock.
//!
//! - A rule is met when every condition holds on fresh data. It notifies once, then stays quiet
//!   while it remains met; it re-arms when a condition is false beyond a hysteresis (2 % of a
//!   flow or chlorophyll threshold, 0.3 °C, 0.03 m of stage or tide, 0.1 m of waves, 1 kn of
//!   wind, 0.3 hPa of pressure change), or notifies again after `repeat_after_hours`.
//! - Background runs are hours apart, so a crossing is judged from the latest value only.
//! - A value older than the age limit, or from a station beyond the distance limit, never makes
//!   a rule fire: the rule is "couldn't check" and keeps its latch. Missing data never fires
//!   either, and a condition that cannot be checked never re-arms a rule.
//! - Marine alerts notify as whether's `AlertNotifier` does: a new alert, a stronger one, or one
//!   that now ends an hour or more later than the one told. An alert stays remembered until it
//!   ends, so one that drops out of a feed for a run is not told again.
//! - Quiet hours, in the target's time zone, hold a notification until they end; it is never
//!   dropped.
//!
//! Events carry values in canonical units and no words: the view words them.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whether_core::storm::analysis::{SIGNIFICANT_HORIZON_SECS, significant_event};
use whether_core::storm::hurricane::hurricane_window;

use super::rules::{AlertMetric, AlertRule, AlertTarget, Condition, QuietHours};
use crate::discover::rank::DEFAULT_MAX_AGE_S as STATION_MAX_AGE_S;
use crate::model::{AlertSeverity, MarineAlert, Measurement, SpotSnapshot, TideSourceKind};
use crate::provenance::{Provenance, Relation, SourceId, StationRef, ValueKind};
use crate::series::{Sample, Series};
use crate::time::{self, UnixTime};
use crate::trip::gonogo::{DayVerdict, Verdict};
use crate::trip::windows::FishingWindow;
use crate::units::SpeedUnit;
use crate::view::MeasurementLimit;

const HOUR: i64 = 3_600;
/// Station wind, pressure and wave observations count this long when the user set no age
/// limit, as on the dashboard.
const OBSERVATION_MAX_AGE_S: i64 = 3 * HOUR;
/// Gridded and satellite values (daily analyses and composites) count this long when the user
/// set no age limit.
const GRIDDED_MAX_AGE_S: i64 = 3 * 24 * HOUR;
/// A marine alert list counts this long when the user set no age limit, as the marine forecast.
const MARINE_ALERTS_MAX_AGE_S: i64 = 12 * HOUR;
/// A storm check counts this long: it runs every three hours.
const STORM_MAX_AGE_S: i64 = 9 * HOUR;
/// whether's current conditions stand for "now" only this close to it.
const CURRENT_WINDOW_S: i64 = HOUR;
/// The span a pressure change is measured over.
const TREND_S: i64 = 3 * HOUR;
/// Pressure observations further apart than this are not interpolated across.
const TREND_GAP: Duration = Duration::from_mins(90);
/// Tide predictions further apart than this are not interpolated across.
const TIDE_GAP: Duration = Duration::from_hours(1);
/// An alert that now ends this much later than the one told is news.
const EXTENDED_S: i64 = HOUR;

// ---------------------------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------------------------

/// What evaluation reads, gathered by the caller.
///
/// Every saved spot and trip a rule names should be present: a rule whose spot or trip is
/// missing is reported as orphaned ([`AlertRule::is_orphaned`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvalInputs {
    /// Saved spots, by spot id.
    pub spots: BTreeMap<String, SpotInputs>,
    /// Saved trips, by trip id.
    pub trips: BTreeMap<String, TripInputs>,
    /// Gridded values for box rules, by rule id.
    pub boxes: BTreeMap<String, BoxInputs>,
    /// The user's distance and age limits, as the view takes them.
    pub limits: Vec<MeasurementLimit>,
}

/// One saved spot.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotInputs {
    pub snapshot: SpotSnapshot,
    /// The spot's fishing windows ([`crate::trip::windows::compute`]).
    pub windows: Vec<FishingWindow>,
}

/// One saved trip. Its places' snapshots come from [`EvalInputs::spots`].
#[derive(Clone, Debug, PartialEq)]
pub struct TripInputs {
    /// The spot ids of the trip's places, in the trip's order.
    pub places: Vec<String>,
    /// The trip's IANA zone, for quiet hours.
    pub time_zone: String,
    /// The go/no-go of each day ([`crate::trip::gonogo::assess`]).
    pub days: Vec<DayVerdict>,
    /// The trip's fishing windows.
    pub windows: Vec<FishingWindow>,
}

/// The gridded values inside one box.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxInputs {
    /// The IANA zone quiet hours are judged in (a box has none of its own).
    pub time_zone: String,
    /// Sea surface temperature and chlorophyll, by metric.
    pub values: BTreeMap<AlertMetric, BoxValues>,
}

/// One grid's values inside a box.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxValues {
    pub source: SourceId,
    /// The instant the grid describes.
    pub time: UnixTime,
    /// The finite values of the cells inside the box, in the metric's canonical unit
    /// ([`crate::map::grid::Grid::finite_values_in`]).
    pub cells: Vec<f32>,
}

// ---------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------

/// A notification due. Not words: the view words it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AlertEvent {
    pub rule_id: String,
    pub kind: EventKind,
    /// The run that found it; earlier than its delivery when quiet hours held it.
    pub found_at: UnixTime,
    /// What met each condition, in the rule's order; a marine condition has one per alert.
    pub findings: Vec<Finding>,
}

/// Why a rule notifies.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Every condition holds, for the first time or after the rule re-armed.
    Met,
    /// A marine alert the rule watches is new, stronger or lasts longer, while it stays met.
    Update,
    /// Still met `repeat_after_hours` after the last notification.
    Repeat,
}

/// What met one condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Finding {
    /// The condition's index in the rule's `all_of`.
    pub condition: usize,
    pub reason: FindingReason,
    /// The value, in the metric's canonical unit (measurement conditions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured: Option<f64>,
    /// When: the value's time, an alert's onset, a window's or storm's start, the first hour of
    /// a Go day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<UnixTime>,
    /// When an alert, window, storm or cone ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<UnixTime>,
    /// The saved spot it was found at, for spot targets and trip places.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<StationRef>,
    /// The alert's event name or the cyclone's name, as issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

/// How a condition was met.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FindingReason {
    /// A value past the rule's threshold, or inside its range.
    Threshold,
    /// A marine alert not told before.
    AlertNew,
    /// A marine alert more severe than when it was told.
    AlertStronger,
    /// A marine alert that now ends an hour or more later than when it was told.
    AlertExtended,
    /// A marine alert already told, unchanged.
    AlertInForce,
    /// A day of the trip is Go.
    TripGo,
    /// A fishing window starts within the rule's minutes, or has started.
    WindowStarts,
    /// A significant storm in the models.
    StormAhead,
    /// An NHC forecast cone over the place.
    Hurricane,
}

impl FindingReason {
    const fn is_news(self) -> bool {
        matches!(
            self,
            Self::AlertNew | Self::AlertStronger | Self::AlertExtended
        )
    }
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

/// What the last run left, by rule id. Rules that no longer exist, or are disabled, have none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AlertState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, RuleState>,
}

/// One rule's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuleState {
    /// The last run's outcome.
    pub status: RuleStatus,
    pub checked_at: UnixTime,
    /// Since when every condition has held, while it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub met_since: Option<UnixTime>,
    /// The last notification (or the one held by quiet hours); while set the rule is latched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified_at: Option<UnixTime>,
    /// A notification held by quiet hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<AlertEvent>,
    /// Marine alerts told, by `spot id|VTEC key` (or the alert id without one).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub marine: BTreeMap<String, ToldAlert>,
}

/// The outcome of a run for one rule.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleStatus {
    /// Every condition holds.
    Met,
    /// A condition does not hold.
    NotMet,
    /// A condition's data is too old, too far away, or its source failed.
    Unchecked {
        condition: usize,
        problem: CheckProblem,
    },
    /// A condition has no data.
    NoData { condition: usize },
    /// The spot or trip the rule watches no longer exists.
    Orphaned,
}

/// Why a condition could not be checked.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckProblem {
    /// The newest value is older than the limit, seconds.
    Stale { age_s: i64, limit_s: i64 },
    /// The station is beyond the distance limit, metres.
    Far { distance_m: f32, limit_m: f32 },
    /// The source reported an error and delivered nothing.
    SourceFailed,
}

/// What was told about a marine alert.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToldAlert {
    pub event: String,
    pub severity: AlertSeverity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends: Option<UnixTime>,
}

impl RuleState {
    fn new(now: UnixTime) -> Self {
        Self {
            status: RuleStatus::NotMet,
            checked_at: now,
            met_since: None,
            notified_at: None,
            pending: None,
            marine: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------------------------

/// Judges `rules` at `now` against `inputs`, from the state the last run left.
///
/// Returns the notifications due now, in the rules' order, and the next state. Disabled rules
/// and rules no longer listed have no state in it; a repeated rule id is judged once.
pub fn evaluate(
    rules: &[AlertRule],
    inputs: &EvalInputs,
    state: &AlertState,
    now: UnixTime,
) -> (Vec<AlertEvent>, AlertState) {
    let mut events = Vec::new();
    let mut next = AlertState::default();
    for rule in rules.iter().filter(|r| r.enabled) {
        if next.rules.contains_key(&rule.id) {
            continue;
        }
        let (event, rule_state) = step(rule, inputs, state.rules.get(&rule.id), now);
        events.extend(event);
        next.rules.insert(rule.id.clone(), rule_state);
    }
    (events, next)
}

/// One rule's run.
fn step(
    rule: &AlertRule,
    inputs: &EvalInputs,
    prev: Option<&RuleState>,
    now: UnixTime,
) -> (Option<AlertEvent>, RuleState) {
    let mut st = prev.cloned().unwrap_or_else(|| RuleState::new(now));
    st.checked_at = now;
    if rule.is_orphaned(
        |id| inputs.spots.contains_key(id),
        |id| inputs.trips.contains_key(id),
    ) {
        st.status = RuleStatus::Orphaned;
        return (None, st);
    }

    let mut ctx = Ctx {
        inputs,
        told: &st.marine,
        now,
        present: BTreeSet::new(),
    };
    let outcome = ctx.rule(rule);
    let present = ctx.present;
    st.marine
        .retain(|key, told| told.ends.is_some_and(|e| e > now) || present.contains(key));

    let mut decided = None;
    match outcome {
        Outcome::Holds(found) => {
            st.status = RuleStatus::Met;
            st.met_since.get_or_insert(now);
            let news = found.iter().any(|f| f.finding.reason.is_news());
            let repeat_due = rule.repeat_after_hours.is_some_and(|h| {
                st.notified_at
                    .is_some_and(|t| now.0 - t.0 >= i64::from(h) * HOUR)
            });
            let kind = if st.notified_at.is_none() {
                Some(EventKind::Met)
            } else if news {
                Some(EventKind::Update)
            } else if repeat_due {
                Some(EventKind::Repeat)
            } else {
                None
            };
            if let Some(kind) = kind {
                st.notified_at = Some(now);
                let mut findings = Vec::with_capacity(found.len());
                for f in found {
                    if let Some((key, told)) = f.told {
                        st.marine.insert(key, told);
                    }
                    findings.push(f.finding);
                }
                decided = Some(AlertEvent {
                    rule_id: rule.id.clone(),
                    kind,
                    found_at: now,
                    findings,
                });
            }
        }
        Outcome::False { clear } => {
            st.status = RuleStatus::NotMet;
            st.met_since = None;
            if clear {
                st.notified_at = None;
            }
        }
        Outcome::Unchecked(condition, problem) => {
            st.status = RuleStatus::Unchecked { condition, problem };
        }
        Outcome::NoData(condition) => st.status = RuleStatus::NoData { condition },
    }

    let due = match (st.pending.take(), decided) {
        (Some(held), Some(new)) => Some(merge(&held, new)),
        (held, new) => held.or(new),
    };
    if is_quiet(rule.quiet_hours, target_zone(rule, inputs), now) {
        st.pending = due;
        (None, st)
    } else {
        (due, st)
    }
}

/// A held notification joined with a newer one: the newer findings, the first finding time,
/// and the kind that matters most (met over update over repeat).
fn merge(held: &AlertEvent, new: AlertEvent) -> AlertEvent {
    AlertEvent {
        kind: held.kind.min(new.kind),
        found_at: held.found_at,
        ..new
    }
}

/// The zone the rule's quiet hours are judged in.
fn target_zone<'a>(rule: &AlertRule, inputs: &'a EvalInputs) -> Option<&'a str> {
    match &rule.target {
        AlertTarget::Spot { id } => inputs
            .spots
            .get(id)
            .map(|s| s.snapshot.spot.time_zone.as_str()),
        AlertTarget::Trip { id } => inputs.trips.get(id).map(|t| t.time_zone.as_str()),
        AlertTarget::Box { .. } => inputs.boxes.get(&rule.id).map(|b| b.time_zone.as_str()),
    }
}

/// Whether `now` falls in the quiet hours, local to `zone`. An unknown zone is never quiet, so
/// nothing waits on it.
fn is_quiet(quiet: Option<QuietHours>, zone: Option<&str>, now: UnixTime) -> bool {
    let (Some(q), Some(zone)) = (quiet, zone) else {
        return false;
    };
    let (Ok(tz), Ok(ts)) = (time::zone(zone), Timestamp::from_second(now.0)) else {
        return false;
    };
    let local = ts.to_zoned(tz);
    // INVARIANT: jiff's hour is 0..=23 and minute 0..=59, so both convert.
    let minute =
        u16::try_from(local.hour()).unwrap_or(0) * 60 + u16::try_from(local.minute()).unwrap_or(0);
    let (start, end) = (q.start_minute, q.end_minute);
    if start <= end {
        start <= minute && minute < end
    } else {
        minute >= start || minute < end
    }
}

// ---------------------------------------------------------------------------------------------
// Conditions
// ---------------------------------------------------------------------------------------------

/// What met a condition, with the marine alert to remember when it is told.
#[derive(Debug)]
struct Found {
    finding: Finding,
    told: Option<(String, ToldAlert)>,
}

/// How a condition, a place or a rule came out.
#[derive(Debug)]
enum Outcome {
    Holds(Vec<Found>),
    /// Not met; `clear` when beyond the hysteresis, which re-arms the rule.
    False {
        clear: bool,
    },
    Unchecked(usize, CheckProblem),
    NoData(usize),
}

/// Every outcome must hold. A clear failure decides it; then a condition that could not be
/// checked, then one without data; no conditions at all never hold.
fn all_of(outcomes: Vec<Outcome>) -> Outcome {
    if outcomes.is_empty() {
        return Outcome::False { clear: false };
    }
    let mut found = Vec::new();
    let (mut unchecked, mut no_data, mut not_met) = (None, None, false);
    for o in outcomes {
        match o {
            Outcome::False { clear: true } => return Outcome::False { clear: true },
            Outcome::False { clear: false } => not_met = true,
            Outcome::Unchecked(i, p) => {
                unchecked.get_or_insert((i, p));
            }
            Outcome::NoData(i) => {
                no_data.get_or_insert(i);
            }
            Outcome::Holds(f) => found.extend(f),
        }
    }
    if let Some((i, p)) = unchecked {
        Outcome::Unchecked(i, p)
    } else if let Some(i) = no_data {
        Outcome::NoData(i)
    } else if not_met {
        Outcome::False { clear: false }
    } else {
        found.sort_by_key(|f| f.finding.condition);
        Outcome::Holds(found)
    }
}

/// One outcome must hold (a trip's places): the first that does; clear only when all are.
fn any_of(outcomes: Vec<Outcome>, none: Outcome) -> Outcome {
    if outcomes.is_empty() {
        return none;
    }
    let all_clear = outcomes
        .iter()
        .all(|o| matches!(o, Outcome::False { clear: true }));
    let mut unchecked = None;
    let mut no_data = None;
    for o in outcomes {
        match o {
            Outcome::Holds(_) => return o,
            Outcome::Unchecked(..) if unchecked.is_none() => unchecked = Some(o),
            Outcome::NoData(_) if no_data.is_none() => no_data = Some(o),
            _ => {}
        }
    }
    unchecked
        .or(no_data)
        .unwrap_or(Outcome::False { clear: all_clear })
}

/// A saved spot judged on its own or as a trip's place.
struct Place<'a> {
    spot_id: &'a str,
    snap: &'a SpotSnapshot,
    /// The spot's windows; `None` for a trip's place, whose windows are the trip's.
    windows: Option<&'a [FishingWindow]>,
}

struct Ctx<'a> {
    inputs: &'a EvalInputs,
    told: &'a BTreeMap<String, ToldAlert>,
    now: UnixTime,
    /// The marine alert keys found in force, so the ones told are remembered while listed.
    present: BTreeSet<String>,
}

impl Ctx<'_> {
    fn rule(&mut self, rule: &AlertRule) -> Outcome {
        let inputs = self.inputs;
        match &rule.target {
            AlertTarget::Spot { id } => {
                let Some(spot) = inputs.spots.get(id) else {
                    return Outcome::NoData(0);
                };
                let place = Place {
                    spot_id: id,
                    snap: &spot.snapshot,
                    windows: Some(&spot.windows),
                };
                let outcomes = rule
                    .all_of
                    .iter()
                    .enumerate()
                    .map(|(i, c)| self.at_place(i, c, &place))
                    .collect();
                all_of(outcomes)
            }
            AlertTarget::Trip { id } => match inputs.trips.get(id) {
                Some(trip) => self.trip(rule, trip),
                None => Outcome::NoData(0),
            },
            AlertTarget::Box { .. } => {
                let values = inputs.boxes.get(&rule.id);
                let outcomes = rule
                    .all_of
                    .iter()
                    .enumerate()
                    .map(|(i, c)| self.in_box(i, c, values))
                    .collect();
                all_of(outcomes)
            }
        }
    }

    /// A trip: go/no-go and windows are the trip's; the other conditions must all hold at one
    /// of its places.
    fn trip(&mut self, rule: &AlertRule, trip: &TripInputs) -> Outcome {
        let mut outcomes = Vec::new();
        let mut at_places = Vec::new();
        for (i, c) in rule.all_of.iter().enumerate() {
            match c {
                Condition::TripGo {} => outcomes.push(trip_go(i, &trip.days)),
                Condition::WindowStarts { within_minutes } => {
                    outcomes.push(window_starts(i, &trip.windows, *within_minutes, self.now));
                }
                _ => at_places.push((i, c)),
            }
        }
        if let Some(&(first, _)) = at_places.first() {
            let mut places = Vec::new();
            for spot_id in &trip.places {
                let Some(spot) = self.inputs.spots.get(spot_id) else {
                    places.push(Outcome::NoData(first));
                    continue;
                };
                let place = Place {
                    spot_id,
                    snap: &spot.snapshot,
                    windows: None,
                };
                let here = at_places
                    .iter()
                    .map(|&(i, c)| self.at_place(i, c, &place))
                    .collect();
                places.push(all_of(here));
            }
            outcomes.push(any_of(places, Outcome::NoData(first)));
        }
        all_of(outcomes)
    }

    fn at_place(&mut self, i: usize, c: &Condition, place: &Place<'_>) -> Outcome {
        let now = self.now;
        match c {
            Condition::Measurement {
                metric,
                below,
                above,
            } => {
                let found = self.place_reading(place.snap, *metric);
                measure(i, *metric, *below, *above, found, Some(place.spot_id))
            }
            Condition::MarineAlert { event } => self.marine(i, event, place),
            Condition::StormAhead {} => storm_ahead(i, place, now),
            Condition::WindowStarts { within_minutes } => match place.windows {
                Some(w) => window_starts(i, w, *within_minutes, now),
                None => Outcome::NoData(i),
            },
            // Go/no-go belongs to trips.
            Condition::TripGo {} => Outcome::NoData(i),
        }
    }

    fn in_box(&self, i: usize, c: &Condition, values: Option<&BoxInputs>) -> Outcome {
        let Condition::Measurement {
            metric,
            below,
            above,
        } = c
        else {
            return Outcome::NoData(i);
        };
        let Some(grid) = values
            .filter(|_| metric.in_boxes())
            .and_then(|b| b.values.get(metric))
        else {
            return Outcome::NoData(i);
        };
        let reading = Reading {
            values: (grid.cells.iter().copied())
                .filter(|v| v.is_finite())
                .map(f64::from)
                .collect(),
            at: grid.time,
            source: grid.source.clone(),
            station: None,
            distance_m: None,
            gridded: true,
            predicted: false,
        };
        let found = if reading.values.is_empty() {
            Err(None)
        } else {
            self.fresh(vec![reading], measurement_of(*metric), false)
        };
        measure(i, *metric, *below, *above, found, None)
    }

    /// The freshest usable value of `metric` at a spot, or why there is none.
    fn place_reading(
        &self,
        snap: &SpotSnapshot,
        metric: AlertMetric,
    ) -> Result<Reading, Option<CheckProblem>> {
        let measurement = measurement_of(metric);
        let failed = snap
            .errors
            .iter()
            .any(|e| e.measurement == Some(measurement));
        self.fresh(readings(snap, metric, self.now), measurement, failed)
    }

    /// The first candidate within the user's limits; else the first one's problem; else, with
    /// no candidate, a failed source or nothing.
    fn fresh(
        &self,
        candidates: Vec<Reading>,
        measurement: Measurement,
        failed: bool,
    ) -> Result<Reading, Option<CheckProblem>> {
        let limit = self
            .inputs
            .limits
            .iter()
            .find(|l| l.measurement == measurement);
        let mut first_problem = None;
        for r in candidates {
            match problem(&r, measurement, limit, self.now) {
                None => return Ok(r),
                Some(p) => {
                    first_problem.get_or_insert(p);
                }
            }
        }
        Err(first_problem.or(failed.then_some(CheckProblem::SourceFailed)))
    }

    /// Marine alerts whose event starts with `prefix`, as whether's `AlertNotifier` tells them.
    fn marine(&mut self, i: usize, prefix: &str, place: &Place<'_>) -> Outcome {
        let snap = place.snap;
        let now = self.now;
        let alerts = &snap.marine.marine_alerts;
        let failed = snap
            .errors
            .iter()
            .any(|e| e.measurement == Some(Measurement::MarineAlerts));
        if failed && alerts.is_empty() {
            return Outcome::Unchecked(i, CheckProblem::SourceFailed);
        }
        let limit_s = (self.inputs.limits.iter())
            .find(|l| l.measurement == Measurement::MarineAlerts)
            .and_then(|l| l.max_age_s)
            .unwrap_or(MARINE_ALERTS_MAX_AGE_S);
        let age_s = (now.0 - snap.fetched_at.0).max(0);
        if age_s > limit_s {
            return Outcome::Unchecked(i, CheckProblem::Stale { age_s, limit_s });
        }
        let key_prefix = format!("{}|", place.spot_id);
        let found: Vec<Found> = alerts
            .iter()
            .filter(|a| a.event.starts_with(prefix) && alert_end(a).is_none_or(|e| e > now))
            .map(|a| {
                let key = format!("{key_prefix}{}", a.vtec_key.as_deref().unwrap_or(&a.id));
                let reason = news(self.told.get(&key), a);
                self.present.insert(key.clone());
                let told = reason.is_news().then(|| {
                    let told = ToldAlert {
                        event: a.event.clone(),
                        severity: a.severity,
                        ends: alert_end(a),
                    };
                    (key, told)
                });
                Found {
                    finding: Finding {
                        at: a.onset,
                        until: alert_end(a),
                        source: Some(SourceId::NWS),
                        subject: Some(a.event.clone()),
                        ..finding(i, reason, Some(place.spot_id))
                    },
                    told,
                }
            })
            .collect();
        if found.is_empty() {
            // An alert told and not yet over keeps the rule latched while it is out of the feed.
            let remembered = self.told.iter().any(|(k, t)| {
                k.starts_with(&key_prefix)
                    && t.event.starts_with(prefix)
                    && t.ends.is_some_and(|e| e > now)
            });
            Outcome::False { clear: !remembered }
        } else {
            Outcome::Holds(found)
        }
    }
}

fn finding(condition: usize, reason: FindingReason, spot_id: Option<&str>) -> Finding {
    Finding {
        condition,
        reason,
        measured: None,
        at: None,
        until: None,
        spot_id: spot_id.map(str::to_owned),
        source: None,
        station: None,
        subject: None,
    }
}

/// When an alert ends: `ends`, or `expires` when no end is given.
fn alert_end(a: &MarineAlert) -> Option<UnixTime> {
    a.ends.or(a.expires)
}

const fn severity_rank(s: AlertSeverity) -> u8 {
    match s {
        AlertSeverity::Unknown => 0,
        AlertSeverity::Minor => 1,
        AlertSeverity::Moderate => 2,
        AlertSeverity::Severe => 3,
        AlertSeverity::Extreme => 4,
    }
}

/// How an alert compares with what was told about it.
fn news(told: Option<&ToldAlert>, a: &MarineAlert) -> FindingReason {
    let Some(told) = told else {
        return FindingReason::AlertNew;
    };
    let extended = match (told.ends, alert_end(a)) {
        (Some(old), Some(new)) => new.0 >= old.0.saturating_add(EXTENDED_S),
        // An end that is no longer given is open, so later.
        (Some(_), None) => true,
        (None, _) => false,
    };
    if severity_rank(a.severity) > severity_rank(told.severity) {
        FindingReason::AlertStronger
    } else if extended {
        FindingReason::AlertExtended
    } else {
        FindingReason::AlertInForce
    }
}

/// A day of the trip is Go. Without a decided day there is no data.
fn trip_go(i: usize, days: &[DayVerdict]) -> Outcome {
    if let Some(day) = days.iter().find(|d| d.verdict == Verdict::Go) {
        return Outcome::Holds(vec![Found {
            finding: Finding {
                at: day.hours.first().map(|h| h.start),
                ..finding(i, FindingReason::TripGo, None)
            },
            told: None,
        }]);
    }
    if days
        .iter()
        .any(|d| matches!(d.verdict, Verdict::NoGo | Verdict::Marginal))
    {
        Outcome::False { clear: true }
    } else {
        Outcome::NoData(i)
    }
}

/// A window starts within `within_minutes`, or has started and not ended.
fn window_starts(
    i: usize,
    windows: &[FishingWindow],
    within_minutes: u32,
    now: UnixTime,
) -> Outcome {
    let horizon = now.0 + i64::from(within_minutes) * 60;
    match windows.iter().find(|w| w.end > now && w.start.0 <= horizon) {
        Some(w) => Outcome::Holds(vec![Found {
            finding: Finding {
                at: Some(w.start),
                until: Some(w.end),
                ..finding(i, FindingReason::WindowStarts, None)
            },
            told: None,
        }]),
        None => Outcome::False { clear: true },
    }
}

/// A significant storm within whether's warning horizon, or an NHC cone over the place.
fn storm_ahead(i: usize, place: &Place<'_>, now: UnixTime) -> Outcome {
    let Some(watch) = place.snap.storm.as_ref() else {
        return Outcome::NoData(i);
    };
    let age_s = (now.0 - watch.checked_at.0).max(0);
    if age_s > STORM_MAX_AGE_S {
        return Outcome::Unchecked(
            i,
            CheckProblem::Stale {
                age_s,
                limit_s: STORM_MAX_AGE_S,
            },
        );
    }
    let mut found = Vec::new();
    if let Some(event) = (watch.point.as_ref()).and_then(|o| significant_event(o, now)) {
        found.push(Finding {
            at: Some(event.start),
            until: Some(event.end),
            source: Some(SourceId::new("open-meteo")),
            ..finding(i, FindingReason::StormAhead, Some(place.spot_id))
        });
    }
    let at = (place.snap.spot.latitude, place.snap.spot.longitude);
    for h in &watch.hurricanes {
        if let Some((start, end)) = hurricane_window(h, at, now, SIGNIFICANT_HORIZON_SECS) {
            found.push(Finding {
                at: Some(start),
                until: Some(end),
                source: Some(SourceId::new("nhc")),
                subject: Some(h.name.clone()),
                ..finding(i, FindingReason::Hurricane, Some(place.spot_id))
            });
        }
    }
    if found.is_empty() {
        Outcome::False { clear: true }
    } else {
        Outcome::Holds(
            found
                .into_iter()
                .map(|finding| Found {
                    finding,
                    told: None,
                })
                .collect(),
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Measurements
// ---------------------------------------------------------------------------------------------

/// One or more values (a box's cells) with where and when they come from.
#[derive(Debug)]
struct Reading {
    values: Vec<f64>,
    at: UnixTime,
    source: SourceId,
    station: Option<StationRef>,
    distance_m: Option<f32>,
    /// A model cell or satellite pixel: judged by the gridded age.
    gridded: bool,
    /// A prediction for `now` (tides, whether's current conditions): never stale.
    predicted: bool,
}

impl Reading {
    fn of(p: &Provenance, at: UnixTime, value: f32) -> Option<Self> {
        value.is_finite().then(|| Self {
            values: vec![f64::from(value)],
            at,
            source: p.source.clone(),
            station: p.station.clone(),
            distance_m: p.distance_m,
            gridded: matches!(p.relation, Relation::Gridded | Relation::Satellite)
                || p.kind == ValueKind::Satellite,
            predicted: p.kind == ValueKind::Predicted,
        })
    }
}

/// The measurement whose limits apply to a metric.
const fn measurement_of(metric: AlertMetric) -> Measurement {
    match metric {
        AlertMetric::Discharge => Measurement::Discharge,
        AlertMetric::GageHeight => Measurement::GageHeight,
        AlertMetric::WaterTemperature => Measurement::WaterTemperature,
        AlertMetric::SeaSurfaceTemp => Measurement::SeaSurfaceTemp,
        AlertMetric::WaveHeight => Measurement::Waves,
        AlertMetric::Wind | AlertMetric::Gusts => Measurement::ObservedWind,
        AlertMetric::PressureTrend => Measurement::ObservedPressure,
        AlertMetric::TideHeight => Measurement::Tide,
        AlertMetric::Chlorophyll => Measurement::Chlorophyll,
    }
}

/// The age limit when the user set none.
const fn default_max_age_s(measurement: Measurement, gridded: bool) -> i64 {
    if gridded {
        return GRIDDED_MAX_AGE_S;
    }
    match measurement {
        Measurement::ObservedWind | Measurement::ObservedPressure | Measurement::Waves => {
            OBSERVATION_MAX_AGE_S
        }
        _ => STATION_MAX_AGE_S,
    }
}

/// Why a reading may not be used: a station beyond the user's distance limit, or a value older
/// than the age limit.
fn problem(
    r: &Reading,
    measurement: Measurement,
    limit: Option<&MeasurementLimit>,
    now: UnixTime,
) -> Option<CheckProblem> {
    if let (Some(distance_m), Some(limit_m)) = (r.distance_m, limit.and_then(|l| l.max_distance_m))
        && distance_m > limit_m
    {
        return Some(CheckProblem::Far {
            distance_m,
            limit_m,
        });
    }
    if r.predicted {
        return None;
    }
    let limit_s = limit
        .and_then(|l| l.max_age_s)
        .unwrap_or_else(|| default_max_age_s(measurement, r.gridded));
    let age_s = (now.0 - r.at.0).max(0);
    (age_s > limit_s).then_some(CheckProblem::Stale { age_s, limit_s })
}

fn latest_at<T>(series: &Series<T>, now: UnixTime) -> Option<&Sample<T>> {
    series.samples().iter().rev().find(|s| s.time <= now)
}

fn latest_reading(series: &Series<f32>, now: UnixTime) -> Option<Reading> {
    let s = latest_at(series, now)?;
    Reading::of(series.provenance(), s.time, s.value)
}

/// The candidate values of `metric` at a spot, preferred first.
fn readings(snap: &SpotSnapshot, metric: AlertMetric, now: UnixTime) -> Vec<Reading> {
    let (water, marine) = (&snap.water, &snap.marine);
    let latest = |s: Option<&Series<f32>>| s.and_then(|s| latest_reading(s, now));
    let found = match metric {
        AlertMetric::Discharge => latest(water.discharge_m3s.as_ref()),
        AlertMetric::GageHeight => latest(water.gage_height.as_ref().map(|h| &h.series)),
        AlertMetric::WaterTemperature => latest(water.temperature_c.as_ref()),
        AlertMetric::SeaSurfaceTemp => latest(marine.sea_surface_temp_c.as_ref()),
        AlertMetric::Chlorophyll => latest(water.chlorophyll_ug_l.as_ref()),
        AlertMetric::WaveHeight => marine.waves.as_ref().and_then(|w| {
            let s = w
                .samples()
                .iter()
                .rev()
                .filter(|s| s.time <= now)
                .find_map(|s| s.value.significant_height_m.map(|h| (s.time, h)))?;
            Reading::of(w.provenance(), s.0, s.1)
        }),
        AlertMetric::Wind | AlertMetric::Gusts => return wind(snap, metric, now),
        AlertMetric::PressureTrend => marine.observed_pressure_hpa.as_ref().and_then(|s| {
            let end = latest_at(s, now)?;
            let start = s.at(UnixTime(end.time.0 - TREND_S), TREND_GAP)?;
            Reading::of(s.provenance(), end.time, end.value - start)
        }),
        AlertMetric::TideHeight => return tide(snap, now),
    };
    found.into_iter().collect()
}

/// Station wind or gusts, then whether's current conditions for about now.
fn wind(snap: &SpotSnapshot, metric: AlertMetric, now: UnixTime) -> Vec<Reading> {
    let gusts = metric == AlertMetric::Gusts;
    let mut out = Vec::new();
    if let Some(series) = snap.marine.observed_wind.as_ref() {
        let seen = (series.samples().iter().rev())
            .filter(|s| s.time <= now)
            .find_map(|s| {
                let v = if gusts {
                    s.value.gust_ms?
                } else {
                    s.value.speed_ms
                };
                Some((s.time, v))
            });
        out.extend(seen.and_then(|(t, v)| Reading::of(series.provenance(), t, v)));
    }
    let current = snap.weather.as_ref().and_then(|w| {
        let c = w.current.as_ref()?;
        let v = if gusts { c.wind_gust } else { c.wind_speed }?;
        ((now.0 - c.time.0).abs() <= CURRENT_WINDOW_S && v.0.is_finite()).then_some((w, c, v))
    });
    if let Some((w, c, v)) = current {
        out.push(Reading {
            values: vec![f64::from(v.0)],
            at: c.time,
            source: SourceId::from_static(w.provider.id()),
            station: None,
            distance_m: None,
            gridded: true,
            predicted: true,
        });
    }
    out
}

/// The observed water level, then the prediction for now; a model's mean sea level has no
/// datum heights.
fn tide(snap: &SpotSnapshot, now: UnixTime) -> Vec<Reading> {
    let Some(tide) = snap
        .marine
        .tide
        .as_ref()
        .filter(|t| t.source_kind != TideSourceKind::ModelMean)
    else {
        return Vec::new();
    };
    let mut out: Vec<Reading> = tide
        .observed
        .as_ref()
        .and_then(|s| latest_reading(s, now))
        .into_iter()
        .collect();
    if let Some(curve) = tide.curve.as_ref()
        && let Some(v) = curve.at(now, TIDE_GAP)
        && let Some(mut r) = Reading::of(curve.provenance(), now, v)
    {
        r.predicted = true;
        out.push(r);
    }
    out
}

/// How far back across a threshold a value must go to re-arm a rule.
fn hysteresis(metric: AlertMetric, threshold: f64) -> f64 {
    match metric {
        AlertMetric::Discharge | AlertMetric::Chlorophyll => 0.02 * threshold.abs(),
        AlertMetric::WaterTemperature
        | AlertMetric::SeaSurfaceTemp
        | AlertMetric::PressureTrend => 0.3,
        AlertMetric::GageHeight | AlertMetric::TideHeight => 0.03,
        AlertMetric::WaveHeight => 0.1,
        AlertMetric::Wind | AlertMetric::Gusts => f64::from(SpeedUnit::Kn.to_canonical(1.0)),
    }
}

/// Judges a value (or a box's cells, any of which may meet it) against `below` and `above`.
fn measure(
    i: usize,
    metric: AlertMetric,
    below: Option<f64>,
    above: Option<f64>,
    found: Result<Reading, Option<CheckProblem>>,
    spot_id: Option<&str>,
) -> Outcome {
    let r = match found {
        Ok(r) => r,
        Err(Some(p)) => return Outcome::Unchecked(i, p),
        Err(None) => return Outcome::NoData(i),
    };
    if below.is_none() && above.is_none() {
        return Outcome::NoData(i);
    }
    let holds = |v: f64| below.is_none_or(|b| v < b) && above.is_none_or(|a| v > a);
    let clear = |v: f64| {
        below.is_some_and(|b| v >= b + hysteresis(metric, b))
            || above.is_some_and(|a| v <= a - hysteresis(metric, a))
    };
    let meeting = r.values.iter().copied().filter(|&v| holds(v));
    // The most telling value: the lowest under a ceiling, else the highest.
    let measured = if above.is_none() {
        meeting.reduce(f64::min)
    } else {
        meeting.reduce(f64::max)
    };
    match measured {
        Some(v) => Outcome::Holds(vec![Found {
            finding: Finding {
                condition: i,
                reason: FindingReason::Threshold,
                measured: Some(v),
                at: Some(r.at),
                until: None,
                spot_id: spot_id.map(str::to_owned),
                source: Some(r.source),
                station: r.station,
                subject: None,
            },
            told: None,
        }]),
        None => Outcome::False {
            clear: r.values.iter().all(|&v| clear(v)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::snapshot::testdata as sample;
    use crate::model::{SourceError, SourceErrorKind, Spot, StormWatch, WaterType, Wind};
    use crate::provenance::{Quality, StationKind, UsgsSiteType};
    use crate::time::{LocalDate, parse_rfc3339};
    use crate::trip::gonogo::HourVerdict;
    use crate::units::DischargeUnit;
    use serde_json::json;
    use whether_core::storm::testdata as storm_data;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/raw/");
    const MIN: i64 = 60;

    fn at(iso: &str) -> UnixTime {
        parse_rfc3339(iso).unwrap()
    }

    fn after(t: UnixTime, seconds: i64) -> UnixTime {
        UnixTime(t.0 + seconds)
    }

    fn cfs(v: f32) -> f64 {
        f64::from(DischargeUnit::Cfs.to_canonical(v))
    }

    fn fixture(path: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}{path}")).unwrap()
    }

    fn croton() -> Spot {
        Spot {
            id: "croton".into(),
            name: "Croton River".into(),
            latitude: 41.2245,
            longitude: -73.8590,
            water_type: WaterType::RiverStream,
            water_body: None,
            time_zone: "America/New_York".into(),
        }
    }

    fn gauge(distance_m: f32) -> Provenance {
        Provenance {
            source: SourceId::USGS,
            station: Some(StationRef {
                source: SourceId::USGS,
                id: "USGS-01375000".into(),
                name: "Croton R at New Croton Dam".into(),
                kind: StationKind::UsgsSite(UsgsSiteType::Stream),
                latitude: 41.2259,
                longitude: -73.8582,
                bin: None,
                reference_id: None,
            }),
            distance_m: Some(distance_m),
            relation: Relation::SameWaterBody,
            observed_at: UnixTime(0),
            kind: ValueKind::Observed,
            quality: Quality::Provisional,
        }
    }

    /// The recorded Croton discharge (USGS continuous values, ft³/s), in m³/s.
    fn croton_flow() -> Vec<Sample<f32>> {
        let body = fixture("usgs/usgs-continuous-croton-01375000-00060-2026-11-01_2026-11-08.json");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let p = &f["properties"];
                assert_eq!(p["unit_of_measure"], "ft^3/s");
                let value: f32 = p["value"].as_str().unwrap().parse().unwrap();
                Sample::new(
                    at(p["time"].as_str().unwrap()),
                    DischargeUnit::Cfs.to_canonical(value),
                )
            })
            .collect()
    }

    /// Synthetic ft³/s values a quarter of an hour apart from `from`, in m³/s.
    fn synthetic_cfs(from: UnixTime, values: &[f32]) -> Vec<Sample<f32>> {
        (0_i64..)
            .zip(values)
            .map(|(i, &v)| {
                Sample::new(
                    after(from, i * 15 * MIN),
                    DischargeUnit::Cfs.to_canonical(v),
                )
            })
            .collect()
    }

    /// The Croton spot as fetched at `now`: the samples up to then.
    fn flow_inputs(samples: &[Sample<f32>], now: UnixTime) -> EvalInputs {
        let mut snap = SpotSnapshot::new(croton(), now);
        let seen = samples.iter().filter(|s| s.time <= now).copied().collect();
        snap.water.discharge_m3s = Some(Series::new(gauge(150.0), seen));
        spot_inputs(snap)
    }

    fn spot_inputs(snapshot: SpotSnapshot) -> EvalInputs {
        let id = snapshot.spot.id.clone();
        EvalInputs {
            spots: BTreeMap::from([(
                id,
                SpotInputs {
                    snapshot,
                    windows: Vec::new(),
                },
            )]),
            ..EvalInputs::default()
        }
    }

    fn rule(target: AlertTarget, all_of: Vec<Condition>) -> AlertRule {
        AlertRule {
            id: "r1".into(),
            name: "Rule".into(),
            enabled: true,
            target,
            all_of,
            repeat_after_hours: None,
            quiet_hours: None,
        }
    }

    fn spot(id: &str) -> AlertTarget {
        AlertTarget::Spot { id: id.into() }
    }

    fn measurement(metric: AlertMetric, below: Option<f64>, above: Option<f64>) -> Condition {
        Condition::Measurement {
            metric,
            below,
            above,
        }
    }

    fn flow_below(v: f32) -> Condition {
        measurement(AlertMetric::Discharge, Some(cfs(v)), None)
    }

    /// Evaluates `rules` once per time, each with its own inputs, carrying the state along.
    /// Returns the events with the time of their run, and the last state.
    fn replay(
        rules: &[AlertRule],
        times: impl IntoIterator<Item = UnixTime>,
        inputs: impl Fn(UnixTime) -> EvalInputs,
    ) -> (Vec<(UnixTime, AlertEvent)>, AlertState) {
        let mut state = AlertState::default();
        let mut events = Vec::new();
        for now in times {
            let (fired, next) = evaluate(rules, &inputs(now), &state, now);
            events.extend(fired.into_iter().map(|e| (now, e)));
            state = next;
        }
        (events, state)
    }

    fn every(from: UnixTime, to: UnixTime, step_s: i64) -> impl Iterator<Item = UnixTime> {
        (0..)
            .map(move |i| after(from, i * step_s))
            .take_while(move |t| *t <= to)
    }

    fn status(state: &AlertState) -> RuleStatus {
        state.rules["r1"].status
    }

    fn once(
        rules: &[AlertRule],
        inputs: &EvalInputs,
        now: UnixTime,
    ) -> (Vec<AlertEvent>, AlertState) {
        evaluate(rules, inputs, &AlertState::default(), now)
    }

    // --- Measurements, hysteresis and repeats ------------------------------------------------

    #[test]
    fn croton_flow_falling_through_a_threshold_fires_once() {
        let flow = croton_flow();
        let rules = [rule(spot("croton"), vec![flow_below(50.0)])];
        let (first, last) = (flow[0].time, flow[flow.len() - 1].time);

        // A run at every sample: the recording dips under 50 cfs, back to 50.2 and under again
        // (13:30 and 14:15 UTC on 2 November), inside the 2 % band, so it tells once.
        let (events, state) = replay(&rules, every(first, last, 15 * MIN), |now| {
            flow_inputs(&flow, now)
        });
        assert_eq!(events.len(), 1, "{events:?}");
        let (when, event) = &events[0];
        assert_eq!(*when, at("2026-11-02T13:30:00Z"));
        assert_eq!(event.kind, EventKind::Met);
        let f = &event.findings[0];
        assert_eq!(f.reason, FindingReason::Threshold);
        assert!((f.measured.unwrap() - cfs(48.9)).abs() < 1e-4);
        assert_eq!(f.at, Some(*when));
        assert_eq!(f.spot_id.as_deref(), Some("croton"));
        assert_eq!(f.station.as_ref().unwrap().id, "USGS-01375000");
        assert_eq!(status(&state), RuleStatus::Met);
        // Met without a break since the second dip.
        assert_eq!(
            state.rules["r1"].met_since,
            Some(at("2026-11-02T14:15:00Z"))
        );

        // Runs hours apart judge the latest value only: still one event, at the first run under.
        let (events, _) = replay(&rules, every(first, last, 3 * 3_600), |now| {
            flow_inputs(&flow, now)
        });
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, at("2026-11-02T15:00:00Z"));
    }

    #[test]
    fn croton_flow_rearms_only_after_recovering_past_the_band() {
        let mut flow = croton_flow();
        let rules = [rule(spot("croton"), vec![flow_below(50.0)])];
        let (first, last) = (flow[0].time, flow[flow.len() - 1].time);
        // After the recording: a rise to 50.8 cfs (inside the band, which ends at 51 cfs), dips,
        // a recovery to 52 cfs, and a fall under again.
        let tail = [50.8, 49.0, 50.9, 48.0, 52.0, 53.0, 45.0];
        let start = after(last, 15 * MIN);
        flow.extend(synthetic_cfs(start, &tail));
        let end = after(start, 15 * MIN * (i64::try_from(tail.len()).unwrap() - 1));

        let mut state = AlertState::default();
        let mut times = Vec::new();
        let mut armed = Vec::new();
        for now in every(first, end, 15 * MIN) {
            let (fired, next) = evaluate(&rules, &flow_inputs(&flow, now), &state, now);
            assert!(fired.iter().all(|e| e.kind == EventKind::Met));
            times.extend(fired.iter().map(|_| now));
            if now >= start {
                armed.push(next.rules["r1"].notified_at.is_none());
            }
            state = next;
        }
        assert_eq!(
            times,
            [at("2026-11-02T13:30:00Z"), end],
            "fires once on the way down and again only after the recovery"
        );
        assert_eq!(armed, [false, false, false, false, true, true, false]);
    }

    #[test]
    fn wind_flapping_inside_one_knot_fires_once() {
        let kn = |v: f32| f64::from(SpeedUnit::Kn.to_canonical(v));
        let rules = [rule(
            spot("battery"),
            vec![measurement(AlertMetric::Wind, None, Some(kn(20.0)))],
        )];
        let t0 = at("2026-11-08T12:00:00Z");
        let speeds = [21.0, 19.5, 21.0, 19.2, 22.0, 18.9, 20.5];
        let wind_at = |now: UnixTime| {
            let i = usize::try_from((now.0 - t0.0) / 3_600).unwrap();
            let mut snap = SpotSnapshot::new(sample::spot(), now);
            let w = Wind {
                speed_ms: SpeedUnit::Kn.to_canonical(speeds[i]),
                gust_ms: None,
                from_deg: None,
            };
            let buoy = Provenance {
                source: SourceId::NDBC,
                ..gauge(900.0)
            };
            snap.marine.observed_wind = Some(Series::new(buoy, vec![Sample::new(now, w)]));
            spot_inputs(snap)
        };
        let (events, _) = replay(&rules, every(t0, after(t0, 6 * 3_600), 3_600), wind_at);
        let hours: Vec<_> = events.iter().map(|(t, _)| (t.0 - t0.0) / 3_600).collect();
        assert_eq!(hours, [0, 6], "18.9 kn re-arms, 19.2 kn does not");
    }

    #[test]
    fn repeat_after_hours_tells_again_while_met() {
        let mut r = rule(spot("croton"), vec![flow_below(50.0)]);
        r.repeat_after_hours = Some(6);
        let t0 = at("2026-11-04T00:00:00Z");
        let flow = croton_flow();
        let times = every(t0, after(t0, 15 * 3_600), 3 * 3_600);
        let (events, _) = replay(&[r], times, |now| flow_inputs(&flow, now));
        let got: Vec<_> = events
            .iter()
            .map(|(t, e)| ((t.0 - t0.0) / 3_600, e.kind))
            .collect();
        assert_eq!(
            got,
            [
                (0, EventKind::Met),
                (6, EventKind::Repeat),
                (12, EventKind::Repeat)
            ]
        );
    }

    #[test]
    fn a_range_holds_only_between_its_bounds() {
        let rules = [rule(
            spot("croton"),
            vec![measurement(
                AlertMetric::Discharge,
                Some(cfs(100.0)),
                Some(cfs(20.0)),
            )],
        )];
        let flow = croton_flow();
        let check = |iso: &str| {
            let now = at(iso);
            status(&once(&rules, &flow_inputs(&flow, now), now).1)
        };
        // 163 cfs at the start, about 50 on 2 November, 12 cfs from 4 November.
        assert_eq!(check("2026-11-01T00:00:00Z"), RuleStatus::NotMet);
        assert_eq!(check("2026-11-02T13:30:00Z"), RuleStatus::Met);
        assert_eq!(check("2026-11-05T00:00:00Z"), RuleStatus::NotMet);
    }

    #[test]
    fn every_spot_metric_reads_its_latest_value() {
        let snap = sample::sample_snapshot();
        let now = after(snap.fetched_at, 10 * MIN);
        let value = |m| readings(&snap, m, now).first().map(|r| r.values[0]);
        let close = |m, want: f64| {
            let got = value(m).unwrap_or_else(|| panic!("{m:?}"));
            assert!((got - want).abs() < 1e-5, "{m:?}: {got}");
        };
        close(AlertMetric::WaterTemperature, 14.6);
        close(AlertMetric::Discharge, 0.34);
        close(AlertMetric::GageHeight, 1.21);
        close(AlertMetric::SeaSurfaceTemp, 15.0);
        close(AlertMetric::WaveHeight, 0.9);
        close(AlertMetric::Wind, 6.0);
        close(AlertMetric::Gusts, 8.0);
        close(AlertMetric::Chlorophyll, 2.5);
        // The observed water level comes before the prediction.
        close(AlertMetric::TideHeight, 1.5);
        // One pressure reading has no three-hour change.
        assert_eq!(value(AlertMetric::PressureTrend), None);
    }

    // --- Freshness, distance and missing data ------------------------------------------------

    #[test]
    fn a_stale_value_never_fires_and_reports_its_age() {
        let rules = [rule(spot("croton"), vec![flow_below(50.0)])];
        let flow = synthetic_cfs(at("2026-11-08T00:00:00Z"), &[20.0]);
        let now = at("2026-11-09T06:00:00Z");
        let (events, state) = once(&rules, &flow_inputs(&flow, now), now);
        assert_eq!(events, []);
        assert_eq!(
            status(&state),
            RuleStatus::Unchecked {
                condition: 0,
                problem: CheckProblem::Stale {
                    age_s: 30 * 3_600,
                    limit_s: 24 * 3_600
                }
            }
        );

        // The user's own age limit counts instead.
        let mut inputs = flow_inputs(&flow, now);
        inputs.limits = vec![MeasurementLimit {
            measurement: Measurement::Discharge,
            max_distance_m: None,
            max_age_s: Some(36 * 3_600),
        }];
        let (events, state) = once(&rules, &inputs, now);
        assert_eq!(events.len(), 1);
        assert_eq!(status(&state), RuleStatus::Met);
    }

    #[test]
    fn a_station_beyond_the_distance_limit_never_fires() {
        let rules = [rule(spot("croton"), vec![flow_below(50.0)])];
        let now = at("2026-11-08T00:00:00Z");
        let mut inputs = flow_inputs(&croton_flow(), now);
        inputs.limits = vec![MeasurementLimit {
            measurement: Measurement::Discharge,
            max_distance_m: Some(100.0),
            max_age_s: None,
        }];
        let (events, state) = once(&rules, &inputs, now);
        assert_eq!(events, []);
        assert_eq!(
            status(&state),
            RuleStatus::Unchecked {
                condition: 0,
                problem: CheckProblem::Far {
                    distance_m: 150.0,
                    limit_m: 100.0
                }
            }
        );
    }

    #[test]
    fn data_that_cannot_be_checked_keeps_the_latch() {
        let rules = [rule(spot("croton"), vec![flow_below(50.0)])];
        let t0 = at("2026-11-08T00:00:00Z");
        let low = synthetic_cfs(t0, &[20.0]);
        let (events, state) = once(&rules, &flow_inputs(&low, t0), t0);
        assert_eq!(events.len(), 1);

        // A day and a half later the only value is a high one that has gone stale: no re-arm.
        let high = synthetic_cfs(after(t0, 3_600), &[90.0]);
        let later = after(t0, 36 * 3_600);
        let (_, state) = evaluate(&rules, &flow_inputs(&high, later), &state, later);
        assert!(matches!(status(&state), RuleStatus::Unchecked { .. }));
        assert!(state.rules["r1"].notified_at.is_some());

        // Fresh and low again: still the same episode, nothing new to tell.
        let fresh = synthetic_cfs(later, &[20.0]);
        let (events, _) = evaluate(&rules, &flow_inputs(&fresh, later), &state, later);
        assert_eq!(events, []);
    }

    #[test]
    fn missing_data_never_fires() {
        let rules = [rule(spot("croton"), vec![flow_below(50.0)])];
        let now = at("2026-11-08T00:00:00Z");
        let mut snap = SpotSnapshot::new(croton(), now);
        let (events, state) = once(&rules, &spot_inputs(snap.clone()), now);
        assert_eq!(events, []);
        assert_eq!(status(&state), RuleStatus::NoData { condition: 0 });

        // A failed source says so.
        snap.errors.push(SourceError {
            source: SourceId::USGS,
            measurement: Some(Measurement::Discharge),
            kind: SourceErrorKind::RateLimited,
            message: "rate limited".into(),
        });
        let (_, state) = once(&rules, &spot_inputs(snap), now);
        assert_eq!(
            status(&state),
            RuleStatus::Unchecked {
                condition: 0,
                problem: CheckProblem::SourceFailed
            }
        );
    }

    // --- Marine alerts -----------------------------------------------------------------------

    /// The recorded ANZ338 alerts, as the NWS provider parses them.
    fn anz338_alerts() -> Vec<MarineAlert> {
        let body = fixture("nws-marine/alerts-active-zone-ANZ338.geo.json");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        v["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let p = &f["properties"];
                let time = |k: &str| p[k].as_str().map(at);
                let vtec = p["parameters"]["VTEC"][0].as_str().unwrap();
                let key: Vec<&str> = vtec.trim_matches('/').split('.').collect();
                MarineAlert {
                    id: p["id"].as_str().unwrap().into(),
                    event: p["event"].as_str().unwrap().into(),
                    headline: p["headline"].as_str().map(Into::into),
                    description: None,
                    instruction: None,
                    onset: time("onset"),
                    ends: time("ends"),
                    expires: time("expires"),
                    severity: AlertSeverity::Minor,
                    zones: p["geocode"]["UGC"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|z| z.as_str().unwrap().into())
                        .collect(),
                    vtec_key: Some(key[2..6].join(".")),
                }
            })
            .collect()
    }

    /// A trip to The Battery, in ANZ338, whose snapshot fetched at `now` lists `alerts`.
    fn trip_inputs(alerts: Vec<MarineAlert>, now: UnixTime) -> EvalInputs {
        let mut snap = SpotSnapshot::new(sample::spot(), now);
        snap.marine.marine_alerts = alerts;
        let mut inputs = spot_inputs(snap);
        inputs.trips.insert(
            "trip-1".into(),
            TripInputs {
                places: vec!["battery".into()],
                time_zone: "America/New_York".into(),
                days: Vec::new(),
                windows: Vec::new(),
            },
        );
        inputs
    }

    fn trip() -> AlertTarget {
        AlertTarget::Trip {
            id: "trip-1".into(),
        }
    }

    fn sca_rule() -> AlertRule {
        rule(
            trip(),
            vec![Condition::MarineAlert {
                event: "Small Craft Advisory".into(),
            }],
        )
    }

    #[test]
    fn the_anz338_small_craft_advisory_fires_once_for_a_trip_there() {
        let alerts = anz338_alerts();
        assert!(alerts[0].zones.iter().any(|z| z == "ANZ338"));
        let rules = [sca_rule()];
        let t0 = at("2026-11-08T14:30:00Z");
        let times = every(t0, after(t0, 12 * 3_600), 3 * 3_600);
        let (events, state) = replay(&rules, times, |now| trip_inputs(alerts.clone(), now));
        assert_eq!(events.len(), 1);
        let (when, event) = &events[0];
        assert_eq!(*when, t0);
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "rule_id": "r1",
                "kind": "met",
                "found_at": t0.0,
                "findings": [{
                    "condition": 0,
                    "reason": "alert_new",
                    "at": at("2026-11-08T06:48:00Z").0,
                    "until": at("2026-11-09T23:00:00Z").0,
                    "spot_id": "battery",
                    "source": "nws",
                    "subject": "Small Craft Advisory"
                }]
            })
        );
        let told = &state.rules["r1"].marine["battery|KOKX.SC.Y.0100"];
        assert_eq!(told.ends, Some(at("2026-11-09T23:00:00Z")));

        // A Gale Warning rule finds nothing in the same feed.
        let gale = AlertRule {
            all_of: vec![Condition::MarineAlert {
                event: "Gale Warning".into(),
            }],
            ..sca_rule()
        };
        let (events, state) = once(&[gale], &trip_inputs(alerts, t0), t0);
        assert_eq!(events, []);
        assert_eq!(status(&state), RuleStatus::NotMet);
    }

    #[test]
    fn an_advisory_is_told_again_only_when_stronger_or_an_hour_longer() {
        let rules = [sca_rule()];
        let t0 = at("2026-11-08T14:30:00Z");
        let base = anz338_alerts();
        let ends = base[0].ends.unwrap();
        let run = |state: &AlertState, hours: i64, f: &dyn Fn(&mut MarineAlert)| {
            let now = after(t0, hours * 3_600);
            let mut alerts = base.clone();
            f(&mut alerts[0]);
            evaluate(&rules, &trip_inputs(alerts, now), state, now)
        };
        let (events, state) = run(&AlertState::default(), 0, &|_| {});
        assert_eq!(events.len(), 1);

        // Reissued with a new id and expiry, ending 30 minutes later: quiet.
        let (events, state) = run(&state, 1, &|a| {
            a.id = "urn:oid:reissued".into();
            a.expires = Some(after(a.expires.unwrap(), 6 * 3_600));
            a.ends = Some(after(ends, 30 * MIN));
        });
        assert_eq!(events, []);

        // Extended by two hours: told, as an update.
        let longer = |a: &mut MarineAlert| a.ends = Some(after(ends, 2 * 3_600));
        let (events, state) = run(&state, 2, &longer);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::Update);
        assert_eq!(events[0].findings[0].reason, FindingReason::AlertExtended);

        // More severe: told again.
        let stronger = |a: &mut MarineAlert| {
            longer(a);
            a.severity = AlertSeverity::Moderate;
        };
        let (events, state) = run(&state, 3, &stronger);
        assert_eq!(events[0].findings[0].reason, FindingReason::AlertStronger);

        // Out of the feed for a run and back, unchanged: quiet, and the rule stays latched.
        let now = after(t0, 4 * 3_600);
        let (events, gap) = evaluate(&rules, &trip_inputs(Vec::new(), now), &state, now);
        assert_eq!(events, []);
        assert_eq!(status(&gap), RuleStatus::NotMet);
        assert!(gap.rules["r1"].notified_at.is_some());
        let (events, state) = run(&gap, 5, &stronger);
        assert_eq!(events, []);

        // Once it has ended it is forgotten and the rule re-arms.
        let over = after(ends, 3 * 3_600);
        let (_, state) = evaluate(&rules, &trip_inputs(Vec::new(), over), &state, over);
        let rs = &state.rules["r1"];
        assert!(rs.marine.is_empty() && rs.notified_at.is_none());
    }

    #[test]
    fn a_failed_or_old_alert_feed_cannot_be_checked() {
        let rules = [sca_rule()];
        let now = at("2026-11-08T14:30:00Z");
        let mut inputs = trip_inputs(Vec::new(), now);
        let snap = &mut inputs.spots.get_mut("battery").unwrap().snapshot;
        snap.errors.push(SourceError {
            source: SourceId::NWS,
            measurement: Some(Measurement::MarineAlerts),
            kind: SourceErrorKind::Transport,
            message: "timed out".into(),
        });
        let (_, state) = once(&rules, &inputs, now);
        assert_eq!(
            status(&state),
            RuleStatus::Unchecked {
                condition: 0,
                problem: CheckProblem::SourceFailed
            }
        );

        let old = trip_inputs(anz338_alerts(), after(now, -13 * 3_600));
        let (events, state) = once(&rules, &old, now);
        assert_eq!(events, []);
        assert!(matches!(
            status(&state),
            RuleStatus::Unchecked {
                problem: CheckProblem::Stale { .. },
                ..
            }
        ));
    }

    // --- Quiet hours -------------------------------------------------------------------------

    fn quiet(start_h: u16, end_h: u16) -> QuietHours {
        QuietHours {
            start_minute: start_h * 60,
            end_minute: end_h * 60,
        }
    }

    #[test]
    fn quiet_hours_hold_a_notification_and_never_drop_it() {
        let mut r = rule(spot("croton"), vec![flow_below(50.0)]);
        r.quiet_hours = Some(quiet(22, 7));
        let rules = [r];
        // 22:30 in New York (EST): the flow is under.
        let t0 = at("2026-11-09T03:30:00Z");
        let low = synthetic_cfs(t0, &[20.0]);
        let (events, state) = once(&rules, &flow_inputs(&low, t0), t0);
        assert_eq!(events, []);
        assert!(state.rules["r1"].pending.is_some());

        // 03:00 local: still waiting.
        let t1 = at("2026-11-09T08:00:00Z");
        let (events, state) = evaluate(&rules, &flow_inputs(&low, t1), &state, t1);
        assert_eq!(events, []);

        // 07:30 local, and the flow has recovered meanwhile: the held notification is delivered.
        let t2 = at("2026-11-09T12:30:00Z");
        let high = synthetic_cfs(t2, &[90.0]);
        let (events, state) = evaluate(&rules, &flow_inputs(&high, t2), &state, t2);
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].kind, events[0].found_at), (EventKind::Met, t0));
        assert!(state.rules["r1"].pending.is_none());
        let (events, _) = evaluate(&rules, &flow_inputs(&high, t2), &state, t2);
        assert_eq!(events, []);
    }

    #[test]
    fn quiet_hours_follow_the_target_zone_across_midnight_and_dst() {
        let ny = Some("America/New_York");
        // 22:00 to 07:00 crosses midnight.
        let night = Some(quiet(22, 7));
        assert!(is_quiet(night, ny, at("2026-11-09T03:30:00Z")));
        assert!(is_quiet(night, ny, at("2026-11-09T11:59:00Z")));
        assert!(!is_quiet(night, ny, at("2026-11-09T12:00:00Z")));
        assert!(!is_quiet(night, ny, at("2026-11-09T02:59:00Z")));
        // The same instant is evening on the west coast.
        let la = Some("America/Los_Angeles");
        assert!(!is_quiet(night, la, at("2026-11-09T03:30:00Z")));
        // On 1 November 2026 the hour from 01:00 happens twice in New York: both are quiet.
        let one = Some(quiet(1, 2));
        assert!(!is_quiet(one, ny, at("2026-11-01T04:30:00Z")));
        assert!(is_quiet(one, ny, at("2026-11-01T05:30:00Z")));
        assert!(is_quiet(one, ny, at("2026-11-01T06:30:00Z")));
        assert!(!is_quiet(one, ny, at("2026-11-01T07:30:00Z")));
        // No zone, an unknown zone or an empty span are never quiet.
        assert!(!is_quiet(night, None, at("2026-11-09T03:30:00Z")));
        assert!(!is_quiet(
            night,
            Some("Mars/Olympus"),
            at("2026-11-09T03:30:00Z")
        ));
        assert!(!is_quiet(Some(quiet(3, 3)), ny, at("2026-11-09T08:00:00Z")));
    }

    // --- Trips, windows, storms and boxes ----------------------------------------------------

    fn day(d: u8, verdict: Verdict) -> DayVerdict {
        let start = at(&format!("2026-11-{d:02}T05:00:00Z"));
        DayVerdict {
            day: LocalDate {
                year: 2026,
                month: 11,
                day: d,
            },
            verdict,
            reasons: Vec::new(),
            hours: vec![HourVerdict {
                start,
                verdict,
                reasons: Vec::new(),
            }],
        }
    }

    #[test]
    fn a_trip_turning_go_fires() {
        let rules = [rule(trip(), vec![Condition::TripGo {}])];
        let now = at("2026-11-08T14:30:00Z");
        let with_days = |days: Vec<DayVerdict>| {
            let mut inputs = trip_inputs(Vec::new(), now);
            inputs.trips.get_mut("trip-1").unwrap().days = days;
            inputs
        };
        let (_, state) = once(&rules, &with_days(vec![day(9, Verdict::Unknown)]), now);
        assert_eq!(status(&state), RuleStatus::NoData { condition: 0 });
        let (events, state) =
            evaluate(&rules, &with_days(vec![day(9, Verdict::NoGo)]), &state, now);
        assert_eq!(events, []);
        assert_eq!(status(&state), RuleStatus::NotMet);
        let days = vec![day(9, Verdict::NoGo), day(10, Verdict::Go)];
        let (events, _) = evaluate(&rules, &with_days(days), &state, now);
        assert_eq!(events.len(), 1);
        let f = &events[0].findings[0];
        assert_eq!(
            (f.reason, f.at),
            (FindingReason::TripGo, Some(at("2026-11-10T05:00:00Z")))
        );

        // On a spot, go/no-go has no data.
        let on_spot = [rule(spot("battery"), vec![Condition::TripGo {}])];
        let (_, state) = once(&on_spot, &with_days(Vec::new()), now);
        assert_eq!(status(&state), RuleStatus::NoData { condition: 0 });
    }

    fn window(start: UnixTime, hours: i64) -> FishingWindow {
        FishingWindow {
            date: LocalDate {
                year: 2026,
                month: 11,
                day: 8,
            },
            start,
            end: after(start, hours * 3_600),
            matched: Vec::new(),
            weight_sum: 2.0,
            no_data: Vec::new(),
        }
    }

    #[test]
    fn a_window_starting_soon_fires_once_per_window() {
        let rules = [rule(
            spot("battery"),
            vec![Condition::WindowStarts { within_minutes: 30 }],
        )];
        let first = at("2026-11-08T18:00:00Z");
        let second = at("2026-11-09T06:00:00Z");
        let inputs = |now: UnixTime| {
            let mut i = trip_inputs(Vec::new(), now);
            i.spots.get_mut("battery").unwrap().windows = vec![window(first, 2), window(second, 2)];
            i
        };
        let times = [
            after(first, -45 * MIN),
            after(first, -20 * MIN),
            after(first, 30 * MIN),
            after(first, 3 * 3_600),
            after(second, -10 * MIN),
        ];
        let (events, _) = replay(&rules, times, inputs);
        let got: Vec<_> = events.iter().map(|(t, e)| (*t, e.findings[0].at)).collect();
        assert_eq!(got, [(times[1], Some(first)), (times[4], Some(second))]);
    }

    #[test]
    fn a_storm_ahead_or_a_hurricane_cone_fires() {
        let rules = [rule(spot("battery"), vec![Condition::StormAhead {}])];
        let now = storm_data::STORM_NOW;
        let with_storm = |outlook, hurricanes, checked_at| {
            let mut snap = SpotSnapshot::new(sample::spot(), now);
            snap.storm = Some(StormWatch::from_outlook(checked_at, outlook, hurricanes));
            spot_inputs(snap)
        };

        let storm = with_storm(storm_data::noreaster(), Vec::new(), now);
        let (events, _) = once(&rules, &storm, now);
        let f = &events[0].findings[0];
        let event = significant_event(&storm_data::noreaster(), now).unwrap();
        assert_eq!(f.reason, FindingReason::StormAhead);
        assert_eq!((f.at, f.until), (Some(event.start), Some(event.end)));

        let dry = with_storm(storm_data::dry_week(), Vec::new(), now);
        let (events, state) = once(&rules, &dry, now);
        assert_eq!(events, []);
        assert_eq!(status(&state), RuleStatus::NotMet);

        let hurricane = storm_data::hurricane();
        let cone = with_storm(storm_data::dry_week(), vec![hurricane.clone()], now);
        let (events, _) = once(&rules, &cone, now);
        let f = &events[0].findings[0];
        assert_eq!(f.reason, FindingReason::Hurricane);
        assert_eq!(f.subject.as_deref(), Some(hurricane.name.as_str()));
        assert_eq!(f.source, Some(SourceId::new("nhc")));

        let old = with_storm(storm_data::noreaster(), Vec::new(), after(now, -10 * 3_600));
        let (events, state) = once(&rules, &old, now);
        assert_eq!(events, []);
        assert!(matches!(
            status(&state),
            RuleStatus::Unchecked {
                problem: CheckProblem::Stale { .. },
                ..
            }
        ));
    }

    fn box_rule(all_of: Vec<Condition>) -> AlertRule {
        rule(
            AlertTarget::Box {
                south: 40.0,
                west: -73.5,
                north: 40.5,
                east: -73.0,
            },
            all_of,
        )
    }

    fn box_inputs(cells: &[f32], time: UnixTime) -> EvalInputs {
        let sst = BoxValues {
            source: SourceId::new("mur-sst"),
            time,
            cells: cells.to_vec(),
        };
        EvalInputs {
            boxes: BTreeMap::from([(
                "r1".into(),
                BoxInputs {
                    time_zone: "America/New_York".into(),
                    values: BTreeMap::from([(AlertMetric::SeaSurfaceTemp, sst)]),
                },
            )]),
            ..EvalInputs::default()
        }
    }

    #[test]
    fn a_box_fires_when_any_cell_meets_and_rearms_when_none_is_near() {
        let rules = [box_rule(vec![measurement(
            AlertMetric::SeaSurfaceTemp,
            None,
            Some(20.0),
        )])];
        let grid_time = at("2026-08-10T09:00:00Z");
        let now = after(grid_time, 30 * 3_600);
        let run = |cells: &[f32], state: &AlertState| {
            evaluate(&rules, &box_inputs(cells, grid_time), state, now)
        };

        let (events, state) = run(&[18.5, f32::NAN, 19.2, 20.4], &AlertState::default());
        let f = &events[0].findings[0];
        assert_eq!(f.measured, Some(f64::from(20.4_f32)));
        assert_eq!(f.source, Some(SourceId::new("mur-sst")));
        assert_eq!(f.spot_id, None);
        let (_, state) = run(&[19.5, 19.8], &state);
        assert!(
            state.rules["r1"].notified_at.is_some(),
            "19.8 °C is within 0.3 °C"
        );
        let (_, state) = run(&[19.5, 19.6], &state);
        assert!(state.rules["r1"].notified_at.is_none());

        // A grid four days old is stale.
        let late = after(grid_time, 4 * 24 * 3_600);
        let (events, state) = once(&rules, &box_inputs(&[21.0], grid_time), late);
        assert_eq!(events, []);
        assert_eq!(
            status(&state),
            RuleStatus::Unchecked {
                condition: 0,
                problem: CheckProblem::Stale {
                    age_s: 4 * 24 * 3_600,
                    limit_s: GRIDDED_MAX_AGE_S
                }
            }
        );

        // Only gridded metrics apply; a box without values has no data.
        let wind = [box_rule(vec![measurement(
            AlertMetric::Wind,
            None,
            Some(5.0),
        )])];
        let (_, state) = once(&wind, &box_inputs(&[21.0], grid_time), now);
        assert_eq!(status(&state), RuleStatus::NoData { condition: 0 });
        let (_, state) = once(&rules, &EvalInputs::default(), now);
        assert_eq!(status(&state), RuleStatus::NoData { condition: 0 });
    }

    #[test]
    fn a_trip_needs_every_place_condition_at_one_place() {
        // Low flow at the Croton and an advisory at The Battery: no single place has both.
        let now = at("2026-11-08T14:30:00Z");
        let mut inputs = trip_inputs(anz338_alerts(), now);
        let flow = synthetic_cfs(now, &[20.0]);
        inputs.spots.extend(flow_inputs(&flow, now).spots);
        inputs.trips.get_mut("trip-1").unwrap().places = vec!["croton".into(), "battery".into()];
        let both = AlertRule {
            all_of: vec![
                flow_below(50.0),
                Condition::MarineAlert {
                    event: "Small Craft Advisory".into(),
                },
            ],
            ..sca_rule()
        };
        let (events, _) = once(&[both], &inputs, now);
        assert_eq!(events, []);

        // The advisory alone holds at The Battery.
        let (events, _) = once(&[sca_rule()], &inputs, now);
        assert_eq!(events[0].findings[0].spot_id.as_deref(), Some("battery"));
    }

    // --- State -------------------------------------------------------------------------------

    #[test]
    fn state_is_kept_by_rule_id_and_dropped_with_the_rule() {
        let now = at("2026-11-08T00:00:00Z");
        let low = synthetic_cfs(now, &[20.0]);
        let inputs = flow_inputs(&low, now);
        let named = |id: &str, target| AlertRule {
            id: id.into(),
            ..rule(target, vec![flow_below(50.0)])
        };
        let rules = [
            named("b", spot("croton")),
            named("a", spot("croton")),
            named("gone", spot("deleted")),
            AlertRule {
                enabled: false,
                ..named("off", spot("croton"))
            },
        ];
        let mut before = AlertState::default();
        before.rules.insert("removed".into(), RuleState::new(now));
        let (events, state) = evaluate(&rules, &inputs, &before, now);
        let fired: Vec<_> = events.iter().map(|e| e.rule_id.as_str()).collect();
        assert_eq!(fired, ["b", "a"]);
        assert_eq!(state.rules.keys().collect::<Vec<_>>(), ["a", "b", "gone"]);
        assert_eq!(state.rules["gone"].status, RuleStatus::Orphaned);

        // Serialized in key order, and read back whole.
        let text = serde_json::to_string(&state).unwrap();
        assert!(text.find("\"a\"").unwrap() < text.find("\"b\"").unwrap());
        assert_eq!(serde_json::from_str::<AlertState>(&text).unwrap(), state);
        assert_eq!(
            serde_json::to_value(&state.rules["a"]).unwrap(),
            json!({
                "status": { "kind": "met" },
                "checked_at": now.0,
                "met_since": now.0,
                "notified_at": now.0
            })
        );
        assert_eq!(serde_json::to_string(&AlertState::default()).unwrap(), "{}");
    }

    #[test]
    fn a_held_notification_survives_a_round_trip() {
        let mut r = rule(spot("croton"), vec![flow_below(50.0)]);
        r.quiet_hours = Some(quiet(22, 7));
        let rules = [r];
        let t0 = at("2026-11-09T03:30:00Z");
        let low = synthetic_cfs(t0, &[20.0]);
        let (_, state) = once(&rules, &flow_inputs(&low, t0), t0);
        let text = serde_json::to_string(&state).unwrap();
        let back: AlertState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, state);
        let t1 = at("2026-11-09T12:30:00Z");
        let (events, _) = evaluate(&rules, &flow_inputs(&low, t1), &back, t1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].found_at, t0);
    }
}
