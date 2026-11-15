//! Logging: a file under the XDG state directory, or stderr.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

/// The level: `-v` flags win, then the configured level, then `warn`.
pub(crate) fn level(verbose: u8, configured: Option<&str>) -> String {
    match verbose {
        0 => configured.unwrap_or("warn").to_string(),
        1 => "info".into(),
        2 => "debug".into(),
        _ => "trace".into(),
    }
}

/// The log file: `$XDG_STATE_HOME/underbite/underbite.log`, else
/// `$HOME/.local/state/underbite/underbite.log`. `env` looks up a variable.
// Used once an interactive command opens a log file.
#[allow(dead_code)]
pub(crate) fn log_path(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let non_empty = |name: &str| env(name).filter(|v| !v.is_empty());
    let base = match non_empty("XDG_STATE_HOME") {
        Some(state) => PathBuf::from(state),
        None => PathBuf::from(non_empty("HOME")?).join(".local/state"),
    };
    Some(base.join("underbite").join("underbite.log"))
}

/// Installs the global subscriber, writing to `file` (appending) or stderr.
pub(crate) fn init(level: &str, file: Option<&Path>) -> anyhow::Result<()> {
    let filter =
        EnvFilter::try_new(level).with_context(|| format!("invalid log level `{level}`"))?;
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);
    let result = match file {
        Some(path) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("creating {}", dir.display()))?;
            }
            let log = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("opening {}", path.display()))?;
            builder
                .with_ansi(false)
                .with_writer(Mutex::new(log))
                .try_init()
        }
        None => builder.with_writer(std::io::stderr).try_init(),
    };
    // A second init (tests) keeps the first subscriber.
    let _ = result;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_wins() {
        assert_eq!(level(0, None), "warn");
        assert_eq!(level(0, Some("debug")), "debug");
        assert_eq!(level(1, Some("error")), "info");
        assert_eq!(level(5, None), "trace");
    }

    #[test]
    fn bad_levels_are_errors() {
        assert!(init("not a level ===", None).is_err());
    }

    #[test]
    fn log_path_prefers_xdg_state_home() {
        let path = log_path(|k| match k {
            "XDG_STATE_HOME" => Some("/state".into()),
            "HOME" => Some("/home/a".into()),
            _ => None,
        });
        assert_eq!(path, Some(PathBuf::from("/state/underbite/underbite.log")));
    }

    #[test]
    fn log_path_falls_back_to_home() {
        let path = log_path(|k| match k {
            "XDG_STATE_HOME" => Some(String::new()),
            "HOME" => Some("/home/a".into()),
            _ => None,
        });
        assert_eq!(
            path,
            Some(PathBuf::from(
                "/home/a/.local/state/underbite/underbite.log"
            ))
        );
        assert_eq!(log_path(|_| None), None);
    }

    #[test]
    fn init_creates_the_log_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("nested").join("underbite.log");
        init("warn", Some(&file)).expect("init");
        assert!(file.exists());
    }
}
