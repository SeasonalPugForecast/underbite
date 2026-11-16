//! The settings calls: migration, validation, TOML export and import, the unit and card
//! catalogues, value entry and the sample settings resource.

use std::path::PathBuf;

use serde_json::{Value, json};
use underbite_ffi::settings::{
    IssueSeverity, SettingsError, card_catalogue, default_settings_json, display_stops,
    export_settings_toml, import_settings_toml, migrate_settings, parse_user_value,
    source_catalogue, unit_options, validate_settings,
};

fn settings(edit: impl FnOnce(&mut Value)) -> String {
    let mut v: Value = serde_json::from_str(&default_settings_json()).unwrap();
    edit(&mut v);
    v.to_string()
}

fn cfs() -> String {
    default_settings_json()
}

fn metric() -> String {
    settings(|v| {
        v["units"] = json!({ "discharge": "m3s", "height": "m", "speed": "ms", "water_temp": "C",
            "pressure": "hPa", "distance": "km" });
    })
}

#[test]
fn the_defaults_validate_and_round_trip() {
    let json = default_settings_json();
    assert_eq!(validate_settings(json.clone()), []);
    let migrated = migrate_settings(json.clone()).unwrap();
    assert_eq!(migrated.issues, []);
    assert_eq!(migrated.json, json);
}

#[test]
fn migration_drops_an_unknown_card_id_and_reports_it() {
    let old = settings(|v| {
        v["cards"]["order"] = json!(["now", "telepathy", "tide", "sources"]);
        v["cards"]["hidden"] = json!(["moon_phase_of_the_week"]);
    });
    let migrated = migrate_settings(old).unwrap();
    let doc: Value = serde_json::from_str(&migrated.json).unwrap();
    assert_eq!(doc["cards"]["order"], json!(["now", "tide", "sources"]));
    assert_eq!(doc["cards"]["hidden"], json!([]));
    let paths: Vec<_> = migrated.issues.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["cards.order[1]", "cards.hidden[0]"]);
    assert!(
        migrated
            .issues
            .iter()
            .all(|i| i.severity == IssueSeverity::Warning)
    );
    assert!(migrated.issues[0].message.contains("telepathy"));
}

#[test]
fn old_settings_decode_with_new_fields_defaulted() {
    let migrated = migrate_settings(r#"{ "units": { "speed": "mph" } }"#.into()).unwrap();
    let doc: Value = serde_json::from_str(&migrated.json).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["units"]["speed"], "mph");
    assert_eq!(doc["units"]["discharge"], "cfs");
    assert_eq!(doc["presentation"]["voice"], "luna");
    assert!(
        doc["sources"]["tide"]["chain"]
            .as_array()
            .is_some_and(|c| !c.is_empty())
    );
}

#[test]
fn the_voice_setting_exists_and_defaults_to_luna() {
    let doc: Value = serde_json::from_str(&default_settings_json()).unwrap();
    assert_eq!(doc["presentation"]["voice"], "luna");
    let plain = settings(|v| v["presentation"]["voice"] = json!("plain"));
    let kept: Value = serde_json::from_str(&migrate_settings(plain).unwrap().json).unwrap();
    assert_eq!(kept["presentation"]["voice"], "plain");
}

#[test]
fn unknown_sources_are_dropped_and_unknown_keys_refused() {
    let old = settings(|v| v["sources"]["tide"]["chain"] = json!(["coops", "tidebook"]));
    let migrated = migrate_settings(old).unwrap();
    assert!(!migrated.json.contains("tidebook"));
    assert_eq!(migrated.issues[0].path, "sources.tide.chain[1]");

    let bad = settings(|v| v["units"]["furlongs"] = json!(3));
    let SettingsError::Invalid { message } = migrate_settings(bad.clone()).unwrap_err();
    assert!(message.contains("units.furlongs"), "{message}");
    let issues = validate_settings(bad);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].severity, IssueSeverity::Error);
}

#[test]
fn settings_from_a_newer_version_are_refused() {
    let newer = settings(|v| v["version"] = json!(2));
    let SettingsError::Invalid { message } = migrate_settings(newer).unwrap_err();
    assert!(message.contains("newer Underbite"), "{message}");
}

