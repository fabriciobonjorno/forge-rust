//! Database implementation and connection pooling.
//!
//! This module provides the concrete database types for each backend
//! (PostgreSQL, MySQL, SQLite) that implement the [`Backend`] trait.

use crate::{Backend, DbError, Transaction, TransactionManager};
use forge_config::{DatabaseConfig, DatabaseScheme};
use sqlx::{
    Pool,
    migrate::{Migrate, Migrator},
    pool::PoolOptions,
};

/// Type aliases for each backend's database pool.
#[cfg(feature = "postgres")]
pub type PgDatabase = Database<sqlx::Postgres>;

#[cfg(feature = "mysql")]
pub type MySqlDatabase = Database<sqlx::MySql>;

#[cfg(feature = "sqlite")]
pub type SqliteDatabase = Database<sqlx::Sqlite>;

/// A database connection pool for a specific backend.
///
/// This is the concrete implementation that applications and adapters use.
/// It wraps SQLx's `Pool<B>` and provides Forge-specific methods like
/// migration, health checks, and readiness.
///
/// `Database` is intentionally not `Clone`; share it behind an [`std::sync::Arc`]
/// (as [`DatabaseHandle`](crate::DatabaseHandle) does) instead of cloning the pool
/// description.
#[derive(Debug)]
pub struct Database<B: Backend> {
    pool: Pool<B>,
    scheme: DatabaseScheme,
    config: DatabaseConfig,
}

impl<B: Backend> Database<B> {
    /// Creates a new database pool from configuration.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, DbError> {
        let url = config.url.as_ref().ok_or_else(|| {
            DbError::Configuration(sqlx::Error::Configuration(
                "database URL is required".into(),
            ))
        })?;

        let pool = PoolOptions::<B>::new()
            .max_connections(config.max_connections.unwrap_or(10))
            .acquire_timeout(
                config
                    .acquire_timeout
                    .unwrap_or(std::time::Duration::from_secs(30)),
            )
            .connect(url.expose())
            .await
            .map_err(DbError::from)?;

        let scheme = url.scheme().unwrap_or(DatabaseScheme::Postgres);

        Ok(Self {
            pool,
            scheme,
            config: config.clone(),
        })
    }

    /// Creates a lazy-connecting database pool (connects on first use).
    pub fn connect_lazy(config: &DatabaseConfig) -> Result<Self, DbError> {
        let url = config.url.as_ref().ok_or_else(|| {
            DbError::Configuration(sqlx::Error::Configuration(
                "database URL is required".into(),
            ))
        })?;

        let pool = PoolOptions::<B>::new()
            .max_connections(config.max_connections.unwrap_or(10))
            .acquire_timeout(
                config
                    .acquire_timeout
                    .unwrap_or(std::time::Duration::from_secs(30)),
            )
            .connect_lazy(url.expose())
            .map_err(DbError::from)?;

        let scheme = url.scheme().unwrap_or(DatabaseScheme::Postgres);

        Ok(Self {
            pool,
            scheme,
            config: config.clone(),
        })
    }

    /// Returns the underlying SQLx pool.
    pub fn pool(&self) -> &Pool<B> {
        &self.pool
    }

    /// Returns the database scheme (postgres, mysql, sqlite).
    pub fn scheme(&self) -> DatabaseScheme {
        self.scheme
    }

    /// Returns the configuration.
    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    /// Pings the database to check connectivity.
    pub async fn ping(&self) -> Result<(), DbError> {
        self.pool().acquire().await.map_err(DbError::from)?;
        Ok(())
    }

    /// Runs all pending migrations.
    pub async fn migrate(&self, migrator: &Migrator) -> Result<Vec<i64>, DbError>
    where
        <B as sqlx::Database>::Connection: Migrate,
    {
        crate::migrate::apply_pending(&self.pool, migrator).await
    }

    /// Rolls back the most recent migration.
    pub async fn rollback(&self, migrator: &Migrator) -> Result<Option<i64>, DbError>
    where
        <B as sqlx::Database>::Connection: Migrate,
    {
        crate::migrate::revert_latest(&self.pool, migrator).await
    }

    /// Closes the pool, waiting for checked-out connections to return.
    pub async fn close(&self) {
        self.pool().close().await;
    }
}

impl<B: Backend> TransactionManager for Database<B> {
    type Transaction<'a>
        = TransactionWrapper<'a, B>
    where
        Self: 'a;

    async fn begin(&self) -> Result<Self::Transaction<'_>, DbError> {
        let tx = self.pool().begin().await.map_err(DbError::from)?;
        Ok(TransactionWrapper(tx))
    }
}

/// Wrapper around SQLx transaction that implements our Transaction trait.
pub struct TransactionWrapper<'a, B: sqlx::Database>(sqlx::Transaction<'a, B>);

impl<B: sqlx::Database> Transaction for TransactionWrapper<'_, B> {
    async fn commit(self) -> Result<(), DbError> {
        self.0.commit().await.map_err(DbError::from)
    }

    async fn rollback(self) -> Result<(), DbError> {
        self.0.rollback().await.map_err(DbError::from)
    }
}
