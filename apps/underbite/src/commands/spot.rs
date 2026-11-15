//! `underbite spot`: conditions for a spot.

use std::process::ExitCode;

use crate::cli::Global;

/// Arguments of `underbite spot`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Latitude and longitude in decimal degrees, as `lat,lon`.
    #[arg(value_name = "LAT,LON", allow_hyphen_values = true)]
    pub(crate) location: String,
    /// Print the view as JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

/// Runs `underbite spot`.
// The command can fail once implemented; the stub cannot yet.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn run(_args: &Args, _global: &Global) -> anyhow::Result<ExitCode> {
    Ok(super::not_implemented("spot"))
}
