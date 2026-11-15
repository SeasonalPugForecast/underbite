//! Fishing-window factor weights, solunar durations and presentation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use underbite_core::view::{TimelineSpan, Voice};
use whether_core::presentation::TimeFormat;

/// Weights of the factors that line up into a fishing window; 0 turns a factor off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WindowFactors {
    pub tide_stage: f64,
    pub low_light: f64,
    pub solunar: f64,
    pub pressure_falling: f64,
    pub water_temp: f64,
}

impl Default for WindowFactors {
    fn default() -> Self {
        Self {
            tide_stage: 1.0,
            low_light: 1.0,
            solunar: 0.5,
            pressure_falling: 0.7,
            water_temp: 1.0,
        }
    }
}

impl WindowFactors {
    /// Every factor with its name, for validation.
    pub(crate) fn named(&self) -> [(&'static str, f64); 5] {
        [
            ("tide_stage", self.tide_stage),
            ("low_light", self.low_light),
            ("solunar", self.solunar),
            ("pressure_falling", self.pressure_falling),
            ("water_temp", self.water_temp),
        ]
    }
}

/// `[windows]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WindowsConfig {
    pub factors: WindowFactors,
    /// Whether the solunar periods are shown at all.
    pub show_solunar: bool,
    /// Major solunar period length in minutes.
    pub solunar_major_minutes: u32,
    /// Minor solunar period length in minutes.
    pub solunar_minor_minutes: u32,
}

impl Default for WindowsConfig {
    fn default() -> Self {
        Self {
            factors: WindowFactors::default(),
            show_solunar: true,
            solunar_major_minutes: 120,
            solunar_minor_minutes: 60,
        }
    }
}

/// `[presentation]`: how the app speaks and shows time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PresentationConfig {
    /// Playful labels (`luna`) or plain ones (`plain`); safety text is always plain.
    pub voice: Voice,
    /// `12h` or `24h`.
    pub clock: TimeFormat,
    /// Hours the timeline and charts reach.
    pub timeline_hours: u16,
    /// Minutes between timeline points.
    pub timeline_step_minutes: u16,
}

impl Default for PresentationConfig {
    fn default() -> Self {
        Self {
            voice: Voice::Luna,
            clock: TimeFormat::H12,
            timeline_hours: 48,
            timeline_step_minutes: 30,
        }
    }
}

impl PresentationConfig {
    /// The span the view takes.
    pub fn timeline_span(&self) -> TimelineSpan {
        TimelineSpan {
            hours: self.timeline_hours,
            step_minutes: self.timeline_step_minutes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_tables_take_defaults() {
        let w: WindowsConfig = toml::from_str("[factors]\nsolunar = 0.2").unwrap();
        assert!((w.factors.solunar - 0.2).abs() < f64::EPSILON);
        assert!((w.factors.tide_stage - 1.0).abs() < f64::EPSILON);
        assert_eq!(w.solunar_major_minutes, 120);
    }

    #[test]
    fn voice_and_clock_parse() {
        let p: PresentationConfig = toml::from_str("voice = \"plain\"\nclock = \"24h\"").unwrap();
        assert_eq!((p.voice, p.clock), (Voice::Plain, TimeFormat::H24));
        assert_eq!(p.timeline_span().hours, 48);
    }
}
