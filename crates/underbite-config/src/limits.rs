//! Go/no-go profiles, fish-stress thresholds and the storm settings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::schema::Issues;

fn yes() -> bool {
    true
}

/// One `[[limits]]` go/no-go profile: the hard limits for a way of fishing. A limit left out
/// is not checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LimitsProfile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wind_kn: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gust_kn: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wave_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_current_kn: Option<f64>,
    /// Coldest air temperature, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_air_temp_c: Option<f64>,
    /// Least visibility, km.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_visibility_km: Option<f64>,
    /// No-go while a small-craft advisory is in force.
    #[serde(default)]
    pub no_small_craft_advisory: bool,
    /// No-go with lightning near the spot.
    #[serde(default = "yes")]
    pub no_lightning: bool,
}

impl LimitsProfile {
    fn of(name: &str, wind: f64, gust: f64, wave: f64, current: Option<f64>) -> Self {
        Self {
            name: name.to_owned(),
            max_wind_kn: Some(wind),
            max_gust_kn: Some(gust),
            max_wave_m: Some(wave),
            max_current_kn: current,
            min_air_temp_c: None,
            min_visibility_km: None,
            no_small_craft_advisory: true,
            no_lightning: true,
        }
    }

    /// The profiles a new install starts with.
    pub fn examples() -> Vec<Self> {
        vec![
            Self::of("Kayak", 12.0, 18.0, 0.6, Some(2.0)),
            Self::of("20 ft boat", 20.0, 25.0, 1.2, None),
            Self::of("Wading", 15.0, 22.0, 0.3, Some(1.5)),
        ]
    }

    pub(crate) fn validate(&self, path: &str, issues: &mut Issues) {
        if self.name.trim().is_empty() {
            issues.error(format!("{path}.name"), "a profile needs a name");
        }
        let maxima = [
            ("max_wind_kn", self.max_wind_kn),
            ("max_gust_kn", self.max_gust_kn),
            ("max_wave_m", self.max_wave_m),
            ("max_current_kn", self.max_current_kn),
            ("min_visibility_km", self.min_visibility_km),
        ];
        for (key, v) in maxima {
            if let Some(v) = v
                && !(v.is_finite() && v >= 0.0)
            {
                issues.error(format!("{path}.{key}"), "must be 0 or more");
            }
        }
        if let (Some(w), Some(g)) = (self.max_wind_kn, self.max_gust_kn)
            && g < w
        {
            issues.warning(
                format!("{path}.max_gust_kn"),
                "the gust limit is below the wind limit, so the wind limit never applies",
            );
        }
    }
}

/// `[thresholds]`: levels where fish or fishing change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    /// Water temperature above which trout fishing is discouraged, °C.
    pub trout_stress_c: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            trout_stress_c: 20.0,
        }
    }
}

/// `[storms]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct StormsConfig {
    /// Hurricanes farther than this from the spot are ignored, km.
    pub hurricane_distance_km: f64,
}

impl Default for StormsConfig {
    fn default() -> Self {
        Self {
            hurricane_distance_km: 3000.0,
        }
    }
}

impl StormsConfig {
    pub(crate) fn validate(&self, issues: &mut Issues) {
        if !(self.hurricane_distance_km.is_finite() && self.hurricane_distance_km > 0.0) {
            issues.error("storms.hurricane_distance_km", "must be greater than 0");
        }
    }
}

impl Thresholds {
    pub(crate) fn validate(&self, issues: &mut Issues) {
        if !(-5.0..=40.0).contains(&self.trout_stress_c) {
            issues.error("thresholds.trout_stress_c", "must be between -5 and 40 °C");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_example_profile_parses() {
        let p: LimitsProfile = toml::from_str(
            "name = \"Kayak\"\nmax_wind_kn = 12\nmax_gust_kn = 18\nmax_wave_m = 0.6\nno_small_craft_advisory = true",
        )
        .unwrap();
        assert_eq!(p.max_wind_kn, Some(12.0));
        assert!(p.no_lightning);
        assert_eq!(p.max_current_kn, None);
    }

    #[test]
    fn examples_validate_and_bad_values_do_not() {
        let mut issues = Issues::default();
        for (i, p) in LimitsProfile::examples().iter().enumerate() {
            p.validate(&format!("limits[{i}]"), &mut issues);
        }
        assert_eq!(issues.0.len(), 0);
        let mut bad = LimitsProfile::examples().remove(0);
        bad.max_wave_m = Some(-1.0);
        bad.max_gust_kn = Some(5.0);
        bad.validate("limits[0]", &mut issues);
        assert_eq!(issues.0.len(), 2);
        assert_eq!(issues.0[0].path, "limits[0].max_wave_m");
    }

    #[test]
    fn storm_defaults() {
        assert!((StormsConfig::default().hurricane_distance_km - 3000.0).abs() < f64::EPSILON);
        assert!((Thresholds::default().trout_stress_c - 20.0).abs() < f64::EPSILON);
    }
}
