//! Source chains per measurement group, provider switches and the location privacy setting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use underbite_core::discover::region::Region;
use underbite_core::provenance::SourceId;

use crate::schema::Issues;

/// Ids the app knows or plans to implement. An id outside this list is a warning, and an id
/// that is listed but not implemented yet is skipped at run time without an error.
pub const KNOWN_SOURCES: [&str; 24] = [
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
    "dfo-iwls",
    "eccc-hydrometric",
    "cwms",
    "rise",
    "cyan",
    "noaa-hab",
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

fn ids(chain: &[&str]) -> Vec<SourceId> {
    chain.iter().map(|id| SourceId::new(*id)).collect()
}

impl SourceGroup {
    fn of(chain: &[&str]) -> Self {
        Self {
            chain: ids(chain),
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
            lake_level: SourceGroup::of(&["usgs", "cwms", "rise", "coops", "glerl"]),
            waves: SourceGroup::of(&["ndbc", "open-meteo-marine"]),
            sst: SourceGroup::of(&["mur-sst", "open-meteo-marine"]),
            clarity: SourceGroup::of(&["usgs", "copernicus"]),
            water_quality: SourceGroup::of(&["usgs", "cyan", "noaa-hab"]),
            marine_forecast: SourceGroup::of(&["nws"]),
            weather: SourceGroup::of(&["nws", "open-meteo"]),
            storm: SourceGroup::of(&["open-meteo-storm", "nhc"]),
        }
    }
}

impl SourcesConfig {
    /// The default chains for a spot in `region`. The United States keeps [`Default`]; Canada
    /// swaps in its tide and river-flow sources; elsewhere only gridded and satellite sources
    /// serve, so the groups that need stations (river flow, lake level, water quality, marine
    /// forecast) are empty, and the tide group is the modelled sea level alone.
    pub fn for_region(region: Region) -> Self {
        let mut s = Self::default();
        match region {
            Region::Us => {}
            Region::Canada => {
                s.tide = SourceGroup::of(&["dfo-iwls", "open-meteo-marine"]);
                s.flow = SourceGroup::of(&["eccc-hydrometric"]);
            }
            Region::Elsewhere => {
                s.water_temperature.chain = ids(&["open-meteo-marine", "mur-sst"]);
                s.tide = SourceGroup::of(&["open-meteo-marine"]);
                s.currents = SourceGroup::of(&["copernicus", "open-meteo-marine"]);
                s.flow = SourceGroup::of(&[]);
                s.lake_level = SourceGroup::of(&[]);
                s.waves = SourceGroup::of(&["open-meteo-marine"]);
                s.clarity = SourceGroup::of(&["copernicus"]);
                s.water_quality = SourceGroup::of(&[]);
                s.marine_forecast = SourceGroup::of(&[]);
                s.weather = SourceGroup::of(&["open-meteo"]);
            }
        }
        s
    }

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
    fn the_united_states_keeps_the_default_chains() {
        assert_eq!(
            SourcesConfig::for_region(Region::Us),
            SourcesConfig::default()
        );
    }

    fn chain_ids(g: &SourceGroup) -> Vec<&str> {
        g.chain.iter().map(SourceId::as_str).collect()
    }

    #[test]
    fn canada_swaps_in_its_tide_and_flow_sources_only() {
        let us = SourcesConfig::default();
        let ca = SourcesConfig::for_region(Region::Canada);
        assert_eq!(chain_ids(&ca.tide), ["dfo-iwls", "open-meteo-marine"]);
        assert_eq!(chain_ids(&ca.flow), ["eccc-hydrometric"]);
        assert_eq!(issues_of(&ca).0.len(), 0);
        let same = us
            .groups()
            .iter()
            .zip(ca.groups())
            .filter(|((_, x), (_, y))| x == y)
            .count();
        assert_eq!(same, 10);
    }

    #[test]
    fn elsewhere_has_only_gridded_and_satellite_sources() {
        let s = SourcesConfig::for_region(Region::Elsewhere);
        let stationed = [
            "usgs",
            "coops",
            "coops-harmonics",
            "ndbc",
            "nwps",
            "nws",
            "glerl",
            "dfo-iwls",
            "eccc-hydrometric",
            "cwms",
            "rise",
            "cyan",
            "noaa-hab",
        ];
        for (name, group) in s.groups() {
            for id in chain_ids(group) {
                assert!(!stationed.contains(&id), "{name}: {id}");
            }
        }
        assert_eq!(chain_ids(&s.tide), ["open-meteo-marine"]);
        assert_eq!(chain_ids(&s.sst), ["mur-sst", "open-meteo-marine"]);
        // Only the groups with no gridded source are empty, and they say so.
        let empty: Vec<&str> = s
            .groups()
            .into_iter()
            .filter(|(_, g)| g.chain.is_empty())
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            empty,
            ["flow", "lake_level", "water_quality", "marine_forecast"]
        );
        let issues = issues_of(&s).0;
        assert_eq!(issues.len(), empty.len());
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
    }

    #[test]
    fn reservoir_and_bloom_sources_join_the_default_chains() {
        let s = SourcesConfig::default();
        assert_eq!(
            chain_ids(&s.lake_level),
            ["usgs", "cwms", "rise", "coops", "glerl"]
        );
        // CyAN before the Gulf forecast: lakes have only CyAN, and a Gulf coast the forecast.
        assert_eq!(chain_ids(&s.water_quality), ["usgs", "cyan", "noaa-hab"]);
        assert_eq!(issues_of(&s).0.len(), 0);
        for id in ["cwms", "rise", "cyan", "noaa-hab"] {
            assert!(KNOWN_SOURCES.contains(&id), "{id}");
        }
    }

    #[test]
    fn the_new_ids_are_known() {
        assert!(KNOWN_SOURCES.contains(&"dfo-iwls"));
        assert!(KNOWN_SOURCES.contains(&"eccc-hydrometric"));
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
