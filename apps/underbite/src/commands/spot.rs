//! `underbite spot`: conditions for a spot.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use serde_json::Value;
use underbite_config::export::parse_toml;
use underbite_config::rivers::RiverRange;
use underbite_config::schema::{Config, Severity, has_errors};
use underbite_core::discover::hydro::{WaterTypeReason, WaterTypeSuggestion, WaterbodyKind};
use underbite_core::model::{Spot, SpotSnapshot, WaterType, ZoneKind};
use underbite_core::provenance::UsgsSiteType;
use underbite_core::time::UnixTime;
use underbite_core::view::{Card, CardState, SolunarSettings, SpotView, ViewInput, build_view};

use crate::cli::Global;
use crate::runtime::env::{Env, var};
use crate::runtime::fetch::{FetchOptions, fetch_spot_with, measurement_limits};

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
    let fetched = fetch_spot_with(
        &spot,
        &config,
        now,
        env.transport.as_ref(),
        env.cache.as_ref(),
        &options,
    );
    if water_type.is_none() {
        eprintln!(
            "underbite: {}",
            inferred_text(fetched.inferred.as_ref(), spot.water_type)
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
        None => (default_config_path(env), false),
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

/// `$XDG_CONFIG_HOME/underbite/config.toml`, else `$HOME/.config/underbite/config.toml`.
fn default_config_path(env: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let absolute = |name: &str| env(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let base =
        absolute("XDG_CONFIG_HOME").or_else(|| absolute("HOME").map(|h| h.join(".config")))?;
    Some(base.join("underbite").join("config.toml"))
}

/// The water type a fetch inferred and why, or the one used when nothing pointed anywhere.
fn inferred_text(inferred: Option<&WaterTypeSuggestion>, fallback: WaterType) -> String {
    match inferred {
        Some(s) => {
            let reasons: Vec<String> = s.reasons.iter().map(reason_text).collect();
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

/// One reason for an inferred water type.
fn reason_text(reason: &WaterTypeReason) -> String {
    let named = |name: &Option<String>, what: &str| match name {
        Some(n) => format!("on {n} ({what})"),
        None => format!("on an unnamed {what}"),
    };
    match reason {
        WaterTypeReason::MarineZone { kind } => format!(
            "in an NWS {} marine zone",
            match kind {
                ZoneKind::Coastal => "coastal",
                ZoneKind::GreatLakesNearshore => "Great Lakes nearshore",
                ZoneKind::GreatLakesOpen => "Great Lakes open",
                ZoneKind::Offshore => "offshore",
            }
        ),
        WaterTypeReason::Waterbody { name, kind, .. } => named(
            name,
            match kind {
                WaterbodyKind::Lake => "lake",
                WaterbodyKind::Reservoir => "reservoir",
                WaterbodyKind::River => "river",
                WaterbodyKind::Estuary => "estuary",
                WaterbodyKind::Other => "waterbody",
            },
        ),
        WaterTypeReason::BayName { name } => format!("{name} is named as a bay or harbor"),
        WaterTypeReason::Stream { name, stream_order } => {
            let what = match stream_order {
                Some(order) => format!("stream of order {order}"),
                None => "stream".to_owned(),
            };
            named(name, &what)
        }
        WaterTypeReason::UsgsSite { site_type } => format!(
            "the nearest USGS site is {}",
            match site_type {
                UsgsSiteType::Stream => "on a stream",
                UsgsSiteType::Lake => "on a lake",
                UsgsSiteType::Estuary => "on an estuary",
                UsgsSiteType::TidalStream => "on a tidal stream",
                UsgsSiteType::Other => "of another kind",
            }
        ),
    }
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

/// What `build_view` takes, from the configuration.
fn view_input(config: &Config, snapshot: SpotSnapshot, now: UnixTime) -> ViewInput {
    let river_ranges = match config.rivers.as_slice() {
        [only] => Some(RiverRange::to_ranges(only)),
        _ => None,
    };
    ViewInput {
        card_order: config.cards.order_for(snapshot.spot.water_type),
        snapshot,
        now,
        units: config.units.display_units(),
        voice: config.presentation.voice,
        clock: config.presentation.clock,
        timeline_span: config.presentation.timeline_span(),
        limits: measurement_limits(&config.sources),
        river_ranges,
        #[allow(clippy::cast_possible_truncation)]
        trout_stress_c: Some(config.thresholds.trout_stress_c as f32),
        solunar: SolunarSettings {
            show: config.windows.show_solunar,
        },
    }
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
                writeln!(out, "  Problem: {}: {}", text("source"), text("message"))?;
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

/// A quantity as `text unit`.
fn quantity(q: &Value) -> String {
    let text = q.get("text").and_then(Value::as_str).unwrap_or_default();
    match q
        .get("unit")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
    {
        Some(unit) => format!("{text} {unit}"),
        None => text.to_owned(),
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
            inferred_text(Some(&s), WaterType::RiverStream),
            "water type tidal_river_estuary (inferred: in an NWS coastal marine zone; on an \
             unnamed river; --water-type sets another)"
        );
        let stream = WaterTypeReason::Stream {
            name: Some("Croton River".into()),
            stream_order: Some(7),
        };
        assert_eq!(reason_text(&stream), "on Croton River (stream of order 7)");
        assert!(inferred_text(None, WaterType::RiverStream).starts_with("water type river_stream"));
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
            default_config_path(&env),
            Some(PathBuf::from("/x/cfg/underbite/config.toml"))
        );
        let home_only = |name: &str| (name == "HOME").then(|| "/h".to_owned());
        assert_eq!(
            default_config_path(&home_only),
            Some(PathBuf::from("/h/.config/underbite/config.toml"))
        );
        assert_eq!(default_config_path(&|_: &str| None), None);
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
