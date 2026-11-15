//! The on-disk cache: JSON files under `$XDG_CACHE_HOME/underbite/v1`, written atomically.
//!
//! Two kinds of entries live here. Values (station lists, harmonic metadata, normals,
//! discovery results, the last snapshot of a spot, the usage ledger) are JSON files by
//! namespace and key. Responses are a body file and a small metadata file named by a hash of the
//! redacted URL. Neither URLs nor keys appear in a file or in a file name, a response to a
//! keyed request is never written, and a request for a fresh copy bypasses the cache.

#![allow(dead_code)] // The fetch driver and the commands are the callers; they are wired in separately.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use underbite_core::time::UnixTime;

use super::transport::{Call, Fetched};

/// Directory name of the current cache format, below `underbite/`.
const VERSION_DIR: &str = "v1";

/// How long a CO-OPS station list stays valid, seconds.
pub(crate) const COOPS_STATION_LIST_TTL_S: i64 = 7 * 86_400;
/// How long the other stations lists stay valid, seconds.
pub(crate) const STATION_LIST_TTL_S: i64 = 86_400;
/// How long harmonic constants stay valid, seconds.
pub(crate) const HARMONICS_TTL_S: i64 = 90 * 86_400;

/// A value with when it was stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Stored<T> {
    /// When it was stored, Unix seconds.
    pub(crate) saved_at: i64,
    /// The value.
    pub(crate) value: T,
}

/// What a cached response needs besides its body.
#[derive(Debug, Serialize, Deserialize)]
struct ResponseMeta {
    saved_at: i64,
    valid_until: i64,
    status: u16,
    content_type: Option<String>,
    date: Option<i64>,
    expires: Option<i64>,
    cache_max_age: Option<u32>,
    last_modified: Option<String>,
    etag: Option<String>,
    rate_limit_remaining: Option<u32>,
}

