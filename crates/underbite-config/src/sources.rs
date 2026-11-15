//! Source chains per measurement group, provider switches and the location privacy setting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use underbite_core::provenance::SourceId;

use crate::schema::Issues;

/// Ids the app knows or plans to implement. An id outside this list is a warning, and an id
/// that is listed but not implemented yet is skipped at run time without an error.
pub const KNOWN_SOURCES: [&str; 18] = [
    "usgs",
    "coops",
    "coops-harmonics",
    "ndbc",
    "nwps",
    "nws",
    "glerl",
    "mur-sst",
    "hf-radar",
    "copernicus",
    "open-meteo-marine",
    "open-meteo-storm",
    "nhc",
    "openweather",
    "open-meteo",
    "met-norway",
    "acis",
    "pirate-weather",
];

/// The most decimals of a coordinate that ever leave the device (about 1 km).
pub const MAX_COORDINATE_PRECISION: u8 = 2;

/// One `[sources.<group>]`: an ordered chain (the first source that answers wins, the rest fill
/// gaps) and the limits past which a value is greyed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceGroup {
    /// Source ids in order of preference.
    pub chain: Vec<SourceId>,
    /// Stations farther than this are greyed; none means no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_distance_km: Option<f64>,
    /// Observations older than this are greyed; none means no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_minutes: Option<u32>,
}

impl SourceGroup {
    fn of(chain: &[&str]) -> Self {
        Self {
            chain: chain.iter().map(|id| SourceId::new(*id)).collect(),
            max_distance_km: None,
            max_age_minutes: None,
        }
    }

    fn validate(&self, path: &str, issues: &mut Issues) {
        if self.chain.is_empty() {
            issues.warning(
                format!("{path}.chain"),
                "the chain is empty, so this group shows nothing",
            );
        }
        for (i, id) in self.chain.iter().enumerate() {
            let s = id.as_str();
            let at = format!("{path}.chain[{i}]");
            if s.is_empty()
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                issues.error(
                    at,
                    format!("`{s}` is not a source id (lowercase, digits and `-`)"),
                );
            } else if !KNOWN_SOURCES.contains(&s) {
                issues.warning(
                    at,
                    format!("`{s}` is not a known source and will be skipped"),
                );
            } else if self.chain[..i].contains(id) {
                issues.warning(at, format!("`{s}` is listed twice"));
            }
        }
        if let Some(km) = self.max_distance_km
            && !(km.is_finite() && km > 0.0)
        {
            issues.error(format!("{path}.max_distance_km"), "must be greater than 0");
        }
        if self.max_age_minutes == Some(0) {
            issues.error(format!("{path}.max_age_minutes"), "must be greater than 0");
        }
    }
}

/// `[sources]`: one chain per measurement group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SourcesConfig {
    /// Decimals of latitude and longitude sent to data providers: 0, 1 or 2.
    pub coordinate_precision: u8,
    pub water_temperature: SourceGroup,
    pub tide: SourceGroup,
    pub currents: SourceGroup,
    /// Discharge, gage height, flow forecast and flood categories.
    pub flow: SourceGroup,
    pub lake_level: SourceGroup,
    pub waves: SourceGroup,
    /// Sea surface temperature.
    pub sst: SourceGroup,
    pub clarity: SourceGroup,
    pub water_quality: SourceGroup,
    /// Marine forecast and alerts.
    pub marine_forecast: SourceGroup,
    /// Weather providers, by their ids in the weather library.
    pub weather: SourceGroup,
    /// The storm-ahead check and hurricane tracks.
    pub storm: SourceGroup,
}

