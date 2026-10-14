//! Database contracts owned by Forge.
//!
//! This crate deliberately contains no SQLx or PostgreSQL types. Application
//! ports can depend on these transaction/error contracts while concrete SQLx
//! adapters remain in infrastructure.

use forge_tenancy::TenantContext;
use thiserror::Error;

// Public modules
pub mod backend;
pub mod database;
pub mod error;
pub mod migrate;

// Re-export SQLx's Migrator for test support
pub use sqlx::migrate::Migrator;

// Re-export sqlx for migration macro access
pub use sqlx;

// Re-export key types at crate root for compatibility
pub use backend::{Backend, RowsAffected};
#[cfg(feature = "mysql")]
pub use database::MySqlDatabase;
#[cfg(feature = "sqlite")]
pub use database::SqliteDatabase;
#[cfg(feature = "postgres")]
pub use database::{Database, PgDatabase, TransactionWrapper};
#[cfg(not(feature = "postgres"))]
pub use database::{Database, TransactionWrapper};
pub use error::{DbError, Kind as DatabaseErrorKind};

/// Testing support module (only available with `testing` feature).
#[cfg(feature = "testing")]
pub mod testing {
    use crate::{DbError, Migrator};

    /// A provisioned test database.
    #[derive(Debug)]
    pub struct Provisioned {
        pub name: String,
        pub url: String,
    }

    impl Provisioned {
        /// Creates a new provisioned test database.
        pub fn new(name: String, url: String) -> Self {
            Self { name, url }
        }
    }

    /// Isolated PostgreSQL database for an integration test.
    #[cfg(feature = "postgres")]
    pub struct PgTestDatabase {
        database: crate::PgDatabase,
        admin: sqlx::PgPool,
        name: String,
    }

    #[cfg(feature = "postgres")]
    impl std::fmt::Debug for PgTestDatabase {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("PgTestDatabase")
                .field("name", &self.name)
                .finish_non_exhaustive()
        }
    }

    #[cfg(feature = "postgres")]
    impl PgTestDatabase {
        /// Creates a uniquely named database using `FORGE_TEST_DATABASE_URL`
        /// as the administrative connection. The target URL must connect as a
        /// role allowed to create and drop databases.
        pub async fn create(migrator: &Migrator) -> Result<Self, DbError> {
            use sqlx::{
                AssertSqlSafe, ConnectOptions,
                postgres::{PgConnectOptions, PgPoolOptions},
            };
            use std::str::FromStr;

            let target = std::env::var("FORGE_TEST_DATABASE_URL").map_err(|_| {
                DbError::Configuration(sqlx::Error::Configuration(
                    "FORGE_TEST_DATABASE_URL is required for PostgreSQL integration tests".into(),
                ))
            })?;
            let options = PgConnectOptions::from_str(&target).map_err(DbError::from)?;
            let admin_options = options.clone().database("postgres");
            let admin = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(admin_options)
                .await
                .map_err(DbError::from)?;
            let name = format!("forge_test_{}", uuid::Uuid::now_v7().simple());

            // The identifier is generated locally from UUID hex, never from
            // environment/user input; PostgreSQL cannot bind DB identifiers.
            sqlx::query(AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
                .execute(&admin)
                .await
                .map_err(DbError::from)?;

            let database_url = options.database(&name).to_url_lossy().to_string();
            let config =
                forge_config::DatabaseConfig::new(forge_config::SecretString::new(database_url));
            let database = match crate::PgDatabase::connect(&config).await {
                Ok(database) => database,
                Err(error) => {
                    let _ =
                        sqlx::query(AssertSqlSafe(format!("DROP DATABASE IF EXISTS \"{name}\"")))
                            .execute(&admin)
                            .await;
                    return Err(error);
                }
            };
            if let Err(error) = database.migrate(migrator).await {
                database.close().await;
                let _ = sqlx::query(AssertSqlSafe(format!("DROP DATABASE IF EXISTS \"{name}\"")))
                    .execute(&admin)
                    .await;
                return Err(error);
            }

            Ok(Self {
                database,
                admin,
                name,
            })
        }

        /// Returns the isolated migrated database.
        #[must_use]
        pub const fn database(&self) -> &crate::PgDatabase {
            &self.database
        }

        /// Closes connections and drops the isolated database.
        pub async fn cleanup(self) -> Result<(), DbError> {
            use sqlx::{AssertSqlSafe, Executor};

            self.database.close().await;
            self.admin
                .execute(AssertSqlSafe(format!(
                    "DROP DATABASE IF EXISTS \"{}\"",
                    self.name
                )))
                .await
                .map_err(DbError::from)?;
            self.admin.close().await;
            Ok(())
        }
    }

    /// Error for test database provisioning.
    #[derive(Debug, thiserror::Error)]
    pub enum ProvisionError {
        #[error("database already exists: {0}")]
        AlreadyExists(String),
        #[error("database not found: {0}")]
        NotFound(String),
        #[error("provisioning failed: {0}")]
        Failed(#[from] DbError),
    }
}

/// Framework-owned database error.
///
/// The public message is intentionally bounded and does not contain SQL,
/// credentials, driver diagnostics, or database object contents. Concrete
/// adapters should log their internal source separately at the outer boundary.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct DatabaseError {
    kind: DatabaseErrorKind,
    message: &'static str,
    retryable: bool,
}

