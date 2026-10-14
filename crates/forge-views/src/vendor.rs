//! Vendored, integrity-pinned JavaScript served through the import map.
//!
//! Files live unmodified under `vendor/`; their provenance (npm tarball,
//! integrity, license) is recorded in each directory's `README.md`.

/// Stimulus 3.2.2 ES module build (`@hotwired/stimulus`, MIT).
pub const STIMULUS_JS: &str = include_str!("../vendor/stimulus/stimulus.js");

/// Version of [`STIMULUS_JS`].
pub const STIMULUS_VERSION: &str = "3.2.2";

/// Lower-case hex SHA-256 of [`STIMULUS_JS`], pinned when the file was vendored.
pub const STIMULUS_SHA256: &str =
    "23deecdac6f36c08e8f39fbed6b27f60c850cacad3a34eb71194462b7b1647f3";

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn vendored_stimulus_matches_its_pinned_digest() {
        let digest = Sha256::digest(STIMULUS_JS.as_bytes());
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex, STIMULUS_SHA256,
            "vendor/stimulus/stimulus.js was modified"
        );
        assert!(
            STIMULUS_JS.contains("export {"),
            "expected an ES module build"
        );
    }
}
