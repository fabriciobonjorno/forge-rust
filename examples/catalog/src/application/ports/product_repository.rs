//! Persistence port for products.
//!
//! Methods return `impl Future + Send` instead of using `async fn` so the
//! `Send` bound is part of the contract: use cases are generic over the
//! repository (`ProductCatalog<R>`), are monomorphized at the composition
//! root, and their futures can run on a multi-threaded executor. No boxing
//! or `dyn` is needed because each process wires exactly one implementation.

use core::{error::Error, fmt};
use std::future::Future;

use forge::core::{ErrorCategory, ForgeError, Id, Page, PageRequest};

use crate::domain::entities::{NewProduct, Product};

/// Stores and loads products.
///
/// Every implementation obeys the same contract, verified by the shared
/// suite in `tests/support/contract.rs`:
///
/// - [`insert`](Self::insert) stores version 1 and fails with
///   [`RepositoryError::SkuTaken`] when another product has the same SKU;
/// - [`find`](Self::find) returns `None` for an unknown identifier;
/// - [`update`](Self::update) writes the mutable state (name and price) only
///   if the stored version still equals `product.version()`, then advances
///   the version by one; it fails with [`RepositoryError::NotFound`] when the
///   product does not exist and with [`RepositoryError::VersionConflict`]
///   when the stored version differs;
/// - [`list`](Self::list) returns products newest first (descending UUIDv7)
///   strictly after the cursor, built with [`Page::from_overfetched`].
pub trait ProductRepository: Send + Sync {
    /// Stores a new product and returns it as stored.
    fn insert(
        &self,
        product: &NewProduct,
    ) -> impl Future<Output = Result<Product, RepositoryError>> + Send;

    /// Loads a product by identifier.
    fn find(
        &self,
        id: Id<Product>,
    ) -> impl Future<Output = Result<Option<Product>, RepositoryError>> + Send;

    /// Persists the name and price of `product` if nobody changed it since it
    /// was read, and returns the stored state with the next version.
    fn update(
        &self,
        product: &Product,
    ) -> impl Future<Output = Result<Product, RepositoryError>> + Send;

    /// Returns one page of products, newest first.
    fn list(
        &self,
        page: PageRequest<Product>,
    ) -> impl Future<Output = Result<Page<Product, Product>, RepositoryError>> + Send;
}

/// A failed repository operation, expressed in application terms.
///
/// Adapters translate driver errors into these variants (for example a unique
/// violation on the SKU constraint into [`RepositoryError::SkuTaken`]), so
/// use cases never see database types.
#[derive(Debug)]
pub enum RepositoryError {
    /// Another product already uses the SKU.
    SkuTaken,
    /// The product does not exist.
    NotFound,
    /// The product was modified since it was read.
    VersionConflict,
    /// The storage backend failed; the error is already classified (for
    /// example retryable `Infrastructure`) and keeps its diagnostic source.
    Backend(ForgeError),
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SkuTaken => formatter.write_str("SKU is already taken"),
            Self::NotFound => formatter.write_str("product not found"),
            Self::VersionConflict => formatter.write_str("product version conflict"),
            Self::Backend(_) => formatter.write_str("product storage failed"),
        }
    }
}

impl Error for RepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RepositoryError> for ForgeError {
    fn from(error: RepositoryError) -> Self {
        match error {
            RepositoryError::SkuTaken => Self::new(
                ErrorCategory::Conflict,
                "sku_taken",
                "a product with this SKU already exists",
            )
            .with_metadata("field", "sku"),
            RepositoryError::NotFound => Self::new(
                ErrorCategory::NotFound,
                "product_not_found",
                "product not found",
            ),
            RepositoryError::VersionConflict => Self::new(
                ErrorCategory::Conflict,
                "concurrent_modification",
                "the product was modified concurrently; reload it and retry",
            ),
            RepositoryError::Backend(error) => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_errors_map_to_stable_codes() {
        let cases = [
            (
                RepositoryError::SkuTaken,
                ErrorCategory::Conflict,
                "sku_taken",
            ),
            (
                RepositoryError::NotFound,
                ErrorCategory::NotFound,
                "product_not_found",
            ),
            (
                RepositoryError::VersionConflict,
                ErrorCategory::Conflict,
                "concurrent_modification",
            ),
        ];
        for (error, category, code) in cases {
            let forge = ForgeError::from(error);
            assert_eq!(forge.category(), category, "{code}");
            assert_eq!(forge.code(), code);
        }
    }

    #[test]
    fn backend_errors_pass_through_unchanged() {
        let backend = ForgeError::new(
            ErrorCategory::Infrastructure,
            "database_unavailable",
            "retry later",
        )
        .with_retryable(true);

        let error = RepositoryError::Backend(backend);
        assert!(error.source().is_some());
        let forge = ForgeError::from(error);

        assert_eq!(forge.code(), "database_unavailable");
        assert!(forge.is_retryable());
    }
}
