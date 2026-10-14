//! In-memory [`ProductRepository`] for use-case and handler tests.
//!
//! It obeys the same contract as `PgProductRepository`, which
//! `tests/catalog.rs` proves by running the suite in `contract.rs` against
//! it.

use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard},
};

use app::{
    application::ports::{ProductRepository, RepositoryError},
    domain::entities::{NewProduct, Product},
};
use chrono::Utc;
use forge::core::{ErrorCategory, ForgeError, Id, Page, PageRequest, Version};

/// Products kept in a map ordered by UUIDv7 identifier.
#[derive(Debug, Default)]
pub struct InMemoryProductRepository {
    products: Mutex<BTreeMap<Id<Product>, Product>>,
}

impl InMemoryProductRepository {
    fn products(&self) -> Result<MutexGuard<'_, BTreeMap<Id<Product>, Product>>, RepositoryError> {
        self.products.lock().map_err(|_| {
            RepositoryError::Backend(ForgeError::new(
                ErrorCategory::Internal,
                "poisoned",
                "an internal error occurred",
            ))
        })
    }
}

impl ProductRepository for InMemoryProductRepository {
    async fn insert(&self, product: &NewProduct) -> Result<Product, RepositoryError> {
        let mut products = self.products()?;
        if products
            .values()
            .any(|stored| stored.sku() == product.sku())
        {
            return Err(RepositoryError::SkuTaken);
        }
        let now = Utc::now();
        let stored = Product::restore(
            product.id(),
            product.sku().clone(),
            product.name().clone(),
            product.price().clone(),
            Version::INITIAL,
            now,
            now,
        );
        products.insert(stored.id(), stored.clone());
        Ok(stored)
    }

    async fn find(&self, id: Id<Product>) -> Result<Option<Product>, RepositoryError> {
        Ok(self.products()?.get(&id).cloned())
    }

    async fn update(&self, product: &Product) -> Result<Product, RepositoryError> {
        let mut products = self.products()?;
        let stored = products
            .get_mut(&product.id())
            .ok_or(RepositoryError::NotFound)?;
        if stored.version() != product.version() {
            return Err(RepositoryError::VersionConflict);
        }
        let next = stored.version().next().map_err(RepositoryError::Backend)?;
        *stored = Product::restore(
            stored.id(),
            stored.sku().clone(),
            product.name().clone(),
            product.price().clone(),
            next,
            stored.created_at(),
            Utc::now(),
        );
        Ok(stored.clone())
    }

    async fn list(
        &self,
        page: PageRequest<Product>,
    ) -> Result<Page<Product, Product>, RepositoryError> {
        let products = self.products()?;
        let fetch_size = usize::try_from(page.limit.fetch_size()).unwrap_or(usize::MAX);
        let rows = products
            .values()
            .rev()
            .filter(|product| page.after.is_none_or(|cursor| product.id() < cursor))
            .take(fetch_size)
            .cloned()
            .collect();
        Ok(Page::from_overfetched(rows, page.limit, Product::id))
    }
}
