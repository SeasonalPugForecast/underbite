//! `underbite record`: record provider responses as test fixtures.

use std::process::ExitCode;

use crate::cli::Global;

/// Arguments of `underbite record`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Provider to record, for example `coops`.
    #[arg(value_name = "PROVIDER")]
    pub(crate) provider: String,
}

/// Runs `underbite record`.
// The command can fail once implemented; the stub cannot yet.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn run(_args: &Args, _global: &Global) -> anyhow::Result<ExitCode> {
    Ok(super::not_implemented("record"))
}
