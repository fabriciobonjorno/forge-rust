//! Stable, runtime-agnostic contracts shared by Forge components.
//!
//! This crate owns only foundational concepts: version-7 typed identifiers,
//! safe application errors, cancellation, and component lifecycle contracts.
//! Transport and infrastructure concerns belong in adapter crates.
//!
//! # Features
//!
//! - `sqlx`: driver-agnostic SQLx `Type`, `Encode` and `Decode` for [`Id`] and
//!   [`Version`], delegating to `Uuid` and `i64` so they work on every backend
//!   whose driver is enabled by the application, with the same validation on
//!   decode.
//! - `postgres`: also implements `PgHasArrayType` (`uuid[]`, `bigint[]`).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod id;
mod lifecycle;
#[cfg(feature = "sqlx")]
mod sqlx_support;
mod version;

pub use error::{ErrorCategory, ErrorMetadata, ErrorReport, ForgeError, Result};
pub use id::{Id, InvalidId};
pub use lifecycle::{
    BoxLifecycleFuture, Cancellation, Lifecycle, LifecycleContext, LifecycleState,
};
pub use version::Version;
