//! Helpers for tests that read recorded fixtures from `tests/fixtures/raw/<source>/<name>`.

use std::path::PathBuf;

/// Path of a recorded fixture, relative to the workspace root.
pub(crate) fn fixture_path(source: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/raw")
        .join(source)
        .join(name)
}

/// The bytes of a recorded fixture. Panics with the missing path if it cannot be read.
pub(crate) fn fixture_bytes(source: &str, name: &str) -> Vec<u8> {
    let path = fixture_path(source, name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

/// A recorded fixture as text. Panics if it is missing or not UTF-8.
pub(crate) fn fixture_str(source: &str, name: &str) -> String {
    let path = fixture_path(source, name);
    String::from_utf8(fixture_bytes(source, name))
        .unwrap_or_else(|e| panic!("fixture {} is not UTF-8: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_recorded_coops_fixture() {
        let text = fixture_str("coops", "coops-8518750-hilo-2026-11-08.json");
        assert!(text.trim_start().starts_with('{'));
        assert!(text.contains("\"predictions\""));
        assert_eq!(text.matches("\"t\":").count(), 27);
        assert_eq!(
            fixture_bytes("coops", "coops-8518750-hilo-2026-11-08.json"),
            text.as_bytes()
        );
    }

    #[test]
    #[should_panic(expected = "no-such-file.json")]
    fn a_missing_fixture_names_its_path() {
        fixture_bytes("coops", "no-such-file.json");
    }
}
