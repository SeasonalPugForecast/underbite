//! `cargo xtask ci`: the gate every commit must pass.

use crate::run_cmd;

/// Extra feature sets that clippy and the tests check besides the defaults, as
/// `(package, cargo arguments)`. Features that exclude each other are listed one
/// set at a time, so `--all-features` is never used.
pub(crate) const FEATURE_SETS: &[(&str, &[&str])] = &[];

pub(crate) fn run() -> Result<(), String> {
    run_cmd("cargo", &["fmt", "--all", "--check"])?;
    run_cmd(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    for (package, extra) in FEATURE_SETS {
        let mut args = vec!["clippy", "--package", package, "--all-targets"];
        args.extend_from_slice(extra);
        args.extend_from_slice(&["--", "-D", "warnings"]);
        run_cmd("cargo", &args)?;
    }
    run_cmd("cargo", &["test", "--workspace"])?;
    for (package, extra) in FEATURE_SETS {
        let mut args = vec!["test", "--package", package];
        args.extend_from_slice(extra);
        run_cmd("cargo", &args)?;
    }
    if cargo_subcommand_available("deny") {
        run_cmd("cargo", &["deny", "check"])?;
    } else {
        println!("note: cargo-deny is not installed; skipping `cargo deny check`");
    }
    println!("ci: all checks passed");
    Ok(())
}

fn cargo_subcommand_available(name: &str) -> bool {
    std::process::Command::new("cargo")
        .args([name, "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_sets_never_use_all_features() {
        for (_, extra) in FEATURE_SETS {
            assert!(!extra.contains(&"--all-features"));
        }
    }

    #[test]
    fn unknown_cargo_subcommands_are_unavailable() {
        assert!(!cargo_subcommand_available(
            "definitely-not-a-cargo-subcommand"
        ));
    }
}
