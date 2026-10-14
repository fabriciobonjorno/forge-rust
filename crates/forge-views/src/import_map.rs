//! Import maps: bare module specifiers mapped to digested asset URLs, the way
//! Rails 8 loads Turbo and Stimulus without a bundler.
//!
//! ```
//! # use forge_views::{AssetEntry, AssetSet, ImportMap};
//! # static TABLE: &[AssetEntry] = &[AssetEntry::new(
//! #     "turbo.js",
//! #     "turbo-c3449cd99d19554d.js",
//! #     "c3449cd99d19554da590a1d8a779f7c583577738f0bc8af1956d5f74bd51194a",
//! #     b"turbo",
//! # )];
//! let assets = AssetSet::new(TABLE)?;
//! let map = ImportMap::builder().pin("@hotwired/turbo", "turbo.js").build(&assets)?;
//! assert!(map.script_tag().starts_with("<script type=\"importmap\">"));
//! // Allow exactly this inline script in the Content-Security-Policy:
//! let policy = forge_views::ContentSecurityPolicy::strict().with_import_map(&map);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{AssetSet, encoding::base64};

/// Maximum number of pins in one import map.
pub const MAX_PINS: usize = 256;
/// Maximum length of a module specifier, in bytes.
pub const MAX_SPECIFIER_BYTES: usize = 256;

/// Invalid import map definition.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ImportMapError {
    /// The specifier is empty, too long, or contains control characters.
    #[error("invalid module specifier {0:?}")]
    InvalidSpecifier(String),
    /// The same specifier was pinned twice.
    #[error("module specifier {0:?} is pinned more than once")]
    DuplicateSpecifier(String),
    /// The logical asset is not in the manifest.
    #[error("module {specifier:?} is pinned to unknown asset {logical:?}")]
    UnknownAsset {
        /// The specifier being pinned.
        specifier: String,
        /// The logical asset name that was not found.
        logical: String,
    },
    /// More than [`MAX_PINS`] pins.
    #[error("import map has more than {MAX_PINS} pins")]
    TooManyPins,
    /// The map could not be serialized.
    #[error("import map could not be encoded: {0}")]
    Encode(String),
}

/// Collects pins before they are resolved against an [`AssetSet`].
#[derive(Clone, Debug, Default)]
pub struct ImportMapBuilder {
    pins: Vec<(String, String)>,
}

impl ImportMapBuilder {
    /// Pins `specifier` (for example `@hotwired/turbo`) to the logical asset
    /// `logical` (for example `turbo.js`). Validation happens in
    /// [`build`](Self::build).
    #[must_use]
    pub fn pin(mut self, specifier: &str, logical: &str) -> Self {
        self.pins.push((specifier.to_owned(), logical.to_owned()));
        self
    }

    /// Resolves every pin to its digested URL and renders the map.
    pub fn build(self, assets: &AssetSet) -> Result<ImportMap, ImportMapError> {
        if self.pins.len() > MAX_PINS {
            return Err(ImportMapError::TooManyPins);
        }
        let mut imports = BTreeMap::new();
        for (specifier, logical) in self.pins {
            if specifier.is_empty()
                || specifier.len() > MAX_SPECIFIER_BYTES
                || specifier.chars().any(char::is_control)
            {
                return Err(ImportMapError::InvalidSpecifier(specifier));
            }
            let url = assets
                .path(&logical)
                .ok_or_else(|| ImportMapError::UnknownAsset {
                    specifier: specifier.clone(),
                    logical: logical.clone(),
                })?;
            if imports.insert(specifier.clone(), url.to_owned()).is_some() {
                return Err(ImportMapError::DuplicateSpecifier(specifier));
            }
        }
        let document = serde_json::json!({ "imports": imports });
        let json = serde_json::to_string(&document)
            .map_err(|failure| ImportMapError::Encode(failure.to_string()))?;
        Ok(ImportMap::from_json(&json))
    }
}

/// A rendered import map and the hash that lets a strict
/// Content-Security-Policy allow its inline script without `unsafe-inline`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportMap {
    body: String,
    tag: String,
    sha256: String,
}

impl ImportMap {
    /// Starts an empty import map.
    #[must_use]
    pub fn builder() -> ImportMapBuilder {
        ImportMapBuilder::default()
    }

    fn from_json(json: &str) -> Self {
        let body = escape_for_script(json);
        let sha256 = base64(&Sha256::digest(body.as_bytes()));
        let tag = format!("<script type=\"importmap\">{body}</script>");
        Self { body, tag, sha256 }
    }

    /// The JSON text placed inside the script element, exactly as hashed.
    #[must_use]
    pub fn json(&self) -> &str {
        &self.body
    }

    /// The complete `<script type="importmap">` element. It carries no
    /// `nonce`; it is allowed by hash. Insert it in the layout `<head>`
    /// before any module script, and do not reformat it: the hash covers the
    /// exact bytes between the tags.
    #[must_use]
    pub fn script_tag(&self) -> &str {
        &self.tag
    }

    /// Base64 SHA-256 of the exact script body.
    #[must_use]
    pub fn sha256_base64(&self) -> &str {
        &self.sha256
    }

    /// The CSP source expression, `'sha256-<base64>'`, for `script-src`.
    #[must_use]
    pub fn csp_source(&self) -> String {
        format!("'sha256-{}'", self.sha256)
    }
}

