//! One module per subcommand: its arguments and its `run`.

use std::process::ExitCode;

pub(crate) mod record;
pub(crate) mod spot;
pub(crate) mod stations;
pub(crate) mod tides;

/// Exit status of a command that is not implemented yet.
const NOT_IMPLEMENTED: u8 = 2;

/// Reports that `name` is not implemented yet.
pub(crate) fn not_implemented(name: &str) -> ExitCode {
    eprintln!("underbite {name}: not implemented yet");
    ExitCode::from(NOT_IMPLEMENTED)
}