/// The cache directory.
#[derive(Clone, Debug)]
pub(crate) struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// A cache rooted at `dir` (created on first write).
    pub(crate) fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The default cache: `$XDG_CACHE_HOME/underbite/v1`, or `$HOME/.cache/underbite/v1`.
    /// `lookup` reads the environment, so tests inject theirs.
    pub(crate) fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let absolute = |name: &str| lookup(name).map(PathBuf::from).filter(|p| p.is_absolute());
        let root =
            absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|h| h.join(".cache")))?;
        Some(Self::at(root.join("underbite").join(VERSION_DIR)))
    }

    /// The directory.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The key of a spot: a hash of its id, never its coordinates.
    pub(crate) fn spot_key(spot_id: &str) -> String {
        format!("spot-{:016x}", fnv1a(spot_id.as_bytes()))
    }

    fn value_path(&self, namespace: &str, key: &str) -> io::Result<PathBuf> {
        for part in [namespace, key] {
            let ok = !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
            if !ok {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cache names are letters, digits, '-', '_' and '.'",
                ));
            }
        }
        Ok(self.dir.join(namespace).join(format!("{key}.json")))
    }

    /// Stores `value` under `namespace` and `key`, stamped with `now`.
    pub(crate) fn put<T: Serialize>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
        now: UnixTime,
    ) -> io::Result<()> {
        let path = self.value_path(namespace, key)?;
        let json = serde_json::to_vec(&Stored {
            saved_at: now.0,
            value,
        })
        .map_err(io::Error::other)?;
        write_atomic(&path, &json)
    }

    /// The stored value. A corrupt file is logged and deleted.
    pub(crate) fn get<T: DeserializeOwned>(&self, namespace: &str, key: &str) -> Option<Stored<T>> {
        let path = self.value_path(namespace, key).ok()?;
        let bytes = fs::read(&path).ok()?;
        match serde_json::from_slice(&bytes) {
            Ok(stored) => Some(stored),
            Err(e) => {
                tracing::warn!(namespace, error = %e, "deleting corrupt cache file");
                let _ = fs::remove_file(&path);
                None
            }
        }
    }

    /// The stored value when it is at most `max_age_s` old at `now`.
    pub(crate) fn get_within<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
        now: UnixTime,
        max_age_s: i64,
    ) -> Option<T> {
        let stored = self.get::<T>(namespace, key)?;
        let age = now.0 - stored.saved_at;
        (0..=max_age_s).contains(&age).then_some(stored.value)
    }

    /// Deletes a stored value; a missing one is not an error.
    pub(crate) fn remove(&self, namespace: &str, key: &str) -> io::Result<()> {
        match fs::remove_file(self.value_path(namespace, key)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn response_paths(&self, call: &Call) -> (PathBuf, PathBuf) {
        let name = format!("{:016x}", fnv1a(call.redacted_url().as_bytes()));
        let dir = self.dir.join("responses");
        (
            dir.join(format!("{name}.json")),
            dir.join(format!("{name}.body")),
        )
    }

    /// A stored response for `call` that is still valid at `now`. Never for a call that asks
    /// for a fresh copy or must not be stored.
    pub(crate) fn load_response(&self, call: &Call, now: UnixTime) -> Option<Fetched> {
        if call.cache.fresh || !call.may_store_on_disk() {
            return None;
        }
        let (meta_path, body_path) = self.response_paths(call);
        let meta: ResponseMeta = serde_json::from_slice(&fs::read(&meta_path).ok()?).ok()?;
        if now.0 < meta.saved_at || now.0 >= meta.valid_until {
            return None;
        }
        let body = fs::read(&body_path).ok()?;
        Some(Fetched {
            status: meta.status,
            body,
            content_type: meta.content_type,
            date: meta.date.map(UnixTime),
            expires: meta.expires.map(UnixTime),
            cache_max_age: meta.cache_max_age,
            last_modified: meta.last_modified,
            etag: meta.etag,
            retry_after: None,
            rate_limit_remaining: meta.rate_limit_remaining,
        })
    }

    /// Stores a successful response for `call`, when it may be: not for a keyed or no-store
    /// call, not for a fresh-copy call, and only when the server's `Expires`, its `max-age` or
    /// the call's own `max_age` says how long it lasts. Returns whether it was written.
    pub(crate) fn store_response(
        &self,
        call: &Call,
        response: &Fetched,
        now: UnixTime,
    ) -> io::Result<bool> {
        if call.cache.fresh || !call.may_store_on_disk() || !(200..300).contains(&response.status) {
            return Ok(false);
        }
        let valid_until = match (
            response.expires,
            response.cache_max_age.or(call.cache.max_age),
        ) {
            (_, Some(max_age)) => now.0 + i64::from(max_age),
            (Some(expires), None) => expires.0,
            (None, None) => return Ok(false),
        };
        if valid_until <= now.0 {
            return Ok(false);
        }
        let meta = ResponseMeta {
            saved_at: now.0,
            valid_until,
            status: response.status,
            content_type: response.content_type.clone(),
            date: response.date.map(|t| t.0),
            expires: response.expires.map(|t| t.0),
            cache_max_age: response.cache_max_age,
            last_modified: response.last_modified.clone(),
            etag: response.etag.clone(),
            rate_limit_remaining: response.rate_limit_remaining,
        };
        let (meta_path, body_path) = self.response_paths(call);
        // The body first: a reader only trusts a body whose metadata exists.
        write_atomic(&body_path, &response.body)?;
        write_atomic(
            &meta_path,
            &serde_json::to_vec(&meta).map_err(io::Error::other)?,
        )?;
        Ok(true)
    }
}

/// FNV-1a, 64 bit: a stable name for a key (std's hashers are not stable across releases).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to a unique temporary file next to `path`, syncs it and renames it over
/// `path`, so readers see the old or the new content, never a mix.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy());
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{unique}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use underbite_providers::request::CacheHint;

    const NOW: UnixTime = UnixTime(1_794_744_000);

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::at(dir.path().join("underbite").join("v1"));
        (dir, cache)
    }

    fn list_files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(list_files(&path));
            } else {
                out.push(path.display().to_string());
            }
        }
        out
    }

    fn json_response(max_age: Option<u32>) -> Fetched {
        Fetched {
            content_type: Some("application/json".into()),
            cache_max_age: max_age,
            ..Fetched::new(200, "{\"a\":1}")
        }
    }

    #[test]
    fn the_directory_follows_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).to_string())
            }
        };
        let xdg = Cache::from_env(env(&[("XDG_CACHE_HOME", "/x/cache"), ("HOME", "/h")]));
        assert_eq!(xdg.expect("dir").dir(), Path::new("/x/cache/underbite/v1"));
        let home = Cache::from_env(env(&[("HOME", "/h")]));
        assert_eq!(
            home.expect("dir").dir(),
            Path::new("/h/.cache/underbite/v1")
        );
        let relative = Cache::from_env(env(&[("XDG_CACHE_HOME", "rel"), ("HOME", "/h")]));
        assert_eq!(
            relative.expect("dir").dir(),
            Path::new("/h/.cache/underbite/v1")
        );
        assert!(Cache::from_env(env(&[])).is_none());
    }

    #[test]
    fn values_round_trip_and_expire() {
        let (_guard, cache) = cache();
        let list = vec!["8518750".to_string(), "8531680".to_string()];
        cache
            .put("stations-coops", "list", &list, NOW)
            .expect("put");
        let later = UnixTime(NOW.0 + 3 * 86_400);
        assert_eq!(
            cache.get_within::<Vec<String>>(
                "stations-coops",
                "list",
                later,
                COOPS_STATION_LIST_TTL_S
            ),
            Some(list.clone())
        );
        assert_eq!(
            cache.get_within::<Vec<String>>("stations-coops", "list", later, STATION_LIST_TTL_S),
            None
        );
        assert_eq!(
            cache
                .get::<Vec<String>>("stations-coops", "list")
                .map(|s| s.saved_at),
            Some(NOW.0)
        );
        cache.remove("stations-coops", "list").expect("remove");
        cache
            .remove("stations-coops", "list")
            .expect("remove twice");
        assert!(cache.get::<Vec<String>>("stations-coops", "list").is_none());
    }

    #[test]
    fn corrupt_files_are_deleted_and_bad_names_refused() {
        let (_guard, cache) = cache();
        write_atomic(&cache.dir().join("ns").join("k.json"), b"{not json").expect("write");
        assert!(cache.get::<u8>("ns", "k").is_none());
        assert!(!cache.dir().join("ns").join("k.json").exists());
        assert!(cache.put("ns", "../escape", &1u8, NOW).is_err());
        assert!(cache.put("a/b", "k", &1u8, NOW).is_err());
        assert!(cache.put("ns", "", &1u8, NOW).is_err());
        assert!(cache.put("ns", ".hidden", &1u8, NOW).is_err());
    }

    #[test]
    fn spot_keys_hide_the_coordinates() {
        let key = Cache::spot_key("battery");
        assert!(key.starts_with("spot-") && key.len() == 21, "{key}");
        assert_eq!(key, Cache::spot_key("battery"));
        assert_ne!(key, Cache::spot_key("croton"));
    }

    #[test]
    fn atomic_writes_leave_no_temporary_files() {
        let (_guard, cache) = cache();
        let path = cache.dir().join("ns").join("k.json");
        write_atomic(&path, b"one").expect("write");
        write_atomic(&path, b"two").expect("overwrite");
        assert_eq!(fs::read(&path).expect("read"), b"two");
        let files = list_files(cache.dir());
        assert_eq!(files.len(), 1, "{files:?}");
        // A failed write cleans up after itself: the target is a directory.
        let dir_target = cache.dir().join("ns").join("d.json");
        fs::create_dir_all(&dir_target).expect("dir");
        assert!(write_atomic(&dir_target, b"x").is_err());
        assert!(
            list_files(cache.dir())
                .iter()
                .all(|f| Path::new(f).extension().is_none_or(|e| e != "tmp"))
        );
    }

    #[test]
    fn responses_are_reused_until_they_expire() {
        let (_guard, cache) = cache();
        let call = Call::get("https://api.example.test/stations.json?type=tidepredictions");
        assert!(
            !cache
                .store_response(&call, &json_response(None), NOW)
                .expect("store")
        );
        assert!(
            cache
                .store_response(&call, &json_response(Some(600)), NOW)
                .expect("store")
        );
        let hit = cache
            .load_response(&call, UnixTime(NOW.0 + 599))
            .expect("hit");
        assert_eq!(hit.body, b"{\"a\":1}");
        assert_eq!(hit.content_type.as_deref(), Some("application/json"));
        assert!(cache.load_response(&call, UnixTime(NOW.0 + 600)).is_none());
        assert!(
            cache
                .load_response(&Call::get("https://api.example.test/other"), NOW)
                .is_none()
        );
    }

    #[test]
    fn the_calls_own_max_age_and_expires_set_the_lifetime() {
        let (_guard, cache) = cache();
        let hinted = Call::get("https://api.example.test/a").with_cache(CacheHint {
            max_age: Some(60),
            ..CacheHint::default()
        });
        assert!(
            cache
                .store_response(&hinted, &json_response(None), NOW)
                .expect("store")
        );
        assert!(cache.load_response(&hinted, UnixTime(NOW.0 + 59)).is_some());
        assert!(cache.load_response(&hinted, UnixTime(NOW.0 + 60)).is_none());

        let expiring = Call::get("https://api.example.test/b");
        let response = Fetched {
            expires: Some(UnixTime(NOW.0 + 30)),
            ..json_response(None)
        };
        assert!(
            cache
                .store_response(&expiring, &response, NOW)
                .expect("store")
        );
        assert!(
            cache
                .load_response(&expiring, UnixTime(NOW.0 + 29))
                .is_some()
        );
        assert!(
            cache
                .load_response(&expiring, UnixTime(NOW.0 + 30))
                .is_none()
        );
    }

    #[test]
    fn keyed_and_no_store_responses_never_reach_the_disk() {
        let (_guard, cache) = cache();
        let keyed = Call::get("https://api.example.test/a?site=1&api_key=SECRET-VALUE");
        assert!(
            !cache
                .store_response(&keyed, &json_response(Some(600)), NOW)
                .expect("store")
        );
        let no_store = Call::get("https://api.example.test/b").with_cache(CacheHint {
            no_store: true,
            ..CacheHint::default()
        });
        assert!(
            !cache
                .store_response(&no_store, &json_response(Some(600)), NOW)
                .expect("store")
        );
        assert!(cache.load_response(&keyed, NOW).is_none());
        assert!(
            list_files(cache.dir()).is_empty(),
            "{:?}",
            list_files(cache.dir())
        );
    }

    #[test]
    fn fresh_copies_bypass_the_cache_and_errors_are_not_stored() {
        let (_guard, cache) = cache();
        let plain = Call::get("https://api.example.test/a");
        assert!(
            cache
                .store_response(&plain, &json_response(Some(600)), NOW)
                .expect("store")
        );
        let fresh = plain.clone().with_cache(CacheHint {
            fresh: true,
            ..CacheHint::default()
        });
        assert!(cache.load_response(&fresh, NOW).is_none());
        assert!(cache.load_response(&plain, NOW).is_some());
        let other = Call::get("https://api.example.test/b");
        let failure = Fetched {
            cache_max_age: Some(600),
            ..Fetched::new(500, "oops")
        };
        assert!(!cache.store_response(&other, &failure, NOW).expect("store"));
        assert!(
            !cache
                .store_response(&fresh, &json_response(Some(600)), NOW)
                .expect("store")
        );
    }

    #[test]
    fn files_and_names_carry_no_urls() {
        let (_guard, cache) = cache();
        let call = Call::get("https://api.example.test/secret-looking/path?station=8518750");
        cache
            .store_response(&call, &json_response(Some(600)), NOW)
            .expect("store");
        cache
            .put("spot", &Cache::spot_key("battery"), &"snapshot", NOW)
            .expect("put");
        for file in list_files(cache.dir()) {
            assert!(
                !file.contains("example") && !file.contains("8518750"),
                "{file}"
            );
            let bytes = fs::read(&file).expect("read");
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("example.test"), "{file}");
        }
    }
}
