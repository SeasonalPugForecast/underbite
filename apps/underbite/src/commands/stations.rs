//! `underbite stations`: the stations discovery finds near a point, ranked per measurement,
//! and the pins and exclusions that steer the ranking.

use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::Subcommand;
use serde_json::json;
use underbite_config::schema::Config;
use underbite_core::discover::rank::{CoverageState, MeasurementRanking, Reason, StationKey};
use underbite_core::model::{Measurement, Spot, WaterType};
use underbite_core::provenance::{Relation, SourceId};
use underbite_core::view::format::Formatter;
use underbite_providers::source::FetchContext;

use crate::cli::Global;
use crate::runtime::env::Env;
use crate::runtime::fetch::{chain_specs, discover, load_choices, requirements, save_choices};

/// Arguments of `underbite stations`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// What to do with stations.
    #[command(subcommand)]
    pub(crate) action: Action,
}

/// `stations` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Action {
    /// List stations near a point, ranked per measurement.
    Near {
        /// Latitude and longitude in decimal degrees, as `lat,lon`.
        #[arg(value_name = "LAT,LON", allow_hyphen_values = true)]
        location: String,
        /// Show only this measurement (`tide`, `tidal_current`, `discharge`, ...); repeatable.
        #[arg(long, value_name = "NAME")]
        measurement: Vec<String>,
        /// The kind of water at the point; it decides which measurements are ranked.
        #[arg(long, value_name = "TYPE", default_value = "tidal_river_estuary")]
        water_type: String,
        /// Always use this station: `source:id` or `source:id:bin`; repeatable.
        #[arg(long, value_name = "SOURCE:ID[:BIN]")]
        pin: Vec<String>,
        /// Never use this station: `source:id` or `source:id:bin`; repeatable.
        #[arg(long, value_name = "SOURCE:ID[:BIN]")]
        exclude: Vec<String>,
        /// Forget the pin or exclusion of this station; repeatable.
        #[arg(long, value_name = "SOURCE:ID[:BIN]")]
        clear: Vec<String>,
        /// How many stations to list per measurement (JSON always lists all).
        #[arg(long, value_name = "N", default_value_t = 5)]
        limit: usize,
        /// Also list the stations left out, with the reason for each.
        #[arg(long)]
        dropped: bool,
        /// Print the ranking as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Parses `lat,lon`.
fn parse_location(text: &str) -> anyhow::Result<(f64, f64)> {
    let (lat, lon) = text
        .split_once(',')
        .with_context(|| format!("{text:?} is not `lat,lon`"))?;
    let lat: f64 = lat.trim().parse().context("latitude")?;
    let lon: f64 = lon.trim().parse().context("longitude")?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        bail!("{text:?} is outside the globe");
    }
    Ok((lat, lon))
}

/// Parses `source:id[:bin]`.
fn parse_key(text: &str) -> anyhow::Result<StationKey> {
    let mut parts = text.splitn(3, ':');
    let (Some(source), Some(id)) = (parts.next(), parts.next()) else {
        bail!("{text:?} is not `source:id` or `source:id:bin`");
    };
    if source.is_empty() || id.is_empty() {
        bail!("{text:?} is not `source:id` or `source:id:bin`");
    }
    let key = StationKey::new(SourceId::new(source), id);
    Ok(match parts.next() {
        Some(bin) => key.with_bin(bin.parse().with_context(|| format!("bin of {text:?}"))?),
        None => key,
    })
}

/// A name in the configuration's snake case, as the enum it names.
fn parse_name<T: serde::de::DeserializeOwned>(what: &str, name: &str) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .map_err(|_| anyhow::anyhow!("unknown {what} {name:?}"))
}

/// The id a point's choices are stored under.
fn spot_id(latitude: f64, longitude: f64) -> String {
    format!("{latitude:.4},{longitude:.4}")
}

