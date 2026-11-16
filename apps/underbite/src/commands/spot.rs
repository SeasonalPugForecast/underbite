//! `underbite spot`: conditions for a spot.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use serde_json::Value;
use underbite_config::export::parse_toml;
use underbite_config::schema::{Config, Severity, has_errors};
use underbite_config::view_input::view_input;
use underbite_core::discover::hydro::WaterTypeSuggestion;
use underbite_core::model::{Spot, WaterType};
use underbite_core::view::strings::Strings;
use underbite_core::view::{Card, CardState, Quantity, SpotView, build_view};
use underbite_fetch::{FetchOptions, fetch_spot};

use crate::cli::Global;
use crate::runtime::env::{Env, var};

/// Arguments of `underbite spot`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Latitude and longitude in decimal degrees, as `lat,lon`.
    #[arg(value_name = "LAT,LON", allow_hyphen_values = true)]
    pub(crate) location: String,
    /// A name for the spot.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// The kind of water, such as `river_stream` or `tidal_river_estuary` (default: guessed
    /// from what the sources report).
    #[arg(long, value_name = "TYPE")]
    pub(crate) water_type: Option<String>,
    /// The spot's IANA time zone, such as `America/New_York`.
    #[arg(long, value_name = "ZONE")]
    pub(crate) tz: Option<String>,
    /// Read this configuration file instead of `$XDG_CONFIG_HOME/underbite/config.toml`.
    #[arg(long, value_name = "FILE")]
    pub(crate) config: Option<PathBuf>,
    /// Print the view as JSON.
    #[arg(long, conflicts_with = "snapshot")]
    pub(crate) json: bool,
    /// Print the raw snapshot as JSON.
    #[arg(long)]
    pub(crate) snapshot: bool,
}

/// Runs `underbite spot`.
pub(crate) fn run(args: &Args, global: &Global) -> anyhow::Result<ExitCode> {
    let config = load_config(args.config.as_deref(), &var)?;
    let (latitude, longitude) = parse_location(&args.location)?;
    let water_type = args
        .water_type
        .as_deref()
        .map(parse_water_type)
        .transpose()?;
    let env = Env::from_global(global)?;
    let now = env.now;
    let spot = Spot {
        id: point_id(latitude, longitude),
        name: args.name.clone().unwrap_or_else(|| "Spot".to_owned()),
        latitude,
        longitude,
        // Without a type given, the fetch infers one; a river when nothing points anywhere.
        water_type: water_type.unwrap_or(WaterType::RiverStream),
        water_body: None,
        time_zone: args.tz.clone().unwrap_or_default(),
    };
    let options = FetchOptions {
        infer_water_type: water_type.is_none(),
        ..env.options.clone()
    };
    let fetched = pollster::block_on(fetch_spot(&env.host(), &spot, &config, now, &options));
    if water_type.is_none() {
        eprintln!(
            "underbite: {}",
            inferred_text(
                fetched.inferred.as_ref(),
                spot.water_type,
                Strings::new(config.presentation.voice)
            )
        );
    }
    let snapshot = fetched.snapshot;

    for e in &snapshot.errors {
        eprintln!("underbite: {}: {}", e.source.as_str(), e.message);
    }
    if args.snapshot {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(ExitCode::SUCCESS);
    }
    let view = build_view(&view_input(&config, snapshot, now));
    if args.json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        print!("{}", render(&view)?);
    }
    Ok(ExitCode::SUCCESS)
}

/// The configuration at `path`, or the user's, or the defaults when there is none. Warnings
/// go to stderr; errors stop the command.
fn load_config(
    path: Option<&Path>,
    env: &impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Config> {
    let (path, explicit) = match path {
        Some(p) => (Some(p.to_path_buf()), true),
        None => (default_config_path(env)?, false),
    };
    let Some(path) = path else {
        return Ok(Config::default());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if !explicit && e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Config::default());
        }
        Err(e) => bail!("cannot read {}: {e}", path.display()),
    };
    let (config, issues) = parse_toml(&text).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    for issue in &issues {
        eprintln!("underbite: {}: {issue}", path.display());
    }
    if has_errors(&issues) {
        let first = issues.iter().find(|i| i.severity == Severity::Error);
        bail!(
            "{} has errors{}",
            path.display(),
            first.map_or_else(String::new, |i| format!(": {i}"))
        );
    }
    Ok(config)
}

