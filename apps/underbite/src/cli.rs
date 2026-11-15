//! Command-line arguments. Each subcommand's own arguments live with the command in
//! `commands/`.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::commands::{record, spot, stations, tides};

/// Conditions for a fishing spot from free data sources.
#[derive(Debug, Parser)]
#[command(name = "underbite", version, about)]
pub(crate) struct Cli {
    /// Options every command shares.
    #[command(flatten)]
    pub(crate) global: Global,
    /// What to do.
    #[command(subcommand)]
    pub(crate) command: Cmd,
}

/// Options every command shares.
#[derive(Debug, Args)]
pub(crate) struct Global {
    /// More logging (-v info, -vv debug, -vvv trace).
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub(crate) verbose: u8,
    /// Answer requests from recorded responses in this directory instead of the network.
    #[arg(long, global = true, hide = true, value_name = "DIR")]
    pub(crate) replay: Option<PathBuf>,
    /// Use this instant as the current time (RFC 3339), for reproducible runs.
    #[arg(long, global = true, hide = true, value_name = "TIME")]
    pub(crate) now: Option<String>,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Cmd {
    /// Show conditions for a spot.
    Spot(spot::Args),
    /// Work with tide and current stations.
    Stations(stations::Args),
    /// Show tide predictions for a station.
    Tides(tides::Args),
    /// Record provider responses as test fixtures (live; run by hand).
    Record(record::Args),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_each_subcommand() {
        let spot = Cli::try_parse_from(["underbite", "spot", "40.70,-74.02", "--json"]);
        assert!(matches!(
            spot,
            Ok(Cli {
                command: Cmd::Spot(spot::Args { json: true, .. }),
                ..
            })
        ));
        assert!(Cli::try_parse_from(["underbite", "stations", "near", "40.70,-74.02"]).is_ok());
        assert!(Cli::try_parse_from(["underbite", "tides", "8518750", "--offline"]).is_ok());
        assert!(Cli::try_parse_from(["underbite", "record", "coops"]).is_ok());
        assert!(Cli::try_parse_from(["underbite"]).is_err());
    }

    #[test]
    fn replay_and_now_are_global_and_hidden() {
        let cli = Cli::try_parse_from([
            "underbite",
            "spot",
            "40.70,-74.02",
            "--replay",
            "tests/fixtures/replay/battery",
            "--now",
            "2026-11-08T14:00:00Z",
        ])
        .unwrap();
        assert!(cli.global.replay.is_some());
        assert_eq!(cli.global.now.as_deref(), Some("2026-11-08T14:00:00Z"));
        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("--replay"));
    }
}