fn relation_text(relation: Relation) -> &'static str {
    match relation {
        Relation::SameWaterBody => "same water body",
        Relation::Upstream => "upstream",
        Relation::Downstream => "downstream",
        Relation::Nearby => "nearby",
        Relation::Gridded => "model grid cell",
        Relation::Satellite => "satellite",
    }
}

fn coverage_text(state: CoverageState) -> &'static str {
    match state {
        CoverageState::Reported => "reports values",
        CoverageState::Listed => "listed, not confirmed",
        CoverageState::NotReported => "no recent values",
    }
}

/// A ranking reason in words; distances and ages come from the formatter.
fn reason_text(f: &Formatter, reason: &Reason, now: i64) -> String {
    match reason {
        Reason::Pinned => "pinned by you".into(),
        Reason::Excluded => "excluded by you".into(),
        Reason::TooFar {
            distance_m,
            limit_m,
        } => f.far_reason(*distance_m, *limit_m),
        Reason::Stale { age_s, limit_s } => f.stale_reason(*age_s, *limit_s),
        Reason::WeakAndVariable => "weak and variable current: no predictions".into(),
        Reason::OtherBin { kept } => format!("bin {kept} of this station is used"),
        Reason::Reported { at } => format!("latest value {}", f.age(now - at.0)),
        Reason::Listed => "listed, not confirmed to report".into(),
        Reason::NotReported { .. } => "checked: no recent values".into(),
        Reason::Relation { relation } => relation_text(*relation).into(),
        Reason::OtherWaterBody { .. } => "on another river or a tributary".into(),
        Reason::NotPlaced => "not found on the mapped streams, so maybe another river".into(),
        Reason::ForecastGaugeFor { site } => format!("forecast gauge for {}", site.id),
        Reason::Secondary => "not the station's main product".into(),
        Reason::Distance { distance_m } => format!("{} away", f.distance(Some(*distance_m)).full()),
    }
}

fn describe(
    f: &Formatter,
    measurement: Measurement,
    ranking: &MeasurementRanking,
    now: i64,
    limit: usize,
    dropped: bool,
) -> String {
    let name = serde_json::to_value(measurement)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut out = String::new();
    let chain: Vec<&str> = ranking.chain.sources.iter().map(SourceId::as_str).collect();
    let _ = writeln!(out, "{name}  (sources: {})", chain.join(", "));
    if ranking.stations.is_empty() {
        let _ = writeln!(out, "  no station within reach");
    }
    for (i, s) in ranking.stations.iter().enumerate().take(limit) {
        let bin = s
            .station
            .bin
            .map(|b| format!(" bin {b}"))
            .unwrap_or_default();
        let age = s
            .age_s
            .map(|a| format!(", last value {}", f.age(a)))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {}. {} {}{bin}  {}  {}, {}{age}{}",
            i + 1,
            s.station.source,
            s.station.id,
            s.station.name,
            f.distance(Some(s.distance_m)).full(),
            relation_text(s.relation),
            if s.pinned { " [pinned]" } else { "" },
        );
        let _ = writeln!(out, "     {}", coverage_text(s.coverage));
        for reason in s
            .reasons
            .iter()
            .filter(|r| !matches!(r, Reason::Listed | Reason::NotReported { .. }))
        {
            let _ = writeln!(out, "     - {}", reason_text(f, reason, now));
        }
    }
    if ranking.stations.len() > limit {
        let _ = writeln!(
            out,
            "  {} more ranked (--limit shows more)",
            ranking.stations.len() - limit
        );
    }
    if !dropped && !ranking.dropped.is_empty() {
        let _ = writeln!(
            out,
            "  {} more left out (--dropped lists them)",
            ranking.dropped.len()
        );
    }
    for d in ranking.dropped.iter().filter(|_| dropped) {
        let bin = d
            .station
            .bin
            .map(|b| format!(" bin {b}"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  dropped: {} {}{bin} {} ({}): {}",
            d.station.source,
            d.station.id,
            d.station.name,
            f.distance(Some(d.distance_m)).full(),
            reason_text(f, &d.reason, now),
        );
    }
    out
}