/// `$XDG_CONFIG_HOME/underbite/config.toml`, else `$HOME/.config/underbite/config.toml`; none
/// when neither is set. An empty variable counts as unset. A relative path in the variable
/// used is an error, as for the cache, so the file is never looked for somewhere the
/// environment did not name.
fn default_config_path(env: &impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<PathBuf>> {
    let set = |name: &'static str| {
        env(name)
            .filter(|v| !v.is_empty())
            .map(|v| (name, PathBuf::from(v)))
    };
    let Some((name, path)) = set("XDG_CONFIG_HOME").or_else(|| set("HOME")) else {
        return Ok(None);
    };
    if !path.is_absolute() {
        bail!(
            "{name} is a relative path ({}); it must be absolute",
            path.display()
        );
    }
    let base = if name == "HOME" {
        path.join(".config")
    } else {
        path
    };
    Ok(Some(base.join("underbite").join("config.toml")))
}

/// The water type a fetch inferred and why, or the one used when nothing pointed anywhere.
fn inferred_text(
    inferred: Option<&WaterTypeSuggestion>,
    fallback: WaterType,
    strings: Strings,
) -> String {
    match inferred {
        Some(s) => {
            let reasons: Vec<String> = s
                .reasons
                .iter()
                .map(|r| strings.water_type_reason(r))
                .collect();
            format!(
                "water type {} (inferred: {}; --water-type sets another)",
                water_type_name(s.water_type),
                reasons.join("; ")
            )
        }
        None => format!(
            "water type {} (nothing pointed to one; --water-type sets another)",
            water_type_name(fallback)
        ),
    }
}

/// A water type as `--water-type` spells it.
fn water_type_name(water: WaterType) -> String {
    serde_json::to_value(water)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The id of an unnamed point, under which its pins, exclusions and last snapshot are kept:
/// the point to two decimals, what leaves the device, so every point in one rounding cell is
/// one spot. `spot` and `stations` share it.
pub(crate) fn point_id(latitude: f64, longitude: f64) -> String {
    // Rounded first, so a value just below zero does not print as `-0.00`.
    let cell = |v: f64| {
        let r = (v * 100.0).round() / 100.0;
        if r == 0.0 { 0.0 } else { r }
    };
    format!("cli-{:.2}_{:.2}", cell(latitude), cell(longitude))
}

/// Parses `lat,lon`.
fn parse_location(text: &str) -> anyhow::Result<(f64, f64)> {
    let (lat, lon) = text
        .split_once(',')
        .ok_or_else(|| anyhow!("expected LAT,LON, such as 40.7033,-74.0170"))?;
    let lat: f64 = lat.trim().parse().context("latitude")?;
    let lon: f64 = lon.trim().parse().context("longitude")?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        bail!("latitude is -90 to 90 and longitude -180 to 180");
    }
    Ok((lat, lon))
}

fn parse_water_type(text: &str) -> anyhow::Result<WaterType> {
    serde_json::from_value(Value::String(text.replace('-', "_")))
        .map_err(|_| anyhow!("unknown water type `{text}`"))
}

// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

/// The view as plain text: every string is the view's own.
fn render(view: &SpotView) -> anyhow::Result<String> {
    let mut out = String::new();
    let h = &view.header;
    writeln!(out, "{} ({}, {})", h.name, h.water_type_label, h.updated)?;
    for n in &view.notices {
        writeln!(out, "! {}", n.text)?;
    }
    for card in &view.cards {
        out.push('\n');
        render_card(&mut out, card)?;
    }
    Ok(out)
}

