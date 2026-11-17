//! The settings calls: migration, validation, TOML export and import, the unit and card
//! catalogues, value entry and the sample settings resource.

use std::path::PathBuf;

use serde_json::{Value, json};
use underbite_ffi::settings::{
    IssueSeverity, SettingsError, card_catalogue, default_settings_json, display_stops,
    export_settings_toml, format_duration, format_setting_value, import_settings_toml,
    limit_summary, measurement_titles, migrate_settings, parse_setting_value, parse_user_value,
    percentile_band_label, setting_field_keys, setting_stops, source_catalogue,
    source_group_titles, source_limits_text, unit_options, validate_settings, water_type_options,
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
    assert_eq!(cards.len(), 20);
    // No source fills the bloom card yet.
    assert!(!cards.iter().find(|c| c.id == "bloom").unwrap().available);
    assert_eq!(cards[0].id, "now");
    assert!(cards.iter().take_while(|c| c.shown).count() >= 5);
    let windows = cards.iter().find(|c| c.id == "windows").unwrap();
    assert_eq!(windows.title, "Snack times");
    assert!(windows.available);
    assert!(cards.iter().find(|c| c.id == "go_no_go").unwrap().available);
    let plain = settings(|v| v["presentation"]["voice"] = json!("plain"));
    let cards = card_catalogue("river_stream".into(), plain).unwrap();
    assert_eq!(
        cards.iter().find(|c| c.id == "windows").unwrap().title,
        "Fishing windows"
    );
    assert!(card_catalogue("swamp".into(), cfs()).is_err());
}

#[test]
fn water_types_come_with_the_cores_names_in_picker_order() {
    let options = water_type_options(cfs()).unwrap();
    let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "river_stream",
            "lake_reservoir",
            "tidal_river_estuary",
            "bay_inshore",
            "surf",
            "nearshore",
            "offshore",
            "great_lakes",
            "pond",
            "ice"
        ]
    );
    assert_eq!(options[2].label, "Tidal river or estuary");
    assert_eq!(options[9].label, "Ice fishing");
    // Both voices name the kinds of water alike, so no second label is sent.
    assert!(options.iter().all(|o| o.plain_label.is_none()));
    let plain = settings(|v| v["presentation"]["voice"] = json!("plain"));
    assert_eq!(water_type_options(plain).unwrap(), options);
    assert!(water_type_options("not json".into()).is_err());
}

#[test]
fn measurement_titles_cover_every_measurement_by_id() {
    let titles = measurement_titles(cfs()).unwrap();
    assert_eq!(titles.len(), 27);
    assert_eq!(titles[0].id, "tide");
    let title = |id: &str| titles.iter().find(|t| t.id == id).map(|t| t.title.as_str());
    assert_eq!(title("discharge"), Some("River flow"));
    assert_eq!(title("observed_wind"), Some("Wind"));
    assert_eq!(title("ph"), Some("pH"));
    assert!(measurement_titles("{".into()).is_err());
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

fn knots() -> String {
    settings(|v| v["units"]["speed"] = json!("kn"))
}

fn parse(field: &str, text: &str, json: String) -> f64 {
    parse_setting_value(field.into(), text.into(), json).unwrap()
}

fn format(field: &str, value: f64, json: String) -> String {
    format_setting_value(field.into(), value, json).unwrap()
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3 * b.abs().max(1.0), "{a} is not {b}");
}

#[test]
fn every_field_key_is_accepted() {
    let keys = setting_field_keys();
    assert!(keys.contains(&"limits[].max_wind_kn".to_owned()));
    assert!(keys.contains(&"sources.tide.max_age_minutes".to_owned()));
    assert!(keys.contains(&"sources.storm.max_distance_km".to_owned()));
    for key in keys {
        assert!(
            format_setting_value(key.clone(), 10.0, default_settings_json()).is_ok(),
            "{key}"
        );
        assert!(
            parse_setting_value(key.clone(), "10".into(), default_settings_json()).is_ok(),
            "{key}"
        );
    }
    // A list position is ignored; an unknown field is refused.
    assert!(format_setting_value("limits[3].max_wind_kn".into(), 1.0, knots()).is_ok());
    assert!(format_setting_value("limits[].name".into(), 1.0, knots()).is_err());
    assert!(format_setting_value("sources.nope.max_age_minutes".into(), 1.0, knots()).is_err());
}

