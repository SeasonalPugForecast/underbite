//! The tool's [`FetchHost`]: the transport (the network, or recorded responses), the response
//! cache and the value cache on disk. Every request any command sends goes through it.

use std::future::ready;

use underbite_core::discover::cache::StationChoices;
use underbite_core::time::UnixTime;
use underbite_fetch::store::{self, spot_key};
use underbite_fetch::{Call, FetchHost, Fetched, TransportError};

use super::cache::Cache;
use super::transport::Transport;

/// A transport and an optional cache at one time.
#[derive(Clone, Copy)]
pub(crate) struct CliHost<'a> {
    transport: &'a dyn Transport,
    cache: Option<&'a Cache>,
    now: UnixTime,
}

impl std::fmt::Debug for CliHost<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliHost")
            .field("cache", &self.cache)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

impl<'a> CliHost<'a> {
    /// A host over `transport`, caching in `cache` (nothing without one), at `now`.
    pub(crate) fn new(
        transport: &'a dyn Transport,
        cache: Option<&'a Cache>,
        now: UnixTime,
    ) -> Self {
        Self {
            transport,
            cache,
            now,
        }
    }
}

impl CliHost<'_> {
    /// Answers `call` from the response cache, or sends it and stores what may be stored. A
    /// call that asks for a fresh copy goes out and is not stored.
    fn exchange(&self, call: &Call) -> Result<Fetched, TransportError> {
        if let Some(hit) = self.cache.and_then(|c| c.load_response(call, self.now)) {
            tracing::debug!(url = %call.redacted_url(), "cached");
            return Ok(hit);
        }
        tracing::debug!(url = %call.redacted_url(), "GET");
        let fetched = self.transport.send(call)?;
        tracing::debug!(
            status = fetched.status,
            bytes = fetched.body.len(),
            "response"
        );
        if let Some(cache) = self.cache
            && let Err(e) = cache.store_response(call, &fetched, self.now)
        {
            tracing::warn!(error = %e, "response not cached");
        }
        Ok(fetched)
    }

    /// Writes a value to the cache, when there is one.
    fn keep(&self, namespace: &str, key: &str, value: &str) {
        if let Some(cache) = self.cache
            && let Err(e) = cache.put_text(namespace, key, value)
        {
            tracing::warn!(namespace, error = %e, "not cached");
        }
    }

    /// Writes the ledger to the cache; without one there is nothing to keep it in.
    fn keep_usage(&self, ledger: &str) -> Result<(), String> {
        let Some(cache) = self.cache else {
            return Ok(());
        };
        cache
            .put_text(store::USAGE, store::LEDGER_KEY, ledger)
            .map_err(|e| e.to_string())
    }
}

/// Every call blocks until it is done: the tool sends one request at a time.
impl FetchHost for CliHost<'_> {
    fn send(&self, call: &Call) -> impl Future<Output = Result<Fetched, TransportError>> + Send {
        ready(self.exchange(call))
    }

    fn cached(&self, call: &Call) -> impl Future<Output = Option<Fetched>> + Send {
        ready(self.cache.and_then(|c| c.load_response(call, self.now)))
    }

    fn load(&self, namespace: &str, key: &str) -> impl Future<Output = Option<String>> + Send {
        ready(self.cache.and_then(|c| c.get_text(namespace, key)))
    }

    fn store(&self, namespace: &str, key: &str, value: String) -> impl Future<Output = ()> + Send {
        self.keep(namespace, key, &value);
        ready(())
    }

    fn save_usage(&self, ledger: String) -> impl Future<Output = Result<(), String>> + Send {
        ready(self.keep_usage(&ledger))
    }
}

/// The pins and exclusions of a spot.
pub(crate) fn load_choices(cache: &Cache, spot_id: &str) -> StationChoices {
    cache
        .get(store::CHOICES, &spot_key(spot_id))
        .map(|stored| stored.value)
        .unwrap_or_default()
}

/// Saves the pins and exclusions of a spot.
pub(crate) fn save_choices(
    cache: &Cache,
    spot_id: &str,
    choices: &StationChoices,
    now: UnixTime,
) -> std::io::Result<()> {
    cache.put(store::CHOICES, &spot_key(spot_id), choices, now)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::runtime::transport::Replay;
    use underbite_providers::request::CacheHint;

    const NOW: UnixTime = UnixTime(1_794_744_000);

    fn replay() -> Replay {
        Replay::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/replay/battery"),
        )
        .expect("routes")
    }

    const POINT: &str = "https://api.weather.gov/points/40.7,-74.02";

    #[test]
    fn responses_are_cached_as_their_hints_allow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let transport = replay();
        let host = CliHost::new(&transport, Some(&cache), NOW);
        let hinted = Call::get(POINT).with_cache(CacheHint {
            max_age: Some(60),
            ..CacheHint::default()
        });
        pollster::block_on(async {
            assert!(host.cached(&hinted).await.is_none());
            let sent = host.send(&hinted).await.expect("sent");
            assert_eq!(host.cached(&hinted).await, Some(sent.clone()));
            assert_eq!(host.send(&hinted).await, Ok(sent));
        });
        assert_eq!(transport.seen().len(), 1);
        let fresh = hinted.with_cache(CacheHint {
            fresh: true,
            ..CacheHint::default()
        });
        pollster::block_on(host.send(&fresh)).expect("sent");
        assert_eq!(transport.seen().len(), 2);
    }

    #[test]
    fn values_and_the_ledger_are_files_and_nothing_without_a_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let transport = replay();
        let host = CliHost::new(&transport, Some(&cache), NOW);
        let ledger = format!(r#"{{"saved_at":{},"value":{{}}}}"#, NOW.0);
        pollster::block_on(async {
            host.store("stations", "ndbc-active", ledger.clone()).await;
            assert_eq!(
                host.load("stations", "ndbc-active").await,
                Some(ledger.clone())
            );
            host.save_usage(ledger.clone()).await.expect("saved");
            assert_eq!(
                host.load(store::USAGE, store::LEDGER_KEY).await,
                Some(ledger.clone())
            );
        });
        assert!(dir.path().join("usage/ledger.json").exists());

        let bare = CliHost::new(&transport, None, NOW);
        pollster::block_on(async {
            bare.store("stations", "ndbc-active", ledger.clone()).await;
            assert_eq!(bare.load("stations", "ndbc-active").await, None);
            assert_eq!(bare.save_usage(ledger).await, Ok(()));
        });
    }

    #[test]
    fn choices_are_kept_per_spot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path());
        let mut choices = StationChoices::default();
        choices.exclude(underbite_core::discover::rank::StationKey::new(
            underbite_core::provenance::SourceId::COOPS,
            "8518750",
        ));
        save_choices(&cache, "battery", &choices, NOW).expect("saved");
        assert_eq!(load_choices(&cache, "battery"), choices);
        assert_eq!(load_choices(&cache, "croton"), StationChoices::default());
    }
}