/// Runs `underbite stations`.
pub(crate) fn run(args: &Args, global: &Global) -> anyhow::Result<ExitCode> {
    let Action::Near {
        location,
        measurement,
        water_type,
        pin,
        exclude,
        clear,
        limit,
        dropped,
        json,
    } = &args.action;
    let (latitude, longitude) = parse_location(location)?;
    let water: WaterType = parse_name("water type", water_type)?;
    let only: Vec<Measurement> = measurement
        .iter()
        .map(|m| parse_name("measurement", m))
        .collect::<Result<_, _>>()?;
    let env = Env::from_global(global)?;
    let id = spot_id(latitude, longitude);

    if !(pin.is_empty() && exclude.is_empty() && clear.is_empty()) {
        let cache = env.cache.as_ref().context(
            "no cache directory (set XDG_CACHE_HOME or HOME) to keep pins and exclusions",
        )?;
        let mut choices = load_choices(cache, &id);
        for text in pin {
            choices.pin(parse_key(text)?);
        }
        for text in exclude {
            choices.exclude(parse_key(text)?);
        }
        for text in clear {
            choices.clear(&parse_key(text)?);
        }
        save_choices(cache, &id, &choices, env.now).context("saving the choices")?;
    }

    let config = Config::default();
    let spot = Spot {
        id,
        name: format!("{latitude:.4}, {longitude:.4}"),
        latitude,
        longitude,
        water_type: water,
        water_body: None,
        time_zone: String::new(),
    };
    let mut wanted = requirements(&config, water);
    if !only.is_empty() {
        wanted.measurements.retain(|m| only.contains(m));
        if wanted.measurements.is_empty() {
            bail!("none of those measurements is shown for {water_type} water");
        }
    }
    let chains = chain_specs(&config.sources, &wanted, water);
    let host = env.host();
    let ctx = FetchContext::new(env.now, env.options.user_agent.clone())
        .with_coordinate_precision(config.sources.coordinate_precision)
        .with_secrets(env.options.secrets.clone());
    let found = discover(&host, &ctx, &spot, chains);

    if *json {
        let value = json!({
            "latitude": latitude,
            "longitude": longitude,
            "water_type": water,
            "measurements": found.ranking.measurements,
            "errors": found.errors,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(ExitCode::SUCCESS);
    }

    let f = Formatter::new(
        config.units.display_units(),
        config.presentation.voice,
        config.presentation.clock,
        "UTC",
    );
    println!("Stations near {latitude:.4}, {longitude:.4} ({water_type})");
    for (m, ranking) in &found.ranking.measurements {
        println!();
        print!("{}", describe(&f, *m, ranking, env.now.0, *limit, *dropped));
    }
    for e in &found.errors {
        eprintln!("note: {}: {}", e.source, e.message);
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_with_and_without_a_bin() {
        assert_eq!(
            parse_key("coops:8518750").unwrap(),
            StationKey::new(SourceId::COOPS, "8518750")
        );
        assert_eq!(
            parse_key("coops:NYH1927:13").unwrap(),
            StationKey::new(SourceId::COOPS, "NYH1927").with_bin(13)
        );
        assert!(parse_key("coops").is_err());
        assert!(parse_key("coops:").is_err());
        assert!(parse_key("coops:x:y").is_err());
    }

    #[test]
    fn locations_parse() {
        assert_eq!(
            parse_location("40.7033,-74.0170").unwrap(),
            (40.7033, -74.017)
        );
        assert!(parse_location("91,0").is_err());
        assert!(parse_location("40.7").is_err());
    }

    #[test]
    fn names_parse_as_the_configuration_spells_them() {
        let m: Measurement = parse_name("measurement", "tidal_current").unwrap();
        assert_eq!(m, Measurement::TidalCurrent);
        assert!(parse_name::<Measurement>("measurement", "tides").is_err());
    }
}