#[test]
fn knots_fields_follow_the_speed_unit() {
    let mph = settings(|v| v["units"]["speed"] = json!("mph"));
    assert_eq!(format("limits[].max_wind_kn", 15.0, mph.clone()), "17 mph");
    close(parse("limits[].max_wind_kn", "17.26", mph.clone()), 15.0);
    close(parse("limits[].max_wind_kn", "20 kn", mph), 20.0);
    // In knots the value passes through untouched.
    assert_eq!(parse("limits[].max_gust_kn", "25", knots()), 25.0);
    assert_eq!(format("limits[].max_gust_kn", 25.0, knots()), "25 kn");
    assert_eq!(format("limits[].max_current_kn", 1.5, knots()), "1.5 kn");
    // Decimal comma.
    assert_eq!(parse("limits[].max_current_kn", "1,5", knots()), 1.5);
    // Round trip through the written text (one decimal in m/s, so within a few percent).
    let text = format("limits[].max_wind_kn", 18.0, metric());
    assert_eq!(text, "9.3 m/s");
    let back = parse("limits[].max_wind_kn", &text, metric());
    assert!((back - 18.0).abs() < 0.2, "{back}");
    // Typed in the unit shown, the stored value reads back exactly.
    assert_eq!(
        parse(
            "limits[].max_wind_kn",
            &format("limits[].max_wind_kn", 18.0, knots()),
            knots()
        ),
        18.0
    );
}

#[test]
fn cfs_fields_follow_the_discharge_unit() {
    assert_eq!(
        format("rivers[].wadeable_below_cfs", 1200.0, cfs()),
        "1,200 cfs"
    );
    assert_eq!(parse("rivers[].wadeable_below_cfs", "1,200", cfs()), 1200.0);
    assert_eq!(parse("rivers[].best_cfs", "1.200,5", cfs()), 1200.5);
    // Metric: 1,000 cfs is 28.3 m3/s.
    assert_eq!(
        format("rivers[].blown_out_above_cfs", 1000.0, metric()),
        "28 m³/s"
    );
    close(
        parse("rivers[].blown_out_above_cfs", "28,3", metric()),
        999.5,
    );
    close(
        parse("rivers[].blown_out_above_cfs", "1500 cfs", metric()),
        1500.0,
    );
    let stops = setting_stops("rivers[].wadeable_below_cfs".into(), 0.0, 2000.0, metric()).unwrap();
    assert_eq!(stops.first().unwrap().value, 0.0);
    assert!(stops.len() >= 3);
    // Stops carry cfs values and m3/s labels.
    let second = &stops[1];
    assert_eq!(
        second.label,
        format("rivers[].wadeable_below_cfs", second.value, metric())
    );
    assert!(second.label.ends_with("m³/s"));
    assert!(second.value > 100.0 && second.value < 1500.0);
    // In cfs the stops are round numbers.
    let cfs_stops = setting_stops("rivers[].best_cfs".into(), 0.0, 2000.0, cfs()).unwrap();
    let values: Vec<f64> = cfs_stops.iter().map(|s| s.value).collect();
    assert_eq!(values, [0.0, 500.0, 1000.0, 1500.0, 2000.0]);
    assert_eq!(cfs_stops[1].label, "500 cfs");
}

#[test]
fn km_fields_follow_the_distance_unit() {
    // Default distance unit is miles.
    assert_eq!(
        format("sources.tide.max_distance_km", 16.09344, cfs()),
        "10 mi"
    );
    close(parse("sources.tide.max_distance_km", "10", cfs()), 16.09344);
    close(
        parse("limits[].min_visibility_km", "5 mi", metric()),
        8.04672,
    );
    assert_eq!(
        parse("storms.hurricane_distance_km", "3000", metric()),
        3000.0
    );
    assert_eq!(
        format("storms.hurricane_distance_km", 3000.0, metric()),
        "3000 km"
    );
    assert_eq!(parse("limits[].min_visibility_km", "1,5", metric()), 1.5);
    let stops = setting_stops("sources.flow.max_distance_km".into(), 0.0, 100.0, metric()).unwrap();
    let values: Vec<f64> = stops.iter().map(|s| s.value).collect();
    assert_eq!(values, [0.0, 50.0, 100.0]);
    assert_eq!(stops[1].label, "50 km");
}

