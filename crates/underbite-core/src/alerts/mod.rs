//! Alert rules: what a user asks to be told about, and their evaluation against fresh data.

pub mod eval;
pub mod rules;

pub use eval::{
    AlertEvent, AlertState, BoxInputs, BoxValues, CheckProblem, EvalInputs, EventKind, Finding,
    FindingReason, RuleState, RuleStatus, SpotInputs, ToldAlert, TripInputs, evaluate,
};
pub use rules::{AlertMetric, AlertRule, AlertTarget, Condition, QuietHours};
