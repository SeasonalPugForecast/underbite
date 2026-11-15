//! Project automation, run as `cargo xtask <command>`.

mod ci;

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci                  fmt check, clippy, tests, cargo-deny";

/// A parsed xtask command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Ci,
    Help,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["ci"] => Ok(Command::Ci),
        [] | ["help" | "-h" | "--help"] => Ok(Command::Help),
        [name] => Err(format!("unknown command `{name}`")),
        _ => Err(format!("unrecognized arguments: {}", args.join(" "))),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match parse(&args) {
        Ok(Command::Ci) => ci::run(),
        Ok(Command::Help) => {
            println!("{USAGE}");
            Ok(())
        }
        Err(e) => Err(format!("{e}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Runs `program args…` in the workspace root, streaming output.
pub(crate) fn run_cmd(program: &str, args: &[&str]) -> Result<(), String> {
    println!("$ {program} {}", args.join(" "));
    let status = std::process::Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("failed to start `{program}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`{program} {}` failed with {status}",
            args.join(" ")
        ))
    }
}

/// The workspace root: the parent of the xtask crate directory.
pub(crate) fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn parses_known_commands() {
        assert_eq!(parse(&args("ci")), Ok(Command::Ci));
        assert_eq!(parse(&args("")), Ok(Command::Help));
    }

    #[test]
    fn rejects_unknown_input() {
        assert!(parse(&args("frobnicate")).is_err());
        assert!(parse(&args("ci extra")).is_err());
    }

    #[test]
    fn workspace_root_contains_the_workspace_manifest() {
        assert!(workspace_root().join("Cargo.toml").is_file());
    }
}
