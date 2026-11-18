//! Alert rules: what a user asks to be told about, and their evaluation against fresh data.

pub mod eval;
pub mod rules;

pub use rules::{AlertMetric, AlertRule, AlertTarget, Condition, QuietHours};