#[test]
fn validation_reports_errors_with_their_paths() {
    let bad = settings(|v| v["storms"]["hurricane_distance_km"] = json!(0));
    let issues = validate_settings(bad);
    assert!(
        issues.iter().any(|i| i.severity == IssueSeverity::Error
            && i.path == "storms.hurricane_distance_km")
    );
}

/// The names on the left of every `=` in a TOML text.
fn toml_keys(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, _)| k.trim().to_lowercase())
        .collect()
}

#[test]
fn keys_never_enter_settings_or_their_toml() {
    // A document with a key-like field is refused in every direction.
    let with_key = settings(|v| v["providers"]["usgs"]["api_key"] = json!("0123456789abcdef"));
    assert!(migrate_settings(with_key.clone()).is_err());
    assert!(export_settings_toml(with_key.clone()).is_err());
    assert!(import_settings_toml("[providers.usgs]\napi_key = \"abc\"\n".into()).is_err());
    assert_eq!(
        validate_settings(with_key)[0].severity,
        IssueSeverity::Error
    );

    // Settings with the account-based provider switched on export without any key.
    let on = settings(|v| v["providers"]["copernicus"]["enabled"] = json!(true));
    let toml = export_settings_toml(on.clone()).unwrap();
    assert!(toml.contains("enabled = true"));
    for key in toml_keys(&toml) {
        for word in ["key", "token", "password", "secret"] {
            assert!(!key.contains(word), "`{key}` looks like a key");
        }
    }
    assert!(!on.contains("api_key"));
}

#[test]
fn toml_round_trips_and_rejects_unknown_keys_clearly() {
    let json = settings(|v| {
        v["units"]["speed"] = json!("mph");
        v["presentation"]["voice"] = json!("plain");
    });
    let toml = export_settings_toml(json.clone()).unwrap();
    let back = import_settings_toml(toml).unwrap();
    assert_eq!(back.json, migrate_settings(json).unwrap().json);

    let SettingsError::Invalid { message } =
        import_settings_toml("[units]\nfurlongs = 1\n".into()).unwrap_err();
    assert!(
        message.contains("unknown key `units.furlongs`"),
        "{message}"
    );
    assert!(import_settings_toml("not toml [".into()).is_err());
}

#[test]
fn parse_user_value_converts_in_the_core() {
    let v = parse_user_value("discharge".into(), "1200".into(), cfs()).unwrap();
    assert!((v - 33.98).abs() < 0.01, "{v}");
    // A unit typed with the number overrides the setting.
    let v = parse_user_value("discharge".into(), "1,200 cfs".into(), metric()).unwrap();
    assert!((v - 33.98).abs() < 0.01, "{v}");
    let v = parse_user_value("discharge".into(), "40".into(), metric()).unwrap();
    assert!((v - 40.0).abs() < 1e-6);
    let v = parse_user_value("gage_height".into(), "10 ft".into(), metric()).unwrap();
    assert!((v - 3.048).abs() < 1e-4);
    let v = parse_user_value("observed_wind".into(), "18".into(), cfs()).unwrap();
    assert!((v - 9.26).abs() < 0.01, "{v}");
    let v = parse_user_value("water_temperature".into(), "68".into(), cfs()).unwrap();
    assert!((v - 20.0).abs() < 1e-4);
    let v = parse_user_value("water_temperature".into(), "20 °C".into(), cfs()).unwrap();
    assert!((v - 20.0).abs() < 1e-6);
    let v = parse_user_value("pressure".into(), "29.92".into(), cfs()).unwrap();
    assert!((v - 1013.2).abs() < 0.2, "{v}");
    let v = parse_user_value("distance".into(), "25".into(), cfs()).unwrap();
    assert!((v - 40_233.6).abs() < 0.5, "{v}");
}

#[test]
fn parse_user_value_refuses_what_is_not_a_number() {
    assert!(parse_user_value("discharge".into(), "lots".into(), cfs()).is_err());
    assert!(parse_user_value("discharge".into(), String::new(), cfs()).is_err());
    assert!(parse_user_value("discharge".into(), "inf".into(), cfs()).is_err());
    assert!(parse_user_value("mood".into(), "1".into(), cfs()).is_err());
}

