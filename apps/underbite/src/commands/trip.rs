//! `underbite trip`: each day of a trip at each place, with what every value is based on.

use std::process::ExitCode;

use anyhow::bail;

use crate::cli::Global;

/// Arguments of `underbite trip`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {}

/// Not available yet.
pub(crate) fn run(_args: &Args, _global: &Global) -> anyhow::Result<ExitCode> {
    bail!("the trip command is not available yet")
}
