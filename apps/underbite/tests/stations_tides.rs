//! The `stations`, `tides` and `record` commands against the recorded New York Harbor
//! responses. Nothing here touches the network: every run answers from the replay directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const NOW: &str = "2026-11-08T14:30:00Z";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn battery() -> PathBuf {
    root().join("tests/fixtures/replay/battery")
}

/// Runs the tool with an isolated cache and no keys.
fn run(cache: &Path, replay: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_underbite"))
        .args(args)
        .arg("--replay")
        .arg(replay)
        .args(["--now", NOW])
        .env_clear()
        .env("XDG_CACHE_HOME", cache)
        .output()
        .expect("run underbite")
}

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).expect("utf-8")
}

fn json(out: &Output) -> Value {
    serde_json::from_str(&stdout(out)).expect("json")
}

fn events(v: &Value) -> Vec<(i64, f64)> {
    v["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| {
            (
                e["t"].as_i64().expect("t"),
                e["height_m"].as_f64().expect("h"),
            )
        })
        .collect()
}

/// The recorded high/low file: (unix seconds, metres).
fn recorded() -> Vec<(i64, f64)> {
    let text = std::fs::read_to_string(
        root().join("tests/fixtures/raw/coops/coops-8518750-hilo-2026-11-08.json"),
    )
    .expect("fixture");
    let body: Value = serde_json::from_str(&text).expect("json");
    body["predictions"]
        .as_array()
        .expect("predictions")
        .iter()
        .map(|p| {
            let t = p["t"].as_str().expect("t");
            let ts: jiff::Timestamp = format!("{}:00Z", t.replace(' ', "T"))
                .parse()
                .expect("time");
            (
                ts.as_second(),
                p["v"].as_str().expect("v").parse().expect("v"),
            )
        })
        .collect()
}

#[test]
fn stations_near_the_battery_rank_the_battery_first_with_reasons() {
    let cache = tempfile::tempdir().expect("tempdir");
    let out = run(
        cache.path(),
        &battery(),
        &[
            "stations",
            "near",
            "40.7033,-74.0170",
            "--pin",
            "coops:NYH1927:13",
            "--json",
        ],
    );
    let v = json(&out);
    let tide = &v["measurements"]["tide"]["stations"][0];
    assert_eq!(tide["station"]["id"], "8518750");
    assert!(tide["reasons"].get(0).is_some());
    let current = &v["measurements"]["tidal_current"]["stations"][0];
    assert_eq!(current["station"]["id"], "NYH1927");
    assert_eq!(current["station"]["bin"], 13);
    assert_eq!(current["pinned"], true);
    assert!(current["reasons"].get(0).is_some());
}

#[test]
fn stations_text_names_distance_relation_and_reasons() {
    let cache = tempfile::tempdir().expect("tempdir");
    let out = run(
        cache.path(),
        &battery(),
        &[
            "stations",
            "near",
            "40.7033,-74.0170",
            "--measurement",
            "tide",
        ],
    );
    let text = stdout(&out);
    let first = text
        .lines()
        .find(|l| l.contains("1. "))
        .expect("first line");
    assert!(
        first.contains("8518750") && first.contains(" mi"),
        "{first}"
    );
    assert!(text.contains("- "), "{text}");
    assert!(!text.contains("tidal_current"));
}

#[test]
fn a_pin_is_kept_for_the_next_run_and_can_be_cleared() {
    let cache = tempfile::tempdir().expect("tempdir");
    let first = |extra: &[&str]| {
        let mut args = vec!["stations", "near", "40.7033,-74.0170", "--json"];
        args.extend_from_slice(extra);
        let v = json(&run(cache.path(), &battery(), &args));
        v["measurements"]["tidal_current"]["stations"][0]["station"]["id"]
            .as_str()
            .expect("id")
            .to_owned()
    };
    assert_eq!(first(&[]), "NYH1927");
    assert_eq!(first(&["--pin", "coops:NYH1919:3"]), "NYH1919");
    assert_eq!(first(&[]), "NYH1919");
    assert_eq!(first(&["--clear", "coops:NYH1919:3"]), "NYH1927");
}