#[test]
fn display_stops_are_round_in_the_users_unit() {
    let stops = display_stops("discharge".into(), 0.0, 56.633_693, cfs()).unwrap();
    let labels: Vec<_> = stops.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        ["0.0 cfs", "500 cfs", "1,000 cfs", "1,500 cfs", "2,000 cfs"]
    );
    assert!((stops[2].value - 28.316_85).abs() < 1e-3);
    // Each stop parses back to itself.
    for s in &stops {
        let back = parse_user_value("discharge".into(), s.label.clone(), cfs()).unwrap();
        assert!((back - s.value).abs() < 1e-3, "{} {back}", s.label);
    }
    let metric_stops = display_stops("discharge".into(), 0.0, 100.0, metric()).unwrap();
    assert_eq!(
        metric_stops.last().map(|s| s.label.as_str()),
        Some("100 m³/s")
    );
    assert!(display_stops("discharge".into(), 5.0, 5.0, cfs()).is_err());
}

#[test]
fn unit_options_list_groups_with_the_core_symbols() {
    let groups = unit_options(settings(|v| v["units"]["wind"] = json!("mph"))).unwrap();
    let ids: Vec<_> = groups.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "discharge",
            "height",
            "stage",
            "lake_level",
            "tide",
            "waves",
            "speed",
            "current",
            "wind",
            "water_temp",
            "air_temp",
            "pressure",
            "distance"
        ]
    );
    let wind = groups.iter().find(|g| g.id == "wind").unwrap();
    assert_eq!((wind.selected.as_str(), wind.overridden), ("mph", true));
    assert_eq!(wind.follows.as_deref(), Some("speed"));
    let labels: Vec<_> = wind.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["kn", "mph", "km/h", "m/s"]);
    let current = groups.iter().find(|g| g.id == "current").unwrap();
    assert_eq!(
        (current.selected.as_str(), current.overridden),
        ("kn", false)
    );
    let pressure = groups.iter().find(|g| g.id == "pressure").unwrap();
    assert_eq!(pressure.selected, "inHg");
    assert!(pressure.options.iter().any(|o| o.label == "kPa"));
}

#[test]
fn the_card_catalogue_follows_the_users_order_and_voice() {
    let cards = card_catalogue("river_stream".into(), cfs()).unwrap();
    assert_eq!(cards.len(), 19);
    assert_eq!(cards[0].id, "now");
    assert!(cards.iter().take_while(|c| c.shown).count() >= 5);
    let windows = cards.iter().find(|c| c.id == "windows").unwrap();
    assert_eq!(windows.title, "Snack times");
    assert!(!windows.available);
    let plain = settings(|v| v["presentation"]["voice"] = json!("plain"));
    let cards = card_catalogue("river_stream".into(), plain).unwrap();
    assert_eq!(
        cards.iter().find(|c| c.id == "windows").unwrap().title,
        "Fishing windows"
    );
    assert!(card_catalogue("swamp".into(), cfs()).is_err());
}

#[test]
fn the_source_catalogue_credits_every_source() {
    let sources = source_catalogue();
    let coops = sources.iter().find(|s| s.id == "coops").unwrap();
    assert!(coops.implemented && !coops.needs_key);
    assert!(coops.credit.contains("CO-OPS"));
    assert!(coops.measurements.iter().any(|m| m == "Tide"));
    let copernicus = sources.iter().find(|s| s.id == "copernicus").unwrap();
    assert!(copernicus.needs_key && copernicus.opt_in && !copernicus.implemented);
    let usgs = sources.iter().find(|s| s.id == "usgs").unwrap();
    assert!(usgs.key_optional && !usgs.needs_key);
    assert!(
        sources
            .iter()
            .all(|s| !s.name.is_empty() && !s.credit.is_empty())
    );
}

fn resource(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../ios/UnderbiteKit/Tests/UnderbiteKitTests/Resources")
        .join(name)
}

/// The file holds the settings the app starts with. `UNDERBITE_WRITE_SAMPLES=1 cargo test`
/// rewrites it.
#[test]
fn sample_settings_are_current() {
    let json = serde_json::to_string_pretty(
        &serde_json::from_str::<Value>(&default_settings_json()).unwrap(),
    )
    .unwrap()
        + "\n";
    let path = resource("sample-settings.json");
    if std::env::var_os("UNDERBITE_WRITE_SAMPLES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &json).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        kept, json,
        "run `UNDERBITE_WRITE_SAMPLES=1 cargo test -p underbite-ffi`"
    );
}
