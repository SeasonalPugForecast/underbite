//! `underbite stations`: the stations discovery finds near a point, ranked per measurement,
//! and the pins and exclusions that steer the ranking.

use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context, bail};
use std::collections::BTreeMap;

use clap::Subcommand;
use serde_json::json;
use underbite_config::schema::Config;
use underbite_core::discover::cache::StationChoices;
use underbite_core::discover::rank::{MeasurementRanking, Reason, StationKey};
use underbite_core::model::{Measurement, Spot, WaterType};
use underbite_core::provenance::SourceId;
use underbite_core::time::UnixTime;
use underbite_core::view::format::Formatter;
use underbite_core::view::stations::{reason_text, relation_text};
use underbite_fetch::store::parse_station_key;
use underbite_fetch::{requirements, stations_near};

use super::spot::point_id;
use crate::cli::Global;
use crate::runtime::env::Env;
use crate::runtime::host::{load_choices, save_choices};

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
    Ok(parse_station_key(text)?)
}

/// A key as the command line writes it.
fn key_text(key: &StationKey) -> String {
    match key.bin {
        Some(bin) => format!("{}:{}:{bin}", key.source, key.id),
        None => format!("{}:{}", key.source, key.id),
    }
}

/// The pins and exclusions, in words, that cover no station of the rankings: a mistyped id, or
/// a station too far away to be a candidate for the measurements ranked.
fn unmatched_choices(
    choices: &StationChoices,
    rankings: &BTreeMap<Measurement, MeasurementRanking>,
) -> Vec<String> {
    let found: Vec<StationKey> = rankings
        .values()
        .flat_map(|r| {
            r.stations
                .iter()
                .map(|s| StationKey::of(&s.station))
                .chain(r.dropped.iter().map(|d| StationKey::of(&d.station)))
        })
        .collect();
    let missing = |what: &str, keys: &std::collections::BTreeSet<StationKey>| {
        keys.iter()
            .filter(|k| !found.iter().any(|f| k.covers(f)))
            .map(|k| {
                format!(
                    "the {what} {} matches no station found near here",
                    key_text(k)
                )
            })
            .collect::<Vec<_>>()
    };
    let mut out = missing("pin", &choices.pinned);
    out.extend(missing("exclusion", &choices.excluded));
    out
}

/// A name in the configuration's snake case, as the enum it names.
fn parse_name<T: serde::de::DeserializeOwned>(what: &str, name: &str) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .map_err(|_| anyhow::anyhow!("unknown {what} {name:?}"))
}

/// The reasons of a used station worth a line: its distance and relation are already on the
/// station's line, and "listed" only stands while nothing says whether it reports.
fn used_reasons(reasons: &[Reason]) -> impl Iterator<Item = &Reason> {
    let settled = reasons
        .iter()
        .any(|r| matches!(r, Reason::Reported { .. } | Reason::NotReported { .. }));
    reasons.iter().filter(move |r| match r {
        Reason::Distance { .. } | Reason::Relation { .. } => false,
        Reason::Listed => !settled,
        _ => true,
    })
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
            relation_text(f, s.relation),
            if s.pinned { " [pinned]" } else { "" },
        );
        for reason in used_reasons(&s.reasons) {
            let _ = writeln!(out, "     - {}", reason_text(f, reason, UnixTime(now)));
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
            reason_text(f, &d.reason, UnixTime(now)),
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
    let id = point_id(latitude, longitude);

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
    let found = pollster::block_on(stations_near(
        &env.host(),
        &spot,
        &config,
        &wanted,
        env.now,
        &env.options,
    ));
    if let Some(cache) = env.cache.as_ref() {
        let choices = load_choices(cache, &spot.id);
        for warning in unmatched_choices(&choices, &found.ranking.measurements) {
            eprintln!("warning: {warning}");
        }
    }

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
    fn keys_read_as_the_store_reads_them() {
        assert_eq!(
            parse_key("usgs:01375000").unwrap(),
            StationKey::new(SourceId::USGS, "USGS-01375000")
        );
        assert!(parse_key("coops").is_err());
    }

    #[test]
    fn a_choice_that_matches_nothing_is_reported() {
        use underbite_core::discover::rank::{ChainSpec, DroppedStation};
        use underbite_core::provenance::StationRef;
        use underbite_core::provenance::{StationKind, UsgsSiteType};
        let station = |id: &str| StationRef {
            source: SourceId::USGS,
            id: id.into(),
            name: "Croton R".into(),
            kind: StationKind::UsgsSite(UsgsSiteType::Stream),
            latitude: 41.2,
            longitude: -73.8,
            bin: None,
            reference_id: None,
        };
        let ranking = MeasurementRanking {
            chain: ChainSpec {
                sources: vec![SourceId::USGS],
                limits: underbite_core::discover::rank::Limits::default(),
            },
            stations: Vec::new(),
            dropped: vec![DroppedStation {
                station: station("USGS-01375000"),
                distance_m: 100.0,
                reason: Reason::Excluded,
            }],
        };
        let rankings = BTreeMap::from([(Measurement::Discharge, ranking)]);
        let mut choices = StationChoices::default();
        choices.exclude(parse_key("usgs:01375000").unwrap());
        choices.pin(parse_key("usgs:01375001").unwrap());
        choices.pin(parse_key("ndbc:44025").unwrap());
        let found = unmatched_choices(&choices, &rankings);
        assert_eq!(
            found,
            [
                "the pin ndbc:44025 matches no station found near here",
                "the pin usgs:USGS-01375001 matches no station found near here",
            ]
        );
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
