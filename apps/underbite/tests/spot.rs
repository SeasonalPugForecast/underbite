//! `underbite spot` against recorded responses at a fixed time.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const NOW: &str = "2026-11-08T14:30:00Z";
const BATTERY: &str = "40.7033,-74.0170";
const CROTON: &str = "41.2250,-73.8589";

fn replay_dir(scenario: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/replay")
        .join(scenario)
}

fn run(scenario: &str, args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_underbite"))
        .env_clear()
        .env("XDG_CONFIG_HOME", home.path())
        .arg("--replay")
        .arg(replay_dir(scenario))
        .args(["--now", NOW, "spot"])
        .args(args)
        .output()
        .unwrap()
}

fn stdout(scenario: &str, args: &[&str]) -> String {
    let out = run(scenario, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn battery_text() {
    let text = stdout("battery", &[BATTERY, "--name", "The Battery"]);
    assert!(text.contains("1:34 PM"), "{text}");
    assert!(text.to_lowercase().contains("not for navigation"), "{text}");
    assert!(text.contains("Credit:"), "{text}");
    insta::assert_snapshot!("battery_text", text);
}

#[test]
fn battery_json() {
    let json = stdout("battery", &[BATTERY, "--name", "The Battery", "--json"]);
    let view: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(view["cards"].as_array().is_some_and(|c| !c.is_empty()));
    insta::assert_snapshot!("battery_json", json);
}

#[test]
fn croton_text() {
    let text = stdout("croton", &[CROTON, "--name", "Croton River"]);
    assert!(text.contains("12.4"), "{text}");
    insta::assert_snapshot!("croton_text", text);
}

#[test]
fn croton_json() {
    let json = stdout("croton", &[CROTON, "--name", "Croton River", "--json"]);
    insta::assert_snapshot!("croton_json", json);
}

#[test]
fn snapshot_flag_prints_the_raw_snapshot() {
    let json = stdout("croton", &[CROTON, "--snapshot"]);
    let snap: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(snap["water"]["discharge_m3s"].is_object());
    assert!(snap.get("cards").is_none());
}

/// Runs the tool on the Battery's recordings with its cache in `cache`.
fn cached(cache: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_underbite"))
        .env_clear()
        .env("XDG_CACHE_HOME", cache)
        .arg("--replay")
        .arg(replay_dir("battery"))
        .args(["--now", NOW])
        .args(args)
        .output()
        .unwrap()
}

/// The station of the observed wind in a `spot --snapshot` run.
fn wind_station(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let snap: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    snap["marine"]["observed_wind"]["provenance"]["station"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_pin_made_with_stations_applies_to_spot_at_the_same_point() {
    let cache = tempfile::tempdir().unwrap();
    let before = wind_station(&cached(cache.path(), &["spot", BATTERY, "--snapshot"]));
    assert_ne!(before, "kptn6");
    // Another point in the same two-decimal cell.
    let pinned = cached(
        cache.path(),
        &[
            "stations",
            "near",
            "40.7041,-74.0162",
            "--pin",
            "ndbc:kptn6",
        ],
    );
    assert!(
        pinned.status.success(),
        "{}",
        String::from_utf8_lossy(&pinned.stderr)
    );
    let after = wind_station(&cached(cache.path(), &["spot", BATTERY, "--snapshot"]));
    assert_eq!(after, "kptn6");
}

#[test]
fn a_relative_cache_directory_is_refused() {
    let out = Command::new(env!("CARGO_BIN_EXE_underbite"))
        .env_clear()
        .env("XDG_CACHE_HOME", "relative/cache")
        .arg("--replay")
        .arg(replay_dir("battery"))
        .args(["--now", NOW, "spot", BATTERY])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("XDG_CACHE_HOME"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_bad_location_fails_with_a_message() {
    let out = run("battery", &["nowhere"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("LAT,LON"));
}

#[test]
fn a_missing_replay_directory_fails() {
    let out = Command::new(env!("CARGO_BIN_EXE_underbite"))
        .env_clear()
        .args(["--replay", "/nonexistent", "spot", BATTERY])
        .output()
        .unwrap();
    assert!(!out.status.success());
}
