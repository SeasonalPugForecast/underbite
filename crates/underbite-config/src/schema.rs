//! The configuration document and its JSON Schema.
//!
//! Every section has defaults, so an old file (or an empty one) always loads; unknown keys are
//! rejected with the key named. Semantic checks that serde cannot make live in
//! [`Config::validate`].

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cards::CardsConfig;
use crate::limits::{LimitsProfile, StormsConfig, Thresholds};
use crate::map::MapConfig;
use crate::rivers::RiverRange;
use crate::sources::{ProvidersConfig, SourcesConfig};
use crate::units::UnitsConfig;
use crate::windows::{PresentationConfig, WindowsConfig};

/// The schema version this build writes and reads.
pub const CURRENT_VERSION: u32 = 1;

/// How serious an [`Issue`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The configuration cannot be used.
    Error,
    /// The configuration works, perhaps not as intended.
    Warning,
}

/// One problem found by validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Dotted path of the offending key, for example `limits[0].max_wave_m`.
    pub path: String,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}: {}: {}", self.path, self.message)
    }
}

/// Whether any issue is an error.
pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == Severity::Error)
}

/// Collects issues with their paths.
#[derive(Debug, Default)]
pub(crate) struct Issues(pub(crate) Vec<Issue>);

impl Issues {
    pub(crate) fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Error, path, message);
    }

    pub(crate) fn warning(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.push(Severity::Warning, path, message);
    }

    fn push(&mut self, severity: Severity, path: impl Into<String>, message: impl Into<String>) {
        self.0.push(Issue {
            severity,
            path: path.into(),
            message: message.into(),
        });
    }
}

/// Reading or writing a configuration file failed.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Not valid TOML, or a key or value the schema does not allow (the message names it).
    #[error("{0}")]
    Parse(#[from] toml::de::Error),
    /// The configuration could not be written as TOML.
    #[error("{0}")]
    Write(#[from] toml::ser::Error),
}

/// The whole configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(title = "Underbite configuration")]
pub struct Config {
    /// Schema version, for migration.
    pub version: u32,
    pub units: UnitsConfig,
    pub sources: SourcesConfig,
    pub providers: ProvidersConfig,
    pub cards: CardsConfig,
    /// Go/no-go profiles.
    pub limits: Vec<LimitsProfile>,
    pub thresholds: Thresholds,
    pub storms: StormsConfig,
    /// The user's own flow ranges, by gauge.
    pub rivers: Vec<RiverRange>,
    pub windows: WindowsConfig,
    pub presentation: PresentationConfig,
    pub map: MapConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            units: UnitsConfig::default(),
            sources: SourcesConfig::default(),
            providers: ProvidersConfig::default(),
            cards: CardsConfig::default(),
            limits: LimitsProfile::examples(),
            thresholds: Thresholds::default(),
            storms: StormsConfig::default(),
            rivers: Vec::new(),
            windows: WindowsConfig::default(),
            presentation: PresentationConfig::default(),
            map: MapConfig::default(),
        }
    }
}

impl Config {
    /// Reads a TOML document; missing keys take their defaults.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(text)?)
    }

    /// Writes the document as TOML.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Every semantic problem, errors and warnings, in document order.
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Issues::default();
        if self.version != CURRENT_VERSION {
            issues.error(
                "version",
                format!("expected {CURRENT_VERSION}, found {}", self.version),
            );
        }
        self.sources.validate(&mut issues);
        self.cards.validate(&mut issues);
        for (i, profile) in self.limits.iter().enumerate() {
            profile.validate(&format!("limits[{i}]"), &mut issues);
            if self.limits[..i].iter().any(|p| p.name == profile.name) {
                issues.warning(format!("limits[{i}].name"), "another profile has this name");
            }
        }
        self.thresholds.validate(&mut issues);
        self.storms.validate(&mut issues);
        for (i, river) in self.rivers.iter().enumerate() {
            validate_river(river, &format!("rivers[{i}]"), &mut issues);
            if self.rivers[..i].iter().any(|r| r.site == river.site) {
                issues.warning(format!("rivers[{i}].site"), "this gauge is listed twice");
            }
        }
        self.validate_windows(&mut issues);
        self.validate_presentation(&mut issues);
        self.map.validate(&mut issues);
        issues.0
    }

    fn validate_windows(&self, issues: &mut Issues) {
        for (name, w) in self.windows.factors.named() {
            if !(w.is_finite() && (0.0..=1.0).contains(&w)) {
                issues.error(
                    format!("windows.factors.{name}"),
                    "must be between 0 (off) and 1",
                );
            }
        }
        if self.windows.solunar_major_minutes == 0 {
            issues.error("windows.solunar_major_minutes", "must be greater than 0");
        }
        if self.windows.solunar_minor_minutes == 0 {
            issues.error("windows.solunar_minor_minutes", "must be greater than 0");
        }
    }

    fn validate_presentation(&self, issues: &mut Issues) {
        let p = &self.presentation;
        if p.timeline_hours == 0 {
            issues.error("presentation.timeline_hours", "must be greater than 0");
        }
        if p.timeline_step_minutes == 0 {
            issues.error(
                "presentation.timeline_step_minutes",
                "must be greater than 0",
            );
        }
    }
}

