//! Command-line arguments.

use clap::{ArgAction, Parser, Subcommand};

/// Conditions for a fishing spot from free data sources.
#[derive(Debug, Parser)]
#[command(name = "underbite", version, about)]
pub(crate) struct Cli {
    /// More logging (-v info, -vv debug, -vvv trace).
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub(crate) verbose: u8,
    /// What to do.
    #[command(subcommand)]
    pub(crate) command: Cmd,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum Cmd {
    /// Show conditions for a spot.
    Spot {
        /// Latitude and longitude in decimal degrees, as `lat,lon`.
        #[arg(value_name = "LAT,LON")]
        location: String,
        /// Print the snapshot as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Work with tide and current stations.
    Stations {
        #[command(subcommand)]
        action: StationsCmd,
    },
    /// Show tide predictions for a station.
    Tides {
        /// Station id, for example 8518750.
        #[arg(value_name = "STATION")]
        station: String,
        /// Use only cached data.
        #[arg(long)]
        offline: bool,
    },
    /// Record provider responses as test fixtures (live; run by hand).
    Record {
        /// Provider to record, for example `coops`.
        #[arg(value_name = "PROVIDER")]
        provider: String,
    },
}

/// `stations` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum StationsCmd {
    /// List stations near a point.
    Near {
        /// Latitude and longitude in decimal degrees, as `lat,lon`.
        #[arg(value_name = "LAT,LON")]
        location: String,
    },
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
                command: Cmd::Spot { json: true, .. },
                ..
            })
        ));
        assert!(Cli::try_parse_from(["underbite", "stations", "near", "40.70,-74.02"]).is_ok());
        assert!(Cli::try_parse_from(["underbite", "tides", "8518750", "--offline"]).is_ok());
        assert!(Cli::try_parse_from(["underbite", "record", "coops"]).is_ok());
        assert!(Cli::try_parse_from(["underbite"]).is_err());
    }
}
