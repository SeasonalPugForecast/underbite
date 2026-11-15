//! `underbite stations`: stations near a point.

use std::process::ExitCode;

use clap::Subcommand;

use crate::cli::Global;

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
    /// List stations near a point.
    Near {
        /// Latitude and longitude in decimal degrees, as `lat,lon`.
        #[arg(value_name = "LAT,LON", allow_hyphen_values = true)]
        location: String,
    },
}

/// Runs `underbite stations`.
// The command can fail once implemented; the stub cannot yet.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn run(_args: &Args, _global: &Global) -> anyhow::Result<ExitCode> {
    Ok(super::not_implemented("stations"))
}