impl Default for SourcesConfig {
    fn default() -> Self {
        let mut water_temperature = SourceGroup::of(&[
            "usgs",
            "coops",
            "ndbc",
            "glerl",
            "open-meteo-marine",
            "mur-sst",
        ]);
        water_temperature.max_distance_km = Some(25.0);
        water_temperature.max_age_minutes = Some(180);
        Self {
            coordinate_precision: MAX_COORDINATE_PRECISION,
            water_temperature,
            tide: SourceGroup::of(&["coops", "coops-harmonics", "open-meteo-marine"]),
            currents: SourceGroup::of(&["coops", "hf-radar", "copernicus", "open-meteo-marine"]),
            flow: SourceGroup::of(&["usgs", "nwps"]),
            lake_level: SourceGroup::of(&["usgs", "coops", "glerl"]),
            waves: SourceGroup::of(&["ndbc", "open-meteo-marine"]),
            sst: SourceGroup::of(&["mur-sst", "open-meteo-marine"]),
            clarity: SourceGroup::of(&["usgs", "copernicus"]),
            water_quality: SourceGroup::of(&["usgs"]),
            marine_forecast: SourceGroup::of(&["nws"]),
            weather: SourceGroup::of(&["nws", "open-meteo"]),
            storm: SourceGroup::of(&["open-meteo-storm", "nhc"]),
        }
    }
}

impl SourcesConfig {
    /// The groups with their table names.
    pub fn groups(&self) -> [(&'static str, &SourceGroup); 12] {
        [
            ("water_temperature", &self.water_temperature),
            ("tide", &self.tide),
            ("currents", &self.currents),
            ("flow", &self.flow),
            ("lake_level", &self.lake_level),
            ("waves", &self.waves),
            ("sst", &self.sst),
            ("clarity", &self.clarity),
            ("water_quality", &self.water_quality),
            ("marine_forecast", &self.marine_forecast),
            ("weather", &self.weather),
            ("storm", &self.storm),
        ]
    }

    pub(crate) fn validate(&self, issues: &mut Issues) {
        if self.coordinate_precision > MAX_COORDINATE_PRECISION {
            issues.error(
                "sources.coordinate_precision",
                format!("at most {MAX_COORDINATE_PRECISION} decimals (about 1 km) may be sent"),
            );
        }
        for (name, group) in self.groups() {
            group.validate(&format!("sources.{name}"), issues);
        }
    }
}

/// `[providers.copernicus]`: opt-in, needs the user's own free account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CopernicusConfig {
    pub enabled: bool,
}

/// `[providers.usgs]`. It has no keys: an optional API key lives only in the keychain (iOS) or
/// `USGS_API_KEY` (CLI), never in a file or an export.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct UsgsConfig {}

/// `[providers]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProvidersConfig {
    pub usgs: UsgsConfig,
    pub copernicus: CopernicusConfig,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Severity;

    fn issues_of(s: &SourcesConfig) -> Issues {
        let mut i = Issues::default();
        s.validate(&mut i);
        i
    }

    #[test]
    fn defaults_are_clean_and_keep_unimplemented_ids() {
        let s = SourcesConfig::default();
        assert_eq!(issues_of(&s).0.len(), 0);
        let ids: Vec<&str> = s
            .water_temperature
            .chain
            .iter()
            .map(SourceId::as_str)
            .collect();
        assert_eq!(
            ids,
            [
                "usgs",
                "coops",
                "ndbc",
                "glerl",
                "open-meteo-marine",
                "mur-sst"
            ]
        );
        assert_eq!(s.coordinate_precision, 2);
        assert!(s.storm.chain.contains(&SourceId::new("open-meteo-storm")));
    }

    #[test]
    fn bad_ids_and_limits_are_reported_with_paths() {
        let mut s = SourcesConfig::default();
        s.tide.chain = vec![
            SourceId::new("Coops"),
            SourceId::new("tidal-fairy"),
            SourceId::new("usgs"),
            SourceId::new("usgs"),
        ];
        s.flow.max_distance_km = Some(0.0);
        s.coordinate_precision = 3;
        let i = issues_of(&s).0;
        let at = |p: &str, sev| i.iter().any(|x| x.path == p && x.severity == sev);
        assert!(at("sources.tide.chain[0]", Severity::Error));
        assert!(at("sources.tide.chain[1]", Severity::Warning));
        assert!(at("sources.tide.chain[3]", Severity::Warning));
        assert!(at("sources.flow.max_distance_km", Severity::Error));
        assert!(at("sources.coordinate_precision", Severity::Error));
    }

    #[test]
    fn a_key_field_is_rejected() {
        assert!(toml::from_str::<ProvidersConfig>("[usgs]\napi_key = \"x\"").is_err());
    }
}