#[test]
fn temperature_fields_follow_the_temperature_unit() {
    // Stored in °C; the defaults read °F.
    assert_eq!(format("limits[].min_air_temp_c", 0.0, cfs()), "32 °F");
    close(parse("limits[].min_air_temp_c", "50", cfs()), 10.0);
    close(parse("thresholds.trout_stress_c", "68 °F", metric()), 20.0);
    assert_eq!(parse("thresholds.trout_stress_c", "20", metric()), 20.0);
    assert_eq!(
        format("thresholds.trout_stress_c", 20.0, metric()),
        "20.0 °C"
    );
    close(parse("thresholds.trout_stress_c", "20,5", metric()), 20.5);
    let stops = setting_stops("thresholds.trout_stress_c".into(), 10.0, 30.0, metric()).unwrap();
    let values: Vec<f64> = stops.iter().map(|s| s.value).collect();
    assert_eq!(values, [10.0, 15.0, 20.0, 25.0, 30.0]);
}

#[test]
fn wave_heights_follow_the_height_unit() {
    assert_eq!(format("limits[].max_wave_m", 1.0, cfs()), "3.3 ft");
    close(parse("limits[].max_wave_m", "3.3", cfs()), 1.006);
    assert_eq!(parse("limits[].max_wave_m", "1,2", metric()), 1.2);
}

#[test]
fn minute_fields_are_durations() {
    let f = |m| format("sources.tide.max_age_minutes", m, cfs());
    assert_eq!(f(45.0), "45 min");
    assert_eq!(f(90.0), "90 min");
    assert_eq!(f(360.0), "6 h");
    assert_eq!(f(2880.0), "2 d");
    assert_eq!(parse("sources.tide.max_age_minutes", "45", cfs()), 45.0);
    assert_eq!(parse("sources.tide.max_age_minutes", "2 h", cfs()), 120.0);
    assert_eq!(parse("sources.tide.max_age_minutes", "1,5 h", cfs()), 90.0);
    assert_eq!(parse("sources.tide.max_age_minutes", "1 d", cfs()), 1440.0);
    assert_eq!(parse("sources.tide.max_age_minutes", "30 min", cfs()), 30.0);
    assert!(
        parse_setting_value("sources.tide.max_age_minutes".into(), "soon".into(), cfs()).is_err()
    );
    for m in [15.0, 90.0, 720.0, 4320.0] {
        assert_eq!(parse("sources.tide.max_age_minutes", &f(m), cfs()), m);
    }
    let stops = setting_stops("sources.tide.max_age_minutes".into(), 0.0, 360.0, cfs()).unwrap();
    let values: Vec<f64> = stops.iter().map(|s| s.value).collect();
    assert_eq!(values, [0.0, 120.0, 240.0, 360.0]);
    assert_eq!(stops[1].label, "2 h");
    assert!(setting_stops("sources.tide.max_age_minutes".into(), 5.0, 5.0, cfs()).is_err());
}

#[test]
fn durations_come_from_the_core() {
    assert_eq!(format_duration(2700, cfs()).unwrap(), "45 min");
    assert_eq!(format_duration(21_600, cfs()).unwrap(), "6 h");
    assert_eq!(format_duration(172_800, cfs()).unwrap(), "2 d");
}

#[test]
fn the_old_value_entry_still_reads_thousands_commas() {
    let v = parse_user_value("discharge".into(), "1,200".into(), cfs()).unwrap();
    assert!((v - 33.98).abs() < 0.01, "{v}");
}