fn render_card(out: &mut String, card: &Card) -> anyhow::Result<()> {
    let chrome = card.chrome();
    let value = serde_json::to_value(card)?;
    writeln!(out, "== {} ==", chrome.title)?;
    match &chrome.state {
        CardState::Fresh => {}
        CardState::Stale { reason }
        | CardState::Far { reason }
        | CardState::Unavailable { reason } => writeln!(out, "[{reason}]")?,
    }
    if let Some(compact) = value.get("compact") {
        let headline = compact.get("headline").map(quantity).unwrap_or_default();
        if !headline.is_empty() {
            writeln!(out, "{headline}")?;
        }
        for line in strings(compact.get("lines")) {
            writeln!(out, "{line}")?;
        }
    }
    if let Some(Value::Object(full)) = value.get("full") {
        for (key, item) in full {
            render_full_item(out, key, item)?;
        }
    }
    if let Some(footnote) = &chrome.footnote {
        writeln!(out, "{footnote}")?;
    }
    for n in &chrome.notices {
        writeln!(out, "{}", n.text)?;
    }
    for c in &chrome.credits {
        match &c.link {
            Some(link) => writeln!(out, "Credit: {} ({link})", c.text)?,
            None => writeln!(out, "Credit: {}", c.text)?,
        }
    }
    Ok(())
}

