//! Server-rendered views for Forge applications (ADR 0022).
//!
//! The crate gives HTTP adapters the primitives behind Rails-style HTML
//! applications, with secure defaults:
//!
//! - [`html`] / [`html_with_status`]: render an Askama view as an HTML
//!   response (auto-escaped; failures become an opaque `500`).
//! - [`see_other`], [`turbo_stream`], [`TurboStreamAction`], [`DomId`],
//!   [`dom_id`]: Turbo-compatible redirects and stream responses.
//! - [`AssetSet`] with [`build::embed_assets`]: content-digested assets
//!   embedded in the binary and served from exact manifest entries only.
//! - [`ImportMap`]: bare module specifiers mapped to digested URLs, with the
//!   hash a strict CSP needs.
//! - [`ContentSecurityPolicy`]: a strict default policy without
//!   `unsafe-inline`.
//! - [`csrf_meta`], [`csrf_field`], [`method_override`]: form helpers.
//!
//! Views belong in the HTTP adapter ring; domain and application code never
//! see this crate. See `docs/views.md` for the developer guide.
//!
//! # Features
//!
//! - `runtime` (default): everything except the build helper.
//! - `build` (default): [`build::embed_assets`]. A `build.rs` should depend
//!   on this crate with `default-features = false, features = ["build"]` to
//!   avoid the HTTP and template dependencies.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

#[cfg(feature = "build")]
pub mod build;
#[cfg(any(feature = "build", feature = "runtime"))]
mod common;

#[cfg(feature = "runtime")]
mod assets;
#[cfg(feature = "runtime")]
mod csp;
#[cfg(feature = "runtime")]
mod encoding;
#[cfg(feature = "runtime")]
mod forms;
#[cfg(feature = "runtime")]
mod import_map;
#[cfg(feature = "runtime")]
mod render;
#[cfg(all(test, feature = "runtime"))]
mod test_support;
#[cfg(feature = "runtime")]
mod turbo;
#[cfg(feature = "runtime")]
pub mod vendor;

#[cfg(feature = "runtime")]
pub use assets::{AssetEntry, AssetError, AssetSet, DEFAULT_PREFIX};
#[cfg(feature = "runtime")]
pub use csp::{ContentSecurityPolicy, CspError};
#[cfg(feature = "runtime")]
pub use forms::{
    CSRF_FIELD_NAME, CSRF_HEADER_NAME, CSRF_META_NAME, CsrfVerified, METHOD_FIELD_NAME, csrf_field,
    csrf_meta, method_override,
};
#[cfg(feature = "runtime")]
pub use import_map::{ImportMap, ImportMapBuilder, ImportMapError};
#[cfg(feature = "runtime")]
pub use render::{html, html_with_status};
#[cfg(feature = "runtime")]
pub use turbo::{
    DomId, DomIdError, RedirectError, StreamAction, TurboStreamAction, TurboStreamError, dom_id,
    see_other, see_other_to_origin, turbo_stream,
};