impl DatabaseError {
    /// Creates a classified database error with a safe public message.
    #[must_use]
    pub const fn new(kind: DatabaseErrorKind, message: &'static str, retryable: bool) -> Self {
        Self {
            kind,
            message,
            retryable,
        }
    }

    /// Error category suitable for application-level matching.
    #[must_use]
    pub const fn kind(&self) -> DatabaseErrorKind {
        self.kind
    }

    /// Whether retrying the operation can be reasonable.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

impl From<sqlx::Error> for DatabaseError {
    fn from(error: sqlx::Error) -> Self {
        Self::new(
            DatabaseErrorKind::Infrastructure,
            "database query failed",
            matches!(
                error,
                sqlx::Error::PoolTimedOut
                    | sqlx::Error::PoolClosed
                    | sqlx::Error::WorkerCrashed
                    | sqlx::Error::Io(_)
                    | sqlx::Error::Tls(_)
            ),
        )
    }
}

/// Monotonic optimistic-lock value stored with mutable records.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RecordVersion(i64);

impl Default for RecordVersion {
    fn default() -> Self {
        Self::INITIAL
    }
}

impl RecordVersion {
    /// Initial version for a newly persisted record.
    pub const INITIAL: Self = Self(1);

    /// Builds a version from a persisted positive value.
    pub fn new(value: i64) -> Result<Self, DatabaseError> {
        if value < 1 {
            return Err(DatabaseError::new(
                DatabaseErrorKind::Infrastructure,
                "invalid persisted record version",
                false,
            ));
        }
        Ok(Self(value))
    }

    /// Raw value used by a persistence adapter.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Next value for an optimistic update.
    pub fn next(self) -> Result<Self, DatabaseError> {
        self.0.checked_add(1).map(Self).ok_or_else(|| {
            DatabaseError::new(
                DatabaseErrorKind::Infrastructure,
                "record version overflow",
                false,
            )
        })
    }
}

/// Transaction behavior required by application use cases.
///
/// Concrete adapters own the connection/transaction implementation. The
/// transaction cannot be committed twice because commit/rollback consume it.
pub trait Transaction: Send {
    /// Commits all work in this transaction.
    fn commit(self) -> impl Future<Output = Result<(), DbError>> + Send;

    /// Explicitly rolls back all work in this transaction.
    fn rollback(self) -> impl Future<Output = Result<(), DbError>> + Send;
}

/// Starts lifetime-scoped transactions.
///
/// The generic associated transaction type prevents a transaction from
/// outliving the manager/adapter that created it.
pub trait TransactionManager: Send + Sync {
    /// Concrete transaction bound to the manager borrow.
    type Transaction<'a>: Transaction + 'a
    where
        Self: 'a;

    /// Begins a new transaction.
    fn begin(&self) -> impl Future<Output = Result<Self::Transaction<'_>, DbError>> + Send;
}

/// Validated maximum number of records requested from a repository page.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PageLimit(u16);

impl PageLimit {
    /// Conservative default page size.
    pub const DEFAULT: Self = Self(50);
    /// Hard upper bound to keep accidental unbounded reads out of repository APIs.
    pub const MAX: u16 = 500;

    /// Creates a page limit in the inclusive range 1..=MAX.
    pub fn new(value: u16) -> Result<Self, DatabaseError> {
        if value == 0 || value > Self::MAX {
            return Err(DatabaseError::new(
                DatabaseErrorKind::Infrastructure,
                "invalid repository page limit",
                false,
            ));
        }
        Ok(Self(value))
    }

    /// Returns the validated raw limit.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl Default for PageLimit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One deterministic cursor-paginated repository page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorPage<T, C> {
    /// Records in stable repository order.
    pub items: Vec<T>,
    /// Cursor to request the next page, or None when this is the final page.
    pub next: Option<C>,
}

impl<T, C> CursorPage<T, C> {
    /// Creates a page without exposing adapter-specific pagination types.
    #[must_use]
    pub fn new(items: Vec<T>, next: Option<C>) -> Self {
        Self { items, next }
    }
}

/// Conventional typed repository contract for aggregate persistence.
///
/// Applications remain free to define narrower domain-specific ports when CRUD
/// semantics are not appropriate. This contract exists for ordinary aggregates
/// and deliberately exposes only Forge-owned types. SQLx rows, pools,
/// transactions and query builders stay in infrastructure adapters.
pub trait Repository: Send + Sync {
    /// Nominal identifier type, typically forge_core::Id<Marker>.
    type Id: Send + Sync;
    /// Aggregate/entity returned by the repository.
    type Entity: Send + Sync;
    /// Opaque deterministic cursor owned by the application/adapter contract.
    type Cursor: Send + Sync;

