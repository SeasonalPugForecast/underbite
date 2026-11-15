//! `underbite tides`: tide predictions for a station.

use std::process::ExitCode;

use crate::cli::Global;

/// Arguments of `underbite tides`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Station id, for example 8518750.
    #[arg(value_name = "STATION")]
    pub(crate) station: String,
    /// Use only cached data.
    #[arg(long)]
    pub(crate) offline: bool,
}

/// Runs `underbite tides`.
// The command can fail once implemented; the stub cannot yet.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn run(_args: &Args, _global: &Global) -> anyhow::Result<ExitCode> {
    Ok(super::not_implemented("tides"))
}
