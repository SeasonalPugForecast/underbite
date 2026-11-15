//! Underbite's command-line tool.

mod cli;
mod logging;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Cmd};

/// Exit status of a command that is not implemented yet.
const NOT_IMPLEMENTED: u8 = 2;

fn not_implemented(name: &str) -> ExitCode {
    eprintln!("underbite {name}: not implemented yet");
    ExitCode::from(NOT_IMPLEMENTED)
}

fn run(cli: &Cli) -> anyhow::Result<ExitCode> {
    logging::init(&logging::level(cli.verbose, None), None)?;
    Ok(match &cli.command {
        Cmd::Spot { .. } => not_implemented("spot"),
        Cmd::Stations { .. } => not_implemented("stations"),
        Cmd::Tides { .. } => not_implemented("tides"),
        Cmd::Record { .. } => not_implemented("record"),
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("underbite: {err:#}");
            ExitCode::FAILURE
        }
    }
}