fn validate_river(river: &RiverRange, path: &str, issues: &mut Issues) {
    let number = river.site.strip_prefix("usgs:");
    if !number.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
        issues.error(
            format!("{path}.site"),
            "expected `usgs:` and the gauge's site number, such as `usgs:01420500`",
        );
    }
    let values = [
        ("wadeable_below_cfs", river.wadeable_below_cfs),
        ("blown_out_above_cfs", river.blown_out_above_cfs),
    ];
    for (key, v) in values {
        if let Some(v) = v
            && !(v.is_finite() && v >= 0.0)
        {
            issues.error(format!("{path}.{key}"), "must be 0 or more");
        }
    }
    if let Some([lo, hi]) = river.best_cfs
        && !(lo.is_finite() && hi.is_finite() && 0.0 <= lo && lo <= hi)
    {
        issues.error(
            format!("{path}.best_cfs"),
            "expected [low, high] with low <= high",
        );
    }
    if let (Some(w), Some(b)) = (river.wadeable_below_cfs, river.blown_out_above_cfs)
        && w >= b
    {
        issues.error(
            format!("{path}.blown_out_above_cfs"),
            "must be above `wadeable_below_cfs`",
        );
    }
}

/// The JSON Schema (2020-12) of the configuration file.
pub fn json_schema() -> serde_json::Value {
    schemars::schema_for!(Config).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example in the architecture document, with its comments.
    const ARCH_EXAMPLE: &str = r#"
[units]
discharge = "cfs"
height = "ft"
speed = "kn"
water_temp = "F"

[sources.water_temperature]
chain = ["usgs", "coops", "ndbc", "glerl", "open-meteo-marine", "mur-sst"]
max_distance_km = 25
max_age_minutes = 180

[sources.tide]
chain = ["coops", "coops-harmonics", "open-meteo-marine"]

[sources.currents]
chain = ["coops", "hf-radar", "copernicus", "open-meteo-marine"]

[sources.flow]
chain = ["usgs", "nwps"]

[providers.usgs]

[providers.copernicus]
enabled = false

[cards]
order = ["windows", "tide", "current", "wind", "water_temp", "flow", "waves", "sun_moon", "sources"]

[cards.river]
order = ["flow", "water_temp", "clarity", "windows", "wind", "sun_moon", "stocking", "sources"]

[[limits]]
name = "Kayak"
max_wind_kn = 12
max_gust_kn = 18
max_wave_m = 0.6
no_small_craft_advisory = true

[[rivers]]
site = "usgs:01420500"
wadeable_below_cfs = 1200
best_cfs = [400, 900]

[windows]
factors = { tide_stage = 1.0, low_light = 1.0, solunar = 0.5, pressure_falling = 0.7, water_temp = 1.0 }

[map]
active_preset = "inshore-flats"
offline_max_mb = 1500
"#;

    #[test]
    fn defaults_validate_clean() {
        assert_eq!(Config::default().validate(), []);
    }

    #[test]
    fn default_config_round_trips_through_toml() {
        let c = Config::default();
        let text = c.to_toml().unwrap();
        assert_eq!(Config::from_toml(&text).unwrap(), c);
        let json: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(json, c);
    }

    #[test]
    fn empty_file_gives_the_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
        assert_eq!(Config::from_toml("version = 1").unwrap(), Config::default());
    }

    #[test]
    fn arch_example_parses_and_validates() {
        let c = Config::from_toml(ARCH_EXAMPLE).unwrap();
        assert_eq!(c.validate(), []);
        assert_eq!(c.rivers[0].best_cfs, Some([400.0, 900.0]));
        assert_eq!(c.limits.len(), 1);
        assert_eq!(c.sources.water_temperature.max_age_minutes, Some(180));
    }

    #[test]
    fn unknown_keys_are_rejected_and_named() {
        for (text, key) in [
            ("colour = 1", "colour"),
            ("[units]\nspeeed = \"kn\"", "speeed"),
            ("[sources.tide]\nchain = []\nbogus = 1", "bogus"),
            ("[map]\nzoom = 3", "zoom"),
            ("[providers.usgs]\napi_key = \"x\"", "api_key"),
            ("[[limits]]\nname = \"a\"\nmax_knots = 1", "max_knots"),
            ("[storms]\nradius = 1", "radius"),
        ] {
            let err = Config::from_toml(text).unwrap_err().to_string();
            assert!(err.contains(key), "{text}: {err}");
        }
    }

    #[test]
    fn unknown_card_is_a_warning_and_a_bad_value_an_error() {
        let c = Config::from_toml(
            "[cards]\norder = [\"tide\", \"mystery\"]\n[windows.factors]\nsolunar = 2",
        )
        .unwrap();
        let issues = c.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.severity == Severity::Warning && i.path == "cards.order[1]")
        );
        assert!(
            issues
                .iter()
                .any(|i| i.severity == Severity::Error && i.path == "windows.factors.solunar")
        );
        assert!(has_errors(&issues));
    }

    #[test]
    fn river_ranges_are_checked() {
        let c = Config::from_toml(
            "[[rivers]]\nsite = \"01420500\"\nbest_cfs = [900, 400]\nwadeable_below_cfs = 2000\nblown_out_above_cfs = 1000",
        )
        .unwrap();
        let paths: Vec<_> = c.validate().into_iter().map(|i| i.path).collect();
        assert_eq!(
            paths,
            [
                "rivers[0].site",
                "rivers[0].best_cfs",
                "rivers[0].blown_out_above_cfs"
            ]
        );
    }

    #[test]
    fn schema_is_stable() {
        insta::assert_json_snapshot!(json_schema());
    }
}