#[test]
fn a_suffix_whose_lowercase_has_another_byte_length_does_not_panic() {
    // KELVIN SIGN (3 bytes) lowercases to "k" (1 byte).
    assert_eq!(
        parse("limits[].max_wind_kn", "15 \u{212A}n", knots()),
        parse("limits[].max_wind_kn", "15 kn", knots())
    );
    assert_eq!(
        parse("sources.tide.max_distance_km", "10 \u{212A}m", cfs()),
        parse("sources.tide.max_distance_km", "10 km", cfs())
    );
}

#[test]
fn odd_unicode_around_a_value_never_panics() {
    let odd = [
        "\u{212A}",
        "\u{212A}n",
        "15\u{212A}",
        "15 \u{212A}\u{212A}n",
        "\u{130}",
        "15 \u{130}n",
        "15 m\u{b3}/s",
        "15 \u{2126}",
        "\u{1F41F}",
        "1\u{1F41F}5 kn",
        "15 k\u{301}n",
        "\u{1C5}",
        "15 \u{DF}",
        "\u{FB01}",
        "  \u{3000}15\u{3000}kn",
        "",
    ];
    let fields = [
        "limits[].max_wind_kn",
        "limits[].max_wave_m",
        "sources.tide.max_distance_km",
        "sources.tide.max_age_minutes",
    ];
    for json in [knots(), cfs(), metric()] {
        for field in fields {
            for text in odd {
                let _ = parse_setting_value(field.into(), text.into(), json.clone());
            }
        }
        for measurement in ["discharge", "water_level", "wind", "temperature"] {
            for text in odd {
                let _ = parse_user_value(measurement.into(), text.into(), json.clone());
            }
        }
    }
}

#[test]
fn unit_groups_that_follow_another_carry_their_own_choice_label() {
    let groups = unit_options(cfs()).unwrap();
    let label = |id: &str| {
        groups
            .iter()
            .find(|g| g.id == id)
            .unwrap()
            .same_as_label
            .clone()
    };
    assert_eq!(label("stage").as_deref(), Some("Same as heights"));
    assert_eq!(label("wind").as_deref(), Some("Same as speeds"));
    assert_eq!(
        label("air_temp").as_deref(),
        Some("Same as water temperature")
    );
    assert_eq!(label("discharge"), None);
}

#[test]
fn source_groups_are_named_in_both_voices() {
    for voice in ["plain", "luna"] {
        let json = settings(|v| v["presentation"]["voice"] = json!(voice));
        let titles = source_group_titles(json).unwrap();
        assert_eq!(titles.len(), 12);
        assert_eq!(titles[0].id, "water_temperature");
        assert_eq!(titles[0].title, "Water temperature");
        let sst = titles.iter().find(|t| t.id == "sst").unwrap();
        assert_eq!(sst.title, "Sea surface temperature");
    }
    assert!(source_group_titles("not json".into()).is_err());
}

#[test]
fn source_limits_read_in_the_users_units() {
    let text = |km, min, json: String| source_limits_text("tide".into(), km, min, json).unwrap();
    assert_eq!(text(None, None, cfs()), "any distance, any age");
    assert_eq!(
        text(Some(25.0), Some(180), metric()),
        "within 25 km, under 3 h old"
    );
    assert_eq!(
        text(Some(25.0), Some(45), cfs()),
        "within 16 mi, under 45 min old"
    );
    assert!(source_limits_text("nope".into(), None, None, cfs()).is_err());
}

#[test]
fn a_profile_summary_counts_limits_and_names_the_switches() {
    assert_eq!(
        limit_summary(0, false, false, cfs()).unwrap(),
        "No limits set"
    );
    assert_eq!(limit_summary(1, false, false, cfs()).unwrap(), "1 limit");
    assert_eq!(
        limit_summary(2, true, true, cfs()).unwrap(),
        "2 limits, no small craft advisory, no lightning"
    );
}

#[test]
fn the_shaded_band_has_its_own_label() {
    assert_eq!(
        percentile_band_label("5th percentile".into(), "95th percentile".into(), cfs()).unwrap(),
        "Shaded: 5th percentile to 95th percentile"
    );
}
