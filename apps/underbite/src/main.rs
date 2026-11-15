//! Underbite's command-line tool.

mod cli;
mod commands;
mod logging;
mod runtime;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Cmd};

fn run(cli: &Cli) -> anyhow::Result<ExitCode> {
    logging::init(&logging::level(cli.global.verbose, None), None)?;
    match &cli.command {
        Cmd::Spot(args) => commands::spot::run(args, &cli.global),
        Cmd::Stations(args) => commands::stations::run(args, &cli.global),
        Cmd::Tides(args) => commands::tides::run(args, &cli.global),
        Cmd::Record(args) => commands::record::run(args, &cli.global),
    }
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