    /// Loads an entity by typed identifier.
    fn find(
        &self,
        id: &Self::Id,
    ) -> impl Future<Output = Result<Option<Self::Entity>, DatabaseError>> + Send;

    /// Inserts a new entity at RecordVersion::INITIAL.
    fn insert(
        &self,
        entity: &Self::Entity,
    ) -> impl Future<Output = Result<(), DatabaseError>> + Send;

    /// Persists an optimistic update and returns the new record version.
    ///
    /// Adapters must classify a zero-row optimistic update as
    /// DatabaseErrorKind::Conflict rather than silently overwriting newer data.
    fn update(
        &self,
        entity: &Self::Entity,
        expected_version: RecordVersion,
    ) -> impl Future<Output = Result<RecordVersion, DatabaseError>> + Send;

    /// Deletes an entity only when its persisted version matches.
    fn delete(
        &self,
        id: &Self::Id,
        expected_version: RecordVersion,
    ) -> impl Future<Output = Result<(), DatabaseError>> + Send;

    /// Reads a bounded deterministic page.
    ///
    /// Concrete adapters should use an indexed stable ordering tuple (normally
    /// created_at plus id) and must not emulate cursors with unbounded OFFSET.
    fn page(
        &self,
        after: Option<&Self::Cursor>,
        limit: PageLimit,
    ) -> impl Future<Output = Result<CursorPage<Self::Entity, Self::Cursor>, DatabaseError>> + Send;
}

/// Typed repository contract for tenant-owned aggregates.
///
/// Every operation requires an already authenticated and authorized
/// TenantContext. There is intentionally no overload that accepts only a raw
/// tenant identifier, so tenant-sensitive application code cannot accidentally
/// omit the authorization boundary.
pub trait TenantRepository: Send + Sync {
    /// Nominal identifier type, typically forge_core::Id<Marker>.
    type Id: Send + Sync;
    /// Aggregate/entity returned by the repository.
    type Entity: Send + Sync;
    /// Opaque deterministic cursor owned by the application/adapter contract.
    type Cursor: Send + Sync;

    /// Loads an entity within the authorized tenant.
    fn find(
        &self,
        context: &TenantContext,
        id: &Self::Id,
    ) -> impl Future<Output = Result<Option<Self::Entity>, DatabaseError>> + Send;

    /// Inserts an entity within the authorized tenant.
    fn insert(
        &self,
        context: &TenantContext,
        entity: &Self::Entity,
    ) -> impl Future<Output = Result<(), DatabaseError>> + Send;

    /// Persists an optimistic tenant-scoped update.
    fn update(
        &self,
        context: &TenantContext,
        entity: &Self::Entity,
        expected_version: RecordVersion,
    ) -> impl Future<Output = Result<RecordVersion, DatabaseError>> + Send;

    /// Deletes within the authorized tenant when the version matches.
    fn delete(
        &self,
        context: &TenantContext,
        id: &Self::Id,
        expected_version: RecordVersion,
    ) -> impl Future<Output = Result<(), DatabaseError>> + Send;

    /// Reads a bounded deterministic page within the authorized tenant.
    fn page(
        &self,
        context: &TenantContext,
        after: Option<&Self::Cursor>,
        limit: PageLimit,
    ) -> impl Future<Output = Result<CursorPage<Self::Entity, Self::Cursor>, DatabaseError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_positive_and_checked() {
        assert!(RecordVersion::new(0).is_err());
        assert_eq!(RecordVersion::INITIAL.get(), 1);
        assert_eq!(RecordVersion::default(), RecordVersion::INITIAL);
        assert_eq!(
            RecordVersion::INITIAL
                .next()
                .expect("version should increment")
                .get(),
            2
        );
        assert!(
            RecordVersion::new(i64::MAX)
                .expect("max positive version is valid")
                .next()
                .is_err()
        );
    }

    #[test]
    fn page_limits_are_bounded() {
        assert_eq!(PageLimit::default().get(), 50);
        assert_eq!(PageLimit::new(1).expect("minimum is valid").get(), 1);
        assert_eq!(
            PageLimit::new(PageLimit::MAX)
                .expect("maximum is valid")
                .get(),
            PageLimit::MAX
        );
        assert!(PageLimit::new(0).is_err());
        assert!(PageLimit::new(PageLimit::MAX + 1).is_err());
    }

    #[test]
    fn cursor_pages_keep_adapter_types_out_of_the_contract() {
        let page = CursorPage::new(vec!["a", "b"], Some("next"));
        assert_eq!(page.items, vec!["a", "b"]);
        assert_eq!(page.next, Some("next"));
    }

    #[test]
    fn error_classification_is_explicit() {
        let error =
            DatabaseError::new(DatabaseErrorKind::Unavailable, "database unavailable", true);
        assert_eq!(error.kind(), DatabaseErrorKind::Unavailable);
        assert!(error.retryable());
        assert_eq!(error.to_string(), "database unavailable");
    }
}
