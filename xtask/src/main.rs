//! Project automation, run as `cargo xtask <command>`.

mod ci;
mod hooks;
mod ios;
mod ios_licenses;
mod ios_remote;
mod lane;
mod remote;
mod vendor;

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci                  fmt check, clippy, tests, cargo-deny
  hooks install [--force]
                      install a git pre-push hook that runs `cargo xtask remote ci --or-local`
  remote doctor|sync|ci|run [--host user@host] [--fetch path]… [-- command…]
                      build on a faster Linux machine over SSH (hosts: remote.local.toml);
                      dependencies are vendored here and shipped, so the host needs no
                      access to the git server
  remote clean --lane <name> | --all-lanes
                      delete a lane's directory on the build host
                      (a linked git worktree is a lane with its own directories)
  ios build [--debug] the iOS core: static libraries and Swift bindings; on a Mac
                      also ios/UnderbiteKit/UnderbiteFFI.xcframework
  ios licenses        write the app's open-source notices
                      (ios/Underbite/Underbite/Resources/Acknowledgements.json)
  ios remote doctor|sync|build|test|app|shots|device [--host user@mac] [--dir d] [--dry-run]
                      the same on the Mac over SSH (host: ios/remote.local.toml)
  ios remote ui [--on iphone|ipad] [--only Class/test]
                      the UI and accessibility tests on the iPhone and iPad simulators,
                      accessibility switched on and each simulator booted again
  ios remote exec [--fetch path] -- <command>
                      run a command in the lane's directory on the Mac, Xcode selected
  ios remote clean --lane <name> | --all-lanes
                      delete a lane's directory and simulators on the Mac";

/// A parsed xtask command line.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Ci,
    HooksInstall { force: bool },
    Remote { args: Vec<String> },
    Ios { args: Vec<String> },
    Help,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["ci"] => Ok(Command::Ci),
        ["hooks", "install"] => Ok(Command::HooksInstall { force: false }),
        ["hooks", "install", "--force"] => Ok(Command::HooksInstall { force: true }),
        [] | ["help" | "-h" | "--help"] => Ok(Command::Help),
        ["remote", rest @ ..] => Ok(Command::Remote {
            args: rest.iter().map(|s| (*s).to_owned()).collect(),
        }),
        ["ios", rest @ ..] => Ok(Command::Ios {
            args: rest.iter().map(|s| (*s).to_owned()).collect(),
        }),
        [name] => Err(format!("unknown command `{name}`")),
        _ => Err(format!("unrecognized arguments: {}", args.join(" "))),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match parse(&args) {
        Ok(Command::Ci) => ci::run(),
        Ok(Command::HooksInstall { force }) => hooks::install(force),
        Ok(Command::Remote { args }) => {
            remote::run(&args.iter().map(String::as_str).collect::<Vec<_>>())
        }
        Ok(Command::Ios { args }) => ios::run(&args.iter().map(String::as_str).collect::<Vec<_>>()),
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
        assert_eq!(
            parse(&args("hooks install")),
            Ok(Command::HooksInstall { force: false })
        );
        assert_eq!(
            parse(&args("hooks install --force")),
            Ok(Command::HooksInstall { force: true })
        );
        assert_eq!(parse(&args("")), Ok(Command::Help));
    }

    #[test]
    fn remote_takes_its_arguments() {
        assert_eq!(
            parse(&args("remote run -- cargo test")),
            Ok(Command::Remote {
                args: vec!["run".into(), "--".into(), "cargo".into(), "test".into()]
            })
        );
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
