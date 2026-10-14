//! Product catalog use cases.
//!
//! Each method is one use case: it receives a validated command or query,
//! loads what it needs through the [`ProductRepository`] port, applies the
//! domain behavior and stores the result. Failures are returned as
//! [`ForgeError`]s with stable codes; adapters decide how to render them.

use forge::core::{ForgeError, Id, Page, Version};

use crate::{
    application::{
        commands::{ChangePrice, CreateProduct, RenameProduct},
        ports::{ProductRepository, RepositoryError},
        queries::{GetProduct, ListProducts},
    },
    domain::entities::{NewProduct, Product},
};

/// Product catalog use cases over a repository implementation `R`.
///
/// Built once at startup by the composition root and shared between
/// requests; it holds no per-request state.
#[derive(Debug)]
pub struct ProductCatalog<R> {
    repository: R,
}

impl<R: ProductRepository> ProductCatalog<R> {
    /// Creates the use cases over `repository`.
    #[must_use]
    pub const fn new(repository: R) -> Self {
        Self { repository }
    }

    /// Adds a product. Fails with `sku_taken` when the SKU is in use.
    pub async fn create(&self, command: CreateProduct) -> Result<Product, ForgeError> {
        let product = NewProduct::new(command.sku, command.name, command.price);
        Ok(self.repository.insert(&product).await?)
    }

    /// Renames a product. Fails with `product_not_found` or
    /// `concurrent_modification`.
    pub async fn rename(&self, command: RenameProduct) -> Result<Product, ForgeError> {
        let mut product = self
            .load_for_update(command.id, command.expected_version)
            .await?;
        product.rename(command.name);
        Ok(self.repository.update(&product).await?)
    }

    /// Changes a product's price. Fails with `currency_mismatch`,
    /// `product_not_found` or `concurrent_modification`.
    pub async fn change_price(&self, command: ChangePrice) -> Result<Product, ForgeError> {
        let mut product = self
            .load_for_update(command.id, command.expected_version)
            .await?;
        product.change_price(command.price)?;
        Ok(self.repository.update(&product).await?)
    }

    /// Loads one product. Fails with `product_not_found`.
    pub async fn get(&self, query: GetProduct) -> Result<Product, ForgeError> {
        self.repository
            .find(query.id)
            .await?
            .ok_or_else(|| RepositoryError::NotFound.into())
    }

    /// Lists one page of products, newest first.
    pub async fn list(&self, query: ListProducts) -> Result<Page<Product, Product>, ForgeError> {
        Ok(self.repository.list(query.page).await?)
    }

    /// Loads the product a client wants to change and rejects the change
    /// early when the client's copy is stale. The repository's guarded
    /// update still catches writes that race between this read and the
    /// update.
    async fn load_for_update(
        &self,
        id: Id<Product>,
        expected_version: Version,
    ) -> Result<Product, ForgeError> {
        let product = self
            .repository
            .find(id)
            .await?
            .ok_or(RepositoryError::NotFound)?;
        if product.version() != expected_version {
            return Err(RepositoryError::VersionConflict.into());
        }
        Ok(product)
    }
}