#[test]
fn bad_arguments_fail_clearly() {
    let cache = tempfile::tempdir().expect("tempdir");
    for args in [
        &["stations", "near", "abc"][..],
        &["stations", "near", "40.7,-74.0", "--pin", "coops"],
        &["stations", "near", "40.7,-74.0", "--measurement", "tides"],
        &["tides", "8518750", "--days", "0"],
        &["tides", "8518750", "--from", "yesterday"],
    ] {
        let out = run(cache.path(), &battery(), args);
        assert!(!out.status.success(), "{args:?}");
    }
}

#[test]
fn tides_for_a_week_give_the_recorded_events_in_local_time() {
    let cache = tempfile::tempdir().expect("tempdir");
    let args = ["tides", "8518750", "--from", "2026-11-08", "--days", "7"];
    let text = stdout(&run(cache.path(), &battery(), &args));
    assert!(text.contains("America/New_York"), "{text}");
    assert!(text.contains("ft MLLW"), "{text}");
    assert!(text.contains("EST"), "{text}");
    assert!(text.contains("Predictions published by NOAA"), "{text}");
    assert!(!text.contains("Computed on this device"), "{text}");

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let v = json(&run(cache.path(), &battery(), &json_args));
    let got = events(&v);
    assert_eq!(v["datum"], "ft MLLW");
    // The days are the station's: 2026-11-08 to 2026-11-14 in New York, which begin at 05:00
    // UTC on the 8th and end at 04:59 UTC on the 15th; the recorded week lies inside.
    let (start, end) = (NOV_8_START, NOV_8_START + 7 * 86_400);
    let want: Vec<_> = recorded()
        .into_iter()
        .filter(|(t, _)| (start..end).contains(t))
        .collect();
    assert_eq!(want.len(), 27);
    assert_eq!(got.len(), want.len());
    for ((t, h), (rt, rh)) in got.iter().zip(&want) {
        assert_eq!(t, rt);
        assert!((h - rh).abs() < 1e-4, "{h} vs {rh}");
    }
    let new_york = jiff::tz::TimeZone::get("America/New_York").expect("zone");
    for e in v["events"].as_array().expect("events") {
        let at = jiff::Timestamp::from_second(e["t"].as_i64().expect("t")).expect("ts");
        let date = at.to_zoned(new_york.clone()).date().to_string();
        assert!(
            ("2026-11-08".."2026-11-15").contains(&date.as_str()),
            "{date}"
        );
    }
}

/// 2026-11-08 00:00 in New York (EST, UTC-5), as a Unix time.
const NOV_8_START: i64 = 1_794_114_000;

#[test]
fn a_day_is_a_day_of_the_station_not_of_utc() {
    // The evening of the 14th in New York is the 15th in UTC: it belongs to `--from
    // 2026-11-14`, and the evening of the 13th does not.
    let cache = tempfile::tempdir().expect("tempdir");
    let args = [
        "tides",
        "8518750",
        "--offline",
        "--from",
        "2026-11-14",
        "--days",
        "1",
        "--json",
    ];
    let v = json(&run(cache.path(), &battery(), &args));
    let new_york = jiff::tz::TimeZone::get("America/New_York").expect("zone");
    let local: Vec<String> = events(&v)
        .iter()
        .map(|(t, _)| {
            jiff::Timestamp::from_second(*t)
                .expect("ts")
                .to_zoned(new_york.clone())
                .strftime("%Y-%m-%d %H")
                .to_string()
        })
        .collect();
    assert!(!local.is_empty(), "{local:?}");
    assert!(
        local.iter().all(|l| l.starts_with("2026-11-14")),
        "{local:?}"
    );
    let last: i32 = local.last().expect("last")[11..].parse().expect("hour");
    assert!(last >= 16, "the evening is included: {local:?}");
    let text = stdout(&run(
        cache.path(),
        &battery(),
        &[
            "tides",
            "8518750",
            "--offline",
            "--from",
            "2026-11-14",
            "--days",
            "1",
        ],
    ));
    assert!(!text.contains("Nov 13"), "{text}");
    assert!(text.contains("Nov 14"), "{text}");
}

#[test]
fn offline_tides_match_the_published_ones_and_say_where_they_come_from() {
    let cache = tempfile::tempdir().expect("tempdir");
    let args = [
        "tides",
        "8518750",
        "--offline",
        "--from",
        "2026-11-08",
        "--days",
        "7",
    ];
    let text = stdout(&run(cache.path(), &battery(), &args));
    assert!(text.contains("Computed on this device"), "{text}");
    assert!(text.contains("ft MLLW"), "{text}");

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let v = json(&run(cache.path(), &battery(), &json_args));
    assert_eq!(v["computed_on_device"], true);
    let got = events(&v);
    let want = recorded();
    assert_eq!(got.len(), want.len());
    for ((t, h), (rt, rh)) in got.iter().zip(&want) {
        assert!((t - rt).abs() <= 120, "{t} vs {rt}");
        assert!((h - rh).abs() <= 0.04, "{h} vs {rh}");
    }
}

