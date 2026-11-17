//! Vendored dependencies for the build hosts.
//!
//! The hosts have no access to the git server the workspace's git dependency
//! lives on, so `cargo xtask remote sync` runs `cargo vendor` here, ships the
//! result with the working tree and writes a Cargo config on the host that
//! replaces crates.io and the git source with that directory. The vendored
//! sources are rebuilt only when `Cargo.lock` changes: a stamp file next to them
//! holds the lock's hash.
//!
//! Everything lives under `target/vendor-sync/`, which is ignored by version
//! control and never shipped by the tree copy.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the vendored sources and their bookkeeping live, relative to the workspace.
pub(crate) const SYNC_DIR: &str = "target/vendor-sync";
/// The vendored crates, under [`SYNC_DIR`]; on the host it is `<dir>/vendor`.
pub(crate) const VENDOR_SUBDIR: &str = "vendor";
/// The Cargo config for the host, under [`SYNC_DIR`].
pub(crate) const HOST_CONFIG: &str = "host-config.toml";
/// The hash of the `Cargo.lock` the vendored sources were made from.
const STAMP: &str = "lock.stamp";

/// The `xtask` alias, as in the repository's own `.cargo/config.toml`.
const ALIAS: &str = "run --quiet --package xtask --";

/// The directory with the vendored crates, to copy to `<dir>/vendor` on the host.
pub(crate) fn vendor_dir(root: &Path) -> PathBuf {
    root.join(SYNC_DIR).join(VENDOR_SUBDIR)
}

/// The host's Cargo config, to copy to `<dir>/.cargo/config.toml` on the host.
pub(crate) fn host_config_path(root: &Path) -> PathBuf {
    root.join(SYNC_DIR).join(HOST_CONFIG)
}

/// A 64-bit FNV-1a hash of `bytes`, as hex. Only a change detector, not a checksum
/// anything relies on.
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}-{}", bytes.len())
}

/// Bumped whenever [`host_config`] writes something new, so every checkout makes it again.
const CONFIG_REVISION: u32 = 2;

/// What the stamp file holds for `lock`: its fingerprint and the host config's revision.
fn stamp_for(lock: &[u8]) -> String {
    format!("{}-r{CONFIG_REVISION}", fingerprint(lock))
}

/// Whether the vendored sources (or the host config) must be made again.
fn is_stale(lock: &[u8], stamp: Option<&str>, vendored: bool) -> bool {
    !vendored || stamp.map(str::trim) != Some(stamp_for(lock).as_str())
}

/// The Cargo config for the host: the `xtask` alias, no network, no incremental
/// builds (each lane builds once per gate, and the caches filled the host's disk), and
/// the source replacement `cargo vendor` printed, pointing at `vendor` beside `.cargo`.
fn host_config(printed: &str) -> Result<String, String> {
    let mut out = format!(
        "[alias]\nxtask = \"{ALIAS}\"\n\n[net]\noffline = true\n\n[build]\nincremental = false\n\n"
    );
    let mut replaced = false;
    for line in printed.lines() {
        if line.trim_start().starts_with("directory") {
            out.push_str("directory = \"");
            out.push_str(VENDOR_SUBDIR);
            out.push_str("\"\n");
            replaced = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if replaced && printed.contains("replace-with") {
        Ok(out)
    } else {
        Err("`cargo vendor` printed no source replacement".into())
    }
}

/// Makes sure `target/vendor-sync/` holds sources for the current `Cargo.lock`,
/// running `cargo vendor --locked` when it does not. Returns whether it ran.
pub(crate) fn ensure(root: &Path) -> Result<bool, String> {
    let sync = root.join(SYNC_DIR);
    let lock = std::fs::read(root.join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    let stamp = std::fs::read_to_string(sync.join(STAMP)).ok();
    let vendored = vendor_dir(root).is_dir() && host_config_path(root).is_file();
    if !is_stale(&lock, stamp.as_deref(), vendored) {
        println!("vendored sources are current for this Cargo.lock");
        return Ok(false);
    }
    // A half-finished earlier run must not look current.
    let _ = std::fs::remove_file(sync.join(STAMP));
    std::fs::create_dir_all(&sync).map_err(|e| format!("{}: {e}", sync.display()))?;
    let target = format!("{SYNC_DIR}/{VENDOR_SUBDIR}");
    println!("$ cargo vendor --locked {target}");
    let out = Command::new("cargo")
        .args(["vendor", "--locked", &target])
        .current_dir(root)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("failed to start `cargo vendor`: {e}"))?;
    if !out.status.success() {
        return Err(format!("`cargo vendor` failed with {}", out.status));
    }
    let config = host_config(&String::from_utf8_lossy(&out.stdout))?;
    std::fs::write(host_config_path(root), config)
        .map_err(|e| format!("{}: {e}", host_config_path(root).display()))?;
    std::fs::write(sync.join(STAMP), stamp_for(&lock))
        .map_err(|e| format!("{}: {e}", sync.join(STAMP).display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRINTED: &str = r#"[source.crates-io]
replace-with = "vendored-sources"

[source."git+ssh://git@example.invalid/w.git?rev=abc"]
git = "ssh://git@example.invalid/w.git"
rev = "abc"
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "target/vendor-sync/vendor"
"#;

    #[test]
    fn the_stamp_tracks_the_lock() {
        let lock = b"version = 4\n";
        let stamp = stamp_for(lock);
        // A stamp from before the host config's last revision is stale.
        assert!(is_stale(lock, Some(&fingerprint(lock)), true));
        assert!(!is_stale(lock, Some(&stamp), true));
        assert!(!is_stale(lock, Some(&format!("{stamp}\n")), true));
        assert!(is_stale(b"version = 4\n# changed\n", Some(&stamp), true));
        assert!(is_stale(lock, None, true), "no stamp");
        assert!(is_stale(lock, Some(&stamp), false), "sources deleted");
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
    }

    #[test]
    fn the_host_config_points_at_the_shipped_directory() {
        let text = host_config(PRINTED).unwrap();
        let table: toml::Table = text.parse().unwrap();
        assert_eq!(table["net"]["offline"].as_bool(), Some(true));
        assert_eq!(table["build"]["incremental"].as_bool(), Some(false));
        assert_eq!(
            table["source"]["vendored-sources"]["directory"].as_str(),
            Some("vendor")
        );
        assert_eq!(
            table["source"]["crates-io"]["replace-with"].as_str(),
            Some("vendored-sources")
        );
        assert!(
            table["source"]
                .as_table()
                .unwrap()
                .keys()
                .any(|k| k.starts_with("git+ssh://")),
            "the git source is replaced too"
        );
        assert!(host_config("nothing here").is_err());
    }

    #[test]
    fn the_alias_matches_the_repository_config() {
        let text =
            std::fs::read_to_string(crate::workspace_root().join(".cargo/config.toml")).unwrap();
        let table: toml::Table = text.parse().unwrap();
        assert_eq!(table["alias"]["xtask"].as_str(), Some(ALIAS));
    }
}
