//! What every command that talks to the sources starts from: the transport (the network, or
//! recorded responses with `--replay`), the response cache from the environment, the current
//! time (`--now`, else the clock) and the fetch options. The clock is read here and nowhere
//! else.

use anyhow::{Context, anyhow};
use underbite_core::time::{UnixTime, parse_rfc3339};
use underbite_fetch::FetchOptions;
use underbite_providers::source::Secrets;

use super::cache::Cache;
use super::host::CliHost;
use super::transport::{Replay, Transport, UreqTransport, user_agent};
use crate::cli::Global;

/// A command's way out to the sources and its notion of now.
pub(crate) struct Env {
    /// The network, or the recorded responses of `--replay`.
    pub(crate) transport: Box<dyn Transport>,
    /// The response and value cache; none when the environment names no directory.
    pub(crate) cache: Option<Cache>,
    /// `--now`, else the clock.
    pub(crate) now: UnixTime,
    /// Metered calls, keys and the User-Agent.
    pub(crate) options: FetchOptions,
}

impl Env {
    /// The environment the global options and the process environment ask for.
    pub(crate) fn from_global(global: &Global) -> anyhow::Result<Self> {
        Self::new(global, &var, clock)
    }

    /// [`Env::from_global`] with the environment read through `lookup` and the time from
    /// `clock` when there is no `--now`.
    fn new(
        global: &Global,
        lookup: &dyn Fn(&str) -> Option<String>,
        clock: impl FnOnce() -> UnixTime,
    ) -> anyhow::Result<Self> {
        let now = match &global.now {
            Some(text) => parse_now(text)?,
            None => clock(),
        };
        let transport: Box<dyn Transport> = match &global.replay {
            Some(dir) => Box::new(Replay::load(dir).context("--replay")?),
            None => Box::new(UreqTransport::new(&user_agent())),
        };
        Ok(Self {
            transport,
            cache: Cache::from_env(lookup).context("the cache directory")?,
            now,
            options: fetch_options(lookup),
        })
    }

    /// A host over this environment's transport and cache, at its time.
    pub(crate) fn host(&self) -> CliHost<'_> {
        CliHost::new(self.transport.as_ref(), self.cache.as_ref(), self.now)
    }
}

/// Metered calls allowed, the USGS key from `USGS_API_KEY` (read through `lookup`), the tool's
/// User-Agent, and the spot's own water type.
fn fetch_options(lookup: impl Fn(&str) -> Option<String>) -> FetchOptions {
    FetchOptions {
        allow_metered: true,
        secrets: Secrets {
            usgs_api_key: lookup("USGS_API_KEY").filter(|k| !k.trim().is_empty()),
        },
        user_agent: user_agent(),
        infer_water_type: false,
    }
}

/// An environment variable, `None` when unset or not Unicode.
pub(crate) fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// `--now`: an RFC 3339 time.
fn parse_now(text: &str) -> anyhow::Result<UnixTime> {
    parse_rfc3339(text).ok_or_else(|| anyhow!("--now {text:?} is not an RFC 3339 time"))
}

/// The current time; the only place the tool reads the clock.
fn clock() -> UnixTime {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    UnixTime(secs)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn global(replay: Option<PathBuf>, now: Option<&str>) -> Global {
        Global {
            verbose: 0,
            replay,
            now: now.map(str::to_owned),
        }
    }

    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    fn battery() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/replay/battery")
    }

    #[test]
    fn now_comes_from_the_flag_else_the_clock() {
        let env = lookup(&[]);
        let flagged = Env::new(&global(None, Some("2026-11-08T14:30:00Z")), &env, || {
            panic!("the clock is not read when --now is given")
        })
        .unwrap();
        assert_eq!(flagged.now, parse_rfc3339("2026-11-08T14:30:00Z").unwrap());
        let clocked = Env::new(&global(None, None), &env, || UnixTime(42)).unwrap();
        assert_eq!(clocked.now, UnixTime(42));
        assert!(Env::new(&global(None, Some("yesterday")), &env, || UnixTime(0)).is_err());
    }

    #[test]
    fn replay_answers_from_its_directory_and_a_missing_one_fails() {
        let env = lookup(&[]);
        let replayed = Env::new(&global(Some(battery()), None), &env, || UnixTime(0)).unwrap();
        let call =
            underbite_fetch::Call::get("https://api.weather.gov/points/40.7,-74.02".to_owned());
        assert_eq!(replayed.transport.send(&call).unwrap().status, 200);
        let missing = PathBuf::from("/nonexistent/replay");
        assert!(Env::new(&global(Some(missing), None), &env, || UnixTime(0)).is_err());
    }

    #[test]
    fn the_cache_follows_the_environment_and_refuses_relative_paths() {
        let at = |pairs| {
            Env::new(&global(None, None), &lookup(pairs), || UnixTime(0))
                .map(|e| e.cache.map(|c| c.dir().to_path_buf()))
        };
        assert_eq!(
            at(&[("XDG_CACHE_HOME", "/x/cache"), ("HOME", "/h")]).unwrap(),
            Some(PathBuf::from("/x/cache/underbite/v1"))
        );
        assert_eq!(
            at(&[("XDG_CACHE_HOME", ""), ("HOME", "/h")]).unwrap(),
            Some(PathBuf::from("/h/.cache/underbite/v1"))
        );
        assert_eq!(at(&[]).unwrap(), None);
        let err = at(&[("XDG_CACHE_HOME", "rel"), ("HOME", "/h")]).unwrap_err();
        assert!(format!("{err:#}").contains("XDG_CACHE_HOME"), "{err:#}");
        assert!(at(&[("HOME", "h")]).is_err());
    }

    #[test]
    fn options_read_the_key_from_the_environment() {
        let env = Env::new(
            &global(None, None),
            &lookup(&[("USGS_API_KEY", "k")]),
            || UnixTime(0),
        )
        .unwrap();
        assert_eq!(env.options.secrets.usgs_api_key.as_deref(), Some("k"));
    }
}
