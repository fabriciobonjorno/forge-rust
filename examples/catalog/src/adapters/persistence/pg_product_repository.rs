//! PostgreSQL implementation of [`ProductRepository`].
//!
//! SQL is written out in full and executed with SQLx runtime queries into a
//! [`ProductRow`]; rows are converted into domain values through the same
//! value-object constructors the API uses. Driver errors become
//! [`RepositoryError`]s here, so no database type crosses the port.

use chrono::{DateTime, Utc};
use forge::{
    core::{ErrorCategory, ForgeError, Id, IntoForgeError, Page, PageRequest, Version},
    db::{DbError, PgDatabase},
};
use sqlx::{FromRow, PgPool};

use crate::{
    application::ports::{ProductRepository, RepositoryError},
    domain::{
        entities::{NewProduct, Product},
        value_objects::{Currency, Money, ProductName, Sku},
    },
};

/// Name of the unique constraint on `products.sku` (see the migration).
const SKU_UNIQUE_CONSTRAINT: &str = "products_sku_key";

const INSERT: &str = "\
    INSERT INTO products (id, sku, name, price_cents, currency) \
    VALUES ($1, $2, $3, $4, $5) \
    RETURNING id, sku, name, price_cents, currency, version, created_at, updated_at";

const FIND: &str = "\
    SELECT id, sku, name, price_cents, currency, version, created_at, updated_at \
    FROM products WHERE id = $1";

const UPDATE: &str = "\
    UPDATE products \
    SET name = $3, price_cents = $4, currency = $5, version = version + 1, updated_at = now() \
    WHERE id = $1 AND version = $2 \
    RETURNING id, sku, name, price_cents, currency, version, created_at, updated_at";

const EXISTS: &str = "SELECT EXISTS (SELECT 1 FROM products WHERE id = $1)";

const LIST: &str = "\
    SELECT id, sku, name, price_cents, currency, version, created_at, updated_at \
    FROM products \
    WHERE ($1::uuid IS NULL OR id < $1) \
    ORDER BY id DESC \
    LIMIT $2";

/// Stores products in the `products` table.
#[derive(Clone, Debug)]
pub struct PgProductRepository {
    pool: PgPool,
}

impl PgProductRepository {
    /// Creates a repository over the application's pool.
    #[must_use]
    pub fn new(database: &PgDatabase) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }
}

impl ProductRepository for PgProductRepository {
    async fn insert(&self, product: &NewProduct) -> Result<Product, RepositoryError> {
        let row = sqlx::query_as::<_, ProductRow>(INSERT)
            .bind(product.id().into_uuid())
            .bind(product.sku().as_str())
            .bind(product.name().as_str())
            .bind(product.price().cents())
            .bind(product.price().currency().as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| {
                let error = DbError::from(error);
                if matches!(error, DbError::UniqueViolation { .. })
                    && error.constraint() == Some(SKU_UNIQUE_CONSTRAINT)
                {
                    RepositoryError::SkuTaken
                } else {
                    backend(error)
                }
            })?;
        row.into_product()
    }

    async fn find(&self, id: Id<Product>) -> Result<Option<Product>, RepositoryError> {
        let row = sqlx::query_as::<_, ProductRow>(FIND)
            .bind(id.into_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        row.map(ProductRow::into_product).transpose()
    }

    async fn update(&self, product: &Product) -> Result<Product, RepositoryError> {
        let row = sqlx::query_as::<_, ProductRow>(UPDATE)
            .bind(product.id().into_uuid())
            .bind(product.version().get())
            .bind(product.name().as_str())
            .bind(product.price().cents())
            .bind(product.price().currency().as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        if let Some(row) = row {
            return row.into_product();
        }
        // Nothing matched `id AND version`: tell a missing product apart
        // from a stale version.
        let exists = sqlx::query_scalar::<_, bool>(EXISTS)
            .bind(product.id().into_uuid())
            .fetch_one(&self.pool)
            .await
            .map_err(backend)?;
        Err(if exists {
            RepositoryError::VersionConflict
        } else {
            RepositoryError::NotFound
        })
    }

    async fn list(
        &self,
        page: PageRequest<Product>,
    ) -> Result<Page<Product, Product>, RepositoryError> {
        let rows = sqlx::query_as::<_, ProductRow>(LIST)
            .bind(page.after.map(Id::into_uuid))
            .bind(page.limit.fetch_size())
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;
        let products = rows
            .into_iter()
            .map(ProductRow::into_product)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Page::from_overfetched(products, page.limit, Product::id))
    }
}

/// One row of `products`, as selected by every query above.
#[derive(FromRow)]
struct ProductRow {
    id: uuid::Uuid,
    sku: String,
    name: String,
    price_cents: i64,
    currency: String,
    version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ProductRow {
    /// Revalidates stored values. The table's CHECK constraints mirror the
    /// domain rules, so a failure means the data was changed outside the
    /// application and is reported as an internal error.
    fn into_product(self) -> Result<Product, RepositoryError> {
        let id = Id::from_uuid(self.id).map_err(invalid_stored)?;
        let version = Version::new(self.version).map_err(invalid_stored)?;
        let currency = Currency::parse(&self.currency).map_err(invalid_stored)?;
        Ok(Product::restore(
            id,
            Sku::parse(&self.sku).map_err(invalid_stored)?,
            ProductName::parse(&self.name).map_err(invalid_stored)?,
            Money::new(self.price_cents, currency).map_err(invalid_stored)?,
            version,
            self.created_at,
            self.updated_at,
        ))
    }
}

fn invalid_stored(error: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::Backend(
        ForgeError::new(
            ErrorCategory::Internal,
            "invalid_stored_product",
            "an internal error occurred",
        )
        .with_source(error),
    )
}

/// Wraps a classified database failure (retryability, safe message and
/// diagnostic source are preserved).
fn backend(error: impl Into<DbError>) -> RepositoryError {
    RepositoryError::Backend(error.into().into_forge_error())
}
