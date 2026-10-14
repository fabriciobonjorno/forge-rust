//! Supported database backends.
//!
//! [`Backend`] is implemented for `sqlx::Postgres`, `sqlx::MySql` and
//! `sqlx::Sqlite`, each behind the Cargo feature of the same name. It is
//! sealed: the backend-specific behavior (URL parsing and connection
//! settings, migration bookkeeping, test database provisioning and error
//! classification) lives in one small module per backend, so
//! [`Database`](crate::Database) needs no bounds beyond `B: Backend`.

use forge_config::DatabaseScheme;
use sqlx::{Connection, Pool, error::DatabaseError, migrate::Migrator, pool::PoolOptions};

use crate::{DbError, error::Kind};

#[cfg(feature = "mysql")]
#[path = "mysql.rs"]
pub(crate) mod mysql;
#[cfg(feature = "postgres")]
#[path = "postgres.rs"]
pub(crate) mod postgres;
#[cfg(feature = "sqlite")]
#[path = "sqlite.rs"]
pub(crate) mod sqlite;

/// A database Forge can connect to: PostgreSQL, MySQL/MariaDB or SQLite.
///
/// Implemented for `sqlx::Postgres` (feature `postgres`), `sqlx::MySql`
/// (feature `mysql`) and `sqlx::Sqlite` (feature `sqlite`). The trait is
/// sealed; its display name is SQLx's `Database::NAME`
/// (`"PostgreSQL"`, `"MySQL"`, `"SQLite"`).
pub trait Backend: sealed::Hooks {
    /// The URL schemes this backend accepts in `FORGE_DATABASE_URL`.
    const SCHEME: DatabaseScheme;
}

/// A statement result that reports the number of affected rows.
///
/// Implemented for the query results of every enabled backend
/// (`PgQueryResult`, `MySqlQueryResult`, `SqliteQueryResult`) so that
/// [`expect_rows`](crate::expect_rows) accepts any of them. Sealed.
pub trait RowsAffected: sealed::QueryResult {
    /// Number of rows inserted, updated or deleted by the statement.
    fn rows_affected(&self) -> u64;
}

/// Backend hooks that are implementation details of `forge-db`.
///
/// The traits are public (so they can bound the public [`Backend`] and
/// [`RowsAffected`] traits) but live in a private module, so no other crate
/// can name or implement them.
pub(crate) mod sealed {
    use super::{Connection, DbError, Migrator, Pool, PoolOptions};
    use forge_config::DatabaseScheme;
    use sqlx::migrate::Migrate;

    /// Per-backend behavior behind [`super::Backend`].
    pub trait Hooks: sqlx::Database {
        /// The URL scheme this backend accepts in `FORGE_DATABASE_URL`.
        const SCHEME: DatabaseScheme;

        /// The connection type must implement Migrate for migration operations.
        type Connection: Migrate + sqlx::Connection;

        /// Parses a URL whose scheme has already been checked and applies
        /// Forge's connection settings.
        fn connect_options(
            url: &str,
        ) -> Result<<<Self as Hooks>::Connection as Connection>::Options, sqlx::Error>;

        /// Adjusts pool settings for the URL; the default keeps them.
        fn pool_options(url: &str, options: PoolOptions<Self>) -> PoolOptions<Self> {
            let _ = url;
            options
        }

        /// Applies pending migrations; see `Database::migrate`.
        fn apply_pending<'a>(
            pool: &'a Pool<Self>,
            migrator: &'a Migrator,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<i64>, DbError>> + Send + 'a>,
        >;

        /// Reverts the latest migration; see `Database::rollback`.
        fn revert_latest<'a>(
            pool: &'a Pool<Self>,
            migrator: &'a Migrator,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<i64>, DbError>> + Send + 'a>,
        >;

        /// Prepares the migration session; runs before the migration lock is
        /// taken. The default does nothing.
        fn prepare_migration_session(
            connection: &mut <Self as sqlx::Database>::Connection,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DbError>> + Send + '_>>
        {
            let _ = connection;
            Box::pin(std::future::ready(Ok(())))
        }

        /// Undoes [`Hooks::prepare_migration_session`] before a successful
        /// migration connection returns to the pool. The default does
        /// nothing.
        fn restore_session(
            connection: &mut <Self as sqlx::Database>::Connection,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DbError>> + Send + '_>>
        {
            let _ = connection;
            Box::pin(std::future::ready(Ok(())))
        }

        /// Creates an empty, uniquely named database for a test.
        #[cfg(feature = "testing")]
        fn provision_test_database(
            name: &str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<crate::testing::Provisioned, DbError>>
                    + Send
                    + '_,
            >,
        >;
    }

    /// Query results accepted by `expect_rows`.
    pub trait QueryResult {}
}

/// Backend-specific classification of a driver error, or `None` to fall
/// back to the portable SQLSTATE rules.
#[expect(dead_code)]
pub(crate) fn classify(error: &dyn DatabaseError) -> Option<Kind> {
    Some(DbError::classify(error))
}

/// Name of the violated constraint: reported by the driver (PostgreSQL) or
/// recovered from the error message (MySQL/MariaDB, SQLite `CHECK`).
#[expect(dead_code)]
pub(crate) fn constraint(error: &dyn DatabaseError) -> Option<String> {
    if let Some(name) = error.constraint() {
        return Some(name.to_owned());
    }
    // For MySQL and SQLite, constraint names are recovered from the error message
    // This is handled by DbError::classify and the From<sqlx::Error> impl
    None
}

/// Accepts `name` only if it looks like an SQL identifier, so text recovered
/// from a driver message can never smuggle SQL or data into error metadata.
#[cfg(any(feature = "mysql", feature = "sqlite"))]
#[allow(dead_code)]
pub(crate) fn identifier(name: &str) -> Option<String> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'));
    valid.then(|| name.to_owned())
}

/// Returns the text between `start` and the next `end` after it.
#[cfg(any(feature = "mysql", feature = "sqlite"))]
#[allow(dead_code)]
pub(crate) fn between<'a>(message: &'a str, start: &str, end: char) -> Option<&'a str> {
    let rest = &message[message.rfind(start)? + start.len()..];
    rest.split(end).next()
}

#[cfg(all(test, any(feature = "mysql", feature = "sqlite")))]
mod tests {
    use super::*;

    #[test]
    fn only_identifier_like_names_are_accepted() {
        assert_eq!(
            identifier("owners_email_key").as_deref(),
            Some("owners_email_key")
        );
        assert_eq!(identifier("t$1").as_deref(), Some("t$1"));
        for rejected in ["", "length(title) > 0", "a;b", "a-b", "émail"] {
            assert_eq!(identifier(rejected), None, "{rejected}");
        }
        assert_eq!(identifier(&"x".repeat(129)), None);
    }

    #[test]
    fn between_uses_the_last_marker() {
        assert_eq!(
            between(
                "Duplicate entry 'for key 'x'' for key 'owners.email'",
                "for key '",
                '\''
            ),
            Some("owners.email")
        );
        assert_eq!(between("no marker here", "for key '", '\''), None);
    }
}
