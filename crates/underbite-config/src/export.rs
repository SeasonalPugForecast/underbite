//! Import and export as TOML or JSON.
//!
//! TOML is the file the user exports and shares; JSON is the form the app stores its preferences
//! in. Both carry exactly the schema's fields, so an export holds no key, token or location, and
//! an import is refused if the file does.

use serde_json::Value;

use crate::migrate::{ImportError, find_key_like, prepare};
use crate::schema::{Config, ConfigError, Issue};

const HEADER: &str = "# Underbite settings\n\
# Units, sources, cards, limits, rivers, windows and map defaults.\n\
# Keys and passwords are never written here; they stay in the keychain or the environment.\n\n";

/// Writing a configuration failed.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Toml(#[from] ConfigError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A field that looks like a secret reached the output; nothing is written.
    #[error("`{0}` looks like a key or password and was not exported")]
    KeyLike(String),
}

fn guard(config: &Config) -> Result<(), ExportError> {
    match find_key_like(&serde_json::to_value(config)?) {
        Some(path) => Err(ExportError::KeyLike(path)),
        None => Ok(()),
    }
}

/// The user-facing export: TOML with a short comment header, in schema order.
pub fn to_toml(config: &Config) -> Result<String, ExportError> {
    guard(config)?;
    Ok(format!("{HEADER}{}", config.to_toml()?))
}

/// The app's storage form: pretty JSON in schema order.
pub fn to_json(config: &Config) -> Result<String, ExportError> {
    guard(config)?;
    Ok(serde_json::to_string_pretty(config)?)
}

fn finish(mut doc: Value) -> Result<(Config, Vec<Issue>), ImportError> {
    let mut issues = prepare(&mut doc)?;
    let config: Config =
        serde_json::from_value(doc).map_err(|e| ImportError::Invalid(e.to_string()))?;
    issues.extend(config.validate());
    Ok((config, issues))
}

/// Reads a TOML file. Older versions are upgraded, a newer one is refused, a key-like or unknown
/// key is refused with its path, and what was dropped or clamped comes back as warnings together
/// with the validation issues (the caller checks `has_errors`).
pub fn parse_toml(text: &str) -> Result<(Config, Vec<Issue>), ImportError> {
    let doc: Value = toml::from_str(text).map_err(|e| ImportError::Syntax(e.to_string()))?;
    finish(doc)
}

/// Reads the JSON form, with the same rules as [`parse_toml`].
pub fn parse_json(text: &str) -> Result<(Config, Vec<Issue>), ImportError> {
    let doc: Value = serde_json::from_str(text).map_err(|e| ImportError::Syntax(e.to_string()))?;
    finish(doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Severity, has_errors};

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
    fn toml_round_trip_is_stable() {
        let config = Config::default();
        let first = to_toml(&config).unwrap();
        assert!(first.starts_with("# Underbite settings"));
        let (back, issues) = parse_toml(&first).unwrap();
        assert_eq!(back, config);
        assert_eq!(issues, []);
        assert_eq!(to_toml(&back).unwrap(), first);
    }

    #[test]
    fn json_round_trip_is_stable() {
        let (config, _) = parse_toml(ARCH_EXAMPLE).unwrap();
        let first = to_json(&config).unwrap();
        let (back, issues) = parse_json(&first).unwrap();
        assert_eq!(back, config);
        assert_eq!(issues, []);
        assert_eq!(to_json(&back).unwrap(), first);
    }

    #[test]
    fn toml_and_json_agree() {
        let (config, _) = parse_toml(ARCH_EXAMPLE).unwrap();
        let (from_json, _) = parse_json(&to_json(&config).unwrap()).unwrap();
        let (from_toml, _) = parse_toml(&to_toml(&config).unwrap()).unwrap();
        assert_eq!(from_json, from_toml);
    }

    #[test]
    fn arch_example_imports_clean() {
        let (config, issues) = parse_toml(ARCH_EXAMPLE).unwrap();
        assert_eq!(issues, []);
        assert_eq!(config.rivers[0].best_cfs, Some([400.0, 900.0]));
    }

    #[test]
    fn empty_and_version_less_files_decode_with_defaults() {
        assert_eq!(parse_toml("").unwrap().0, Config::default());
        assert_eq!(parse_json("{}").unwrap().0, Config::default());
        assert_eq!(parse_json("{\"version\":0}").unwrap().0, Config::default());
    }

    #[test]
    fn newer_files_are_refused() {
        let e = parse_toml("version = 2").unwrap_err();
        assert!(e.to_string().contains("newer Underbite"), "{e}");
    }

    #[test]
    fn key_like_content_is_refused_naming_the_path() {
        for (text, path) in [
            (
                "[providers.usgs]\napi_key = \"x\"",
                "providers.usgs.api_key",
            ),
            ("token = \"x\"", "token"),
            ("[map]\npassword = \"x\"", "map.password"),
            ("[[limits]]\nname = \"a\"\nkey = 1", "limits[0].key"),
        ] {
            let e = parse_toml(text).unwrap_err();
            assert_eq!(e, ImportError::KeyLike { path: path.into() }, "{text}");
            assert!(e.to_string().contains(path));
        }
    }

    #[test]
    fn unknown_keys_are_refused_with_the_path() {
        let e = parse_toml("[sources.tide]\nchain = []\nbogus = 1").unwrap_err();
        assert!(e.to_string().contains("sources.tide.bogus"), "{e}");
        let e = parse_json("{\"windows\":{\"factors\":{\"lunar\":1}}}").unwrap_err();
        assert!(e.to_string().contains("windows.factors.lunar"), "{e}");
    }

    #[test]
    fn dropped_ids_and_clamped_precision_are_reported() {
        let (config, issues) = parse_toml(
            "[sources]\ncoordinate_precision = 5\n[sources.tide]\nchain = [\"coops\", \"mystery\"]\n\
             [cards]\norder = [\"tide\", \"unicorn\"]",
        )
        .unwrap();
        assert_eq!(config.sources.coordinate_precision, 2);
        assert_eq!(config.sources.tide.chain.len(), 1);
        assert_eq!(config.cards.order.len(), 1);
        assert!(!has_errors(&issues));
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "sources.coordinate_precision",
                "sources.tide.chain[1]",
                "cards.order[1]"
            ]
        );
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
    }

    #[test]
    fn bad_values_come_back_as_errors() {
        let (_, issues) = parse_toml("[windows.factors]\nsolunar = 2").unwrap();
        assert!(has_errors(&issues));
        assert!(matches!(
            parse_toml("[units]\nspeed = \"furlongs\""),
            Err(ImportError::Invalid(_))
        ));
        assert!(matches!(parse_toml("= nope"), Err(ImportError::Syntax(_))));
    }

    #[test]
    fn exports_carry_no_key_or_coordinates() {
        let (config, _) = parse_toml(ARCH_EXAMPLE).unwrap();
        for text in [to_toml(&config).unwrap(), to_json(&config).unwrap()] {
            let body: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
            let lower = body.join("\n").to_ascii_lowercase();
            for word in [
                "key",
                "token",
                "password",
                "secret",
                "latitude",
                "longitude",
            ] {
                assert!(!lower.contains(word), "{word}");
            }
        }
    }
}