/// One field of a card's full form: labels, quantities, upcoming events, the sections of
/// rows and the sources lists. Charts and day tables are for the phone.
fn render_full_item(out: &mut String, key: &str, item: &Value) -> anyhow::Result<()> {
    match (key, item) {
        ("chart" | "charts" | "days" | "spark", _) => {}
        (k, Value::String(s)) if !s.is_empty() && k.ends_with("label") => {
            writeln!(out, "  {s}")?;
        }
        (_, Value::Object(o)) if o.contains_key("text") => {
            writeln!(out, "  {}: {}", key.replace('_', " "), quantity(item))?;
        }
        ("credits", Value::Array(credits)) => {
            for c in credits {
                let text = |k: &str| c.get(k).and_then(Value::as_str);
                match text("link") {
                    Some(link) => writeln!(
                        out,
                        "  Credit: {} ({link})",
                        text("text").unwrap_or_default()
                    )?,
                    None => writeln!(out, "  Credit: {}", text("text").unwrap_or_default())?,
                }
            }
        }
        ("stations", Value::Array(stations)) => {
            for s in stations {
                let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or_default();
                let distance = s.get("distance").and_then(Value::as_str);
                write!(
                    out,
                    "  {}: {} ({}",
                    text("measurement"),
                    text("station"),
                    text("source")
                )?;
                if let Some(d) = distance {
                    write!(out, ", {d}")?;
                }
                writeln!(out, ", {})", text("age"))?;
            }
        }
        ("errors", Value::Array(errors)) => {
            for e in errors {
                let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or_default();
                match e.get("measurement").and_then(Value::as_str) {
                    Some(m) => writeln!(
                        out,
                        "  Problem: {}, {m}: {}",
                        text("source"),
                        text("message")
                    )?,
                    None => writeln!(out, "  Problem: {}: {}", text("source"), text("message"))?,
                }
            }
        }
        ("alerts", Value::Array(alerts)) => {
            for a in alerts {
                render_alert(out, a)?;
            }
        }
        ("sections", Value::Array(sections)) => {
            for s in sections {
                if let Some(title) = s.get("title").and_then(Value::as_str) {
                    writeln!(out, "  {title}")?;
                }
                for row in s
                    .get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let text = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or_default();
                    write!(out, "    {}: {}", text("label"), text("value"))?;
                    match row.get("detail").and_then(Value::as_str) {
                        Some(detail) => writeln!(out, " ({detail})")?,
                        None => writeln!(out)?,
                    }
                }
            }
        }
        ("next", Value::Array(events)) => {
            for e in events {
                let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or_default();
                match e.get("height").filter(|h| h.is_object()).map(quantity) {
                    Some(h) => writeln!(out, "  {} {} ({h})", text("label"), text("time"))?,
                    None => writeln!(out, "  {} {}", text("label"), text("time"))?,
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// One alert of the alerts card: its headline (or event), severity and times, then the
/// description and instruction whole, as issued, one indented line per line of the source.
fn render_alert(out: &mut String, alert: &Value) -> anyhow::Result<()> {
    let text = |k: &str| {
        alert
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    writeln!(
        out,
        "  {}",
        text("headline").or(text("event")).unwrap_or_default()
    )?;
    let mut facts = Vec::new();
    for (label, key) in [
        ("Severity", "severity_label"),
        ("From", "onset"),
        ("Until", "ends"),
        ("Expires", "expires"),
    ] {
        if let Some(v) = text(key) {
            facts.push(format!("{label}: {v}"));
        }
    }
    if !facts.is_empty() {
        writeln!(out, "    {}", facts.join(" · "))?;
    }
    for key in ["description", "instruction"] {
        if let Some(body) = text(key) {
            writeln!(out)?;
            for line in body.lines() {
                if line.trim().is_empty() {
                    writeln!(out)?;
                } else {
                    writeln!(out, "    {line}")?;
                }
            }
        }
    }
    Ok(())
}

/// A quantity as the view's own `Quantity::full` writes it; an object that is only a text is
/// that text.
fn quantity(q: &Value) -> String {
    match serde_json::from_value::<Quantity>(q.clone()) {
        Ok(q) => q.full(),
        Err(_) => q
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}

fn strings(v: Option<&Value>) -> Vec<&str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use underbite_core::discover::hydro::{WaterTypeReason, WaterbodyKind};
    use underbite_core::model::ZoneKind;
    use underbite_core::view::Voice;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn alerts_print_their_description_and_instruction_whole() {
        let description = format!(
            "* WHAT...West winds 10 to 20 kt.\n\n* IMPACTS...{}",
            "Conditions will be hazardous to small craft. ".repeat(20)
        );
        let alert = serde_json::json!({
            "event": "Small Craft Advisory",
            "headline": null,
            "description": description,
            "instruction": "Remain in port.",
            "severity_label": "Minor",
            "onset": "Nov 8, 1:48 AM",
            "ends": "Nov 9, 6:00 PM",
            "expires": null,
        });
        let mut out = String::new();
        render_full_item(&mut out, "alerts", &serde_json::json!([alert])).unwrap();
        assert!(out.starts_with("  Small Craft Advisory\n"), "{out}");
        assert!(
            out.contains("    Severity: Minor · From: Nov 8, 1:48 AM · Until: Nov 9, 6:00 PM\n"),
            "{out}"
        );
        for line in description.lines().chain(["Remain in port."]) {
            assert!(out.contains(line), "{line:?} missing from {out}");
        }
        assert!(!out.contains("Expires"), "{out}");
    }

    #[test]
    fn quantities_are_joined_as_the_view_writes_them() {
        let q = |text: &str, unit: &str, value: Option<f32>| {
            quantity(&serde_json::json!({"value": value, "text": text, "unit": unit}))
        };
        assert_eq!(q("1", "%", Some(1.0)), "1%");
        assert_eq!(q("1,200", "cfs", Some(1200.0)), "1,200 cfs");
        assert_eq!(q("--", "cfs", None), "--");
        assert_eq!(q("NE", "", Some(0.0)), "NE");
        assert_eq!(quantity(&serde_json::json!({"text": "Calm"})), "Calm");
    }

    #[test]
    fn error_rows_name_their_measurement() {
        let errors = serde_json::json!([
            {"source": "NOAA CO-OPS", "measurement": "Water temperature", "kind": "No data",
             "message": "no recent data"},
            {"source": "NWS", "measurement": null, "kind": "Failed", "message": "timed out"},
        ]);
        let mut out = String::new();
        render_full_item(&mut out, "errors", &errors).unwrap();
        assert_eq!(
            out,
            "  Problem: NOAA CO-OPS, Water temperature: no recent data\n  Problem: NWS: timed out\n"
        );
    }

    #[test]
    fn the_window_is_not_a_flag() {
        // The cards' requirements set how many days are fetched.
        assert!(Cli::try_parse_from(["spot", "40.70,-74.02"]).is_ok());
        assert!(Cli::try_parse_from(["spot", "40.70,-74.02", "--days", "3"]).is_err());
    }

    #[test]
    fn locations_allow_negative_values() {
        assert_eq!(
            parse_location("40.7033,-74.0170").unwrap(),
            (40.7033, -74.017)
        );
        assert_eq!(parse_location(" -33.9 , 151.2 ").unwrap(), (-33.9, 151.2));
        assert!(parse_location("40.7").is_err());
        assert!(parse_location("91,0").is_err());
        assert!(parse_location("a,b").is_err());
    }

    #[test]
    fn a_point_id_is_its_rounding_cell() {
        assert_eq!(point_id(40.7033, -74.0170), "cli-40.70_-74.02");
        assert_eq!(point_id(40.7049, -74.0151), point_id(40.7033, -74.0170));
        assert_ne!(point_id(40.7051, -74.0170), point_id(40.7033, -74.0170));
        assert_eq!(point_id(-0.001, 0.004), point_id(0.001, -0.004));
        assert_eq!(point_id(-0.001, 0.004), "cli-0.00_0.00");
    }

    #[test]
    fn the_inferred_water_type_says_why() {
        let s = WaterTypeSuggestion {
            water_type: WaterType::TidalRiverEstuary,
            reasons: vec![
                WaterTypeReason::MarineZone {
                    kind: ZoneKind::Coastal,
                },
                WaterTypeReason::Waterbody {
                    name: None,
                    kind: WaterbodyKind::River,
                    area_km2: 175.6,
                },
            ],
        };
        assert_eq!(
            inferred_text(Some(&s), WaterType::RiverStream, Strings::new(Voice::Plain)),
            "water type tidal_river_estuary (inferred: in an NWS coastal marine zone; on an \
             unnamed river; --water-type sets another)"
        );
        let stream = WaterTypeReason::Stream {
            name: Some("Croton River".into()),
            stream_order: Some(7),
        };
        assert_eq!(
            Strings::new(Voice::Plain).water_type_reason(&stream),
            "on Croton River (stream of order 7)"
        );
        assert!(
            inferred_text(None, WaterType::RiverStream, Strings::new(Voice::Plain))
                .starts_with("water type river_stream")
        );
    }

    #[test]
    fn water_types_take_either_separator() {
        assert_eq!(
            parse_water_type("river-stream").unwrap(),
            WaterType::RiverStream
        );
        assert_eq!(
            parse_water_type("tidal_river_estuary").unwrap(),
            WaterType::TidalRiverEstuary
        );
        assert!(parse_water_type("moat").is_err());
    }

    #[test]
    fn config_path_follows_the_environment() {
        let env = |name: &str| match name {
            "XDG_CONFIG_HOME" => Some("/x/cfg".to_owned()),
            "HOME" => Some("/h".to_owned()),
            _ => None,
        };
        assert_eq!(
            default_config_path(&env).unwrap(),
            Some(PathBuf::from("/x/cfg/underbite/config.toml"))
        );
        let home_only = |name: &str| (name == "HOME").then(|| "/h".to_owned());
        assert_eq!(
            default_config_path(&home_only).unwrap(),
            Some(PathBuf::from("/h/.config/underbite/config.toml"))
        );
        let empty = |name: &str| match name {
            "XDG_CONFIG_HOME" => Some(String::new()),
            "HOME" => Some("/h".to_owned()),
            _ => None,
        };
        assert_eq!(
            default_config_path(&empty).unwrap(),
            Some(PathBuf::from("/h/.config/underbite/config.toml"))
        );
        assert_eq!(default_config_path(&|_: &str| None).unwrap(), None);
    }

    #[test]
    fn a_relative_config_home_is_an_error() {
        let relative = |name: &str| match name {
            "XDG_CONFIG_HOME" => Some("cfg".to_owned()),
            "HOME" => Some("/h".to_owned()),
            _ => None,
        };
        let err = default_config_path(&relative).unwrap_err();
        assert!(err.to_string().contains("XDG_CONFIG_HOME"), "{err}");
        assert!(load_config(None, &relative).is_err());
        let home = |name: &str| (name == "HOME").then(|| "h".to_owned());
        assert!(default_config_path(&home).is_err());
    }

    #[test]
    fn a_missing_default_config_means_defaults_but_a_missing_explicit_one_fails() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().display().to_string();
        let env = move |name: &str| (name == "XDG_CONFIG_HOME").then(|| root.clone());
        assert!(load_config(None, &env).is_ok());
        assert!(load_config(Some(&dir.path().join("nope.toml")), &env).is_err());
    }

    #[test]
    fn config_errors_stop_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.toml");
        std::fs::write(&file, "[thresholds]\ntrout_stress_c = 99\n").unwrap();
        let err = load_config(Some(&file), &|_: &str| None).unwrap_err();
        assert!(err.to_string().contains("trout_stress_c"), "{err}");
    }
}