/// Makes JSON safe inside a `<script>` element. `<`, `>` and `&` could end
/// the element or open a comment in the HTML parser, and U+2028/U+2029 are
/// line terminators in some script contexts. All of them can only occur inside
/// JSON strings, where `\uXXXX` escapes are equivalent.
fn escape_for_script(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssetEntry;

    static TABLE: &[AssetEntry] = &[
        AssetEntry::new(
            "turbo.js",
            "turbo-c3449cd99d19554d.js",
            "c3449cd99d19554da590a1d8a779f7c583577738f0bc8af1956d5f74bd51194a",
            b"turbo",
        ),
        AssetEntry::new(
            "controllers/app.js",
            "controllers/app-a2098bd92b10bf8b.js",
            "a2098bd92b10bf8b816d24b7556b1ce8c49a879d130489065ef1051c17e042f6",
            b"export const answer = 42;\n",
        ),
    ];

    fn assets() -> AssetSet {
        AssetSet::new(TABLE).expect("valid table")
    }

    fn body_of(tag: &str) -> &str {
        tag.strip_prefix("<script type=\"importmap\">")
            .and_then(|rest| rest.strip_suffix("</script>"))
            .expect("script element")
    }

    #[test]
    fn renders_sorted_json_with_digested_urls() {
        let map = ImportMap::builder()
            .pin("@hotwired/turbo", "turbo.js")
            .pin("app", "controllers/app.js")
            .build(&assets())
            .expect("valid map");

        assert_eq!(
            map.json(),
            "{\"imports\":{\"@hotwired/turbo\":\"/assets/turbo-c3449cd99d19554d.js\",\
             \"app\":\"/assets/controllers/app-a2098bd92b10bf8b.js\"}}"
        );
        let parsed: serde_json::Value = serde_json::from_str(map.json()).expect("valid JSON");
        assert_eq!(
            parsed["imports"]["app"],
            "/assets/controllers/app-a2098bd92b10bf8b.js"
        );
    }

    #[test]
    fn script_tag_has_no_nonce_and_carries_exactly_the_hashed_body() {
        let map = ImportMap::builder()
            .pin("@hotwired/turbo", "turbo.js")
            .build(&assets())
            .expect("valid map");

        assert!(map.script_tag().starts_with("<script type=\"importmap\">"));
        assert!(!map.script_tag().contains("nonce"));
        assert_eq!(body_of(map.script_tag()), map.json());
    }

    #[test]
    fn hash_is_the_base64_sha256_of_the_script_body() {
        let map = ImportMap::builder()
            .pin("@hotwired/turbo", "turbo.js")
            .build(&assets())
            .expect("valid map");

        // Independently computed with:
        //   printf '%s' '{"imports":{"@hotwired/turbo":"/assets/turbo-c3449cd99d19554d.js"}}' \
        //     | openssl dgst -sha256 -binary | base64
        assert_eq!(
            map.sha256_base64(),
            "mH1ogHvYcN6oG992YDBq8JYlIqdRLaqN80Fj3frcADY="
        );
        let recomputed = base64(&Sha256::digest(body_of(map.script_tag()).as_bytes()));
        assert_eq!(map.sha256_base64(), recomputed);
        assert_eq!(
            map.csp_source(),
            format!("'sha256-{}'", map.sha256_base64())
        );
    }

    #[test]
    fn script_breaking_characters_are_escaped_and_still_hashed_as_rendered() {
        let map = ImportMap::builder()
            .pin("</script><script>alert(1)", "turbo.js")
            .pin("a&b\u{2028}", "controllers/app.js")
            .build(&assets())
            .expect("valid map");

        let tag = map.script_tag();
        assert_eq!(tag.matches("</script>").count(), 1);
        assert!(!body_of(tag).contains(['<', '>', '&', '\u{2028}']));
        let parsed: serde_json::Value = serde_json::from_str(map.json()).expect("valid JSON");
        assert!(parsed["imports"]["</script><script>alert(1)"].is_string());
        assert_eq!(
            map.sha256_base64(),
            base64(&Sha256::digest(body_of(tag).as_bytes()))
        );
    }

    #[test]
    fn unknown_assets_duplicates_and_bad_specifiers_are_rejected() {
        let assets = assets();

        assert_eq!(
            ImportMap::builder()
                .pin("x", "missing.js")
                .build(&assets)
                .expect_err("unknown"),
            ImportMapError::UnknownAsset {
                specifier: "x".to_owned(),
                logical: "missing.js".to_owned(),
            }
        );
        assert_eq!(
            ImportMap::builder()
                .pin("x", "turbo.js")
                .pin("x", "controllers/app.js")
                .build(&assets)
                .expect_err("duplicate"),
            ImportMapError::DuplicateSpecifier("x".to_owned())
        );
        for specifier in ["", "a\nb", &"x".repeat(MAX_SPECIFIER_BYTES + 1)] {
            assert!(matches!(
                ImportMap::builder()
                    .pin(specifier, "turbo.js")
                    .build(&assets)
                    .expect_err("invalid specifier"),
                ImportMapError::InvalidSpecifier(_)
            ));
        }
    }

    #[test]
    fn pin_count_is_bounded() {
        let mut builder = ImportMap::builder();
        for index in 0..=MAX_PINS {
            builder = builder.pin(&format!("m{index}"), "turbo.js");
        }

        assert_eq!(
            builder.build(&assets()).expect_err("too many"),
            ImportMapError::TooManyPins
        );
    }
}
