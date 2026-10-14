//! Stable, runtime-agnostic contracts shared by Forge components.
//!
//! This crate owns only foundational concepts: version-7 typed identifiers,
//! safe application errors, cancellation, and component lifecycle contracts.
//! Transport and infrastructure concerns belong in adapter crates.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod id;
mod lifecycle;
mod pagination;
mod version;

pub use error::{ErrorCategory, ErrorMetadata, ErrorReport, ForgeError, IntoForgeError, Result};
pub use id::{Id, InvalidId};
pub use lifecycle::{
    BoxLifecycleFuture, Cancellation, Lifecycle, LifecycleContext, LifecycleState,
};
pub use pagination::{Page, PageLimit, PageRequest};
pub use version::Version;