#[test]
fn offline_tides_run_again_from_the_cache_alone() {
    let cache = tempfile::tempdir().expect("tempdir");
    let args = [
        "tides",
        "8518750",
        "--offline",
        "--from",
        "2026-11-08",
        "--days",
        "7",
        "--json",
    ];
    let seeded = json(&run(cache.path(), &battery(), &args));

    // A replay with no routes answers nothing: the second run can only use the cache.
    let empty = tempfile::tempdir().expect("tempdir");
    std::fs::write(empty.path().join("routes.toml"), "").expect("routes");
    let again = json(&run(cache.path(), empty.path(), &args));
    assert_eq!(events(&again), events(&seeded));

    let fresh = tempfile::tempdir().expect("tempdir");
    let out = run(fresh.path(), empty.path(), &args);
    assert!(!out.status.success());
}

#[test]
fn record_writes_the_body_and_prints_a_redacted_index_line() {
    let cache = tempfile::tempdir().expect("tempdir");
    let out_dir = tempfile::tempdir().expect("tempdir");
    let args = [
        "record",
        "coops",
        "predictions",
        "--station",
        "8518750",
        "--begin",
        "2026-11-08",
        "--end",
        "2026-11-14",
        "--out",
        out_dir.path().to_str().expect("path"),
    ];
    let line = stdout(&run(cache.path(), &battery(), &args));
    assert!(
        line.starts_with("- `coops-8518750-hilo-2026-11-08.json`: `https://api.tidesandcurrents"),
        "{line}"
    );
    assert!(
        line.contains("begin_date=20261108&end_date=20261114"),
        "{line}"
    );
    assert!(line.trim_end().ends_with("(2026-11-08)"), "{line}");
    let written = out_dir
        .path()
        .join("coops/coops-8518750-hilo-2026-11-08.json");
    let want =
        std::fs::read(root().join("tests/fixtures/raw/coops/coops-8518750-hilo-2026-11-08.json"))
            .expect("fixture");
    assert_eq!(std::fs::read(&written).expect("written"), want);

    // A second recording does not replace it without --force.
    let again = run(cache.path(), &battery(), &args);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("--force"));
    let mut forced = args.to_vec();
    forced.push("--force");
    assert!(run(cache.path(), &battery(), &forced).status.success());
}

#[test]
fn record_refuses_a_request_that_carries_a_key() {
    let out_dir = tempfile::tempdir().expect("tempdir");
    let cache = tempfile::tempdir().expect("tempdir");
    let out = Command::new(env!("CARGO_BIN_EXE_underbite"))
        .args(["record", "usgs", "latest-continuous", "--site", "01375000"])
        .args(["--out", out_dir.path().to_str().expect("path")])
        .arg("--replay")
        .arg(battery())
        .env_clear()
        .env("XDG_CACHE_HOME", cache.path())
        .env("USGS_API_KEY", "not-a-real-key")
        .output()
        .expect("run");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("carries a key"), "{err}");
    assert!(!err.contains("not-a-real-key"));
    assert!(!out_dir.path().join("usgs").exists());
}

#[test]
fn the_battery_stations_in_words() {
    let cache = tempfile::tempdir().expect("tempdir");
    let text = stdout(&run(
        cache.path(),
        &battery(),
        &["stations", "near", "40.7033,-74.0170"],
    ));
    // Distance and relation are on the station's line, not repeated as reasons.
    assert!(!text.contains("listed, not confirmed"), "{text}");
    assert!(!text.contains("     - Same water body"), "{text}");
    assert!(!text.contains("     - 0.2 mi"), "{text}");
    insta::assert_snapshot!("battery_stations_text", text);
}

#[test]
fn record_without_a_resource_lists_them() {
    let cache = tempfile::tempdir().expect("tempdir");
    let text = stdout(&run(cache.path(), &battery(), &["record", "coops"]));
    assert!(text.contains("predictions"), "{text}");
}
