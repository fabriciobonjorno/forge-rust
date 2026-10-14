//! Classified persistence errors and their mapping to [`ForgeError`].

use std::fmt;

use forge_core::{ForgeError, IntoForgeError};
use sqlx::{
    error::{DatabaseError, ErrorKind},
    migrate::MigrateError,
};
use thiserror::Error;

/// A classified database failure (PostgreSQL, MySQL/MariaDB or SQLite).
///
/// [`From<sqlx::Error>`] inspects the driver's error kind, SQLSTATE and
/// backend error codes so callers can react to constraint violations,
/// retryable conditions and optimistic-lock conflicts without parsing driver
/// messages. Converting into [`ForgeError`] yields a stable code and a
/// caller-safe message; SQL text, driver messages and constraint names are
/// never part of that message (a constraint name is attached as `constraint`
/// metadata when known).
///
/// | Variant | PostgreSQL | MySQL / MariaDB | SQLite |
/// | --- | --- | --- | --- |
/// | `UniqueViolation` | `23505` | `1062` (and other duplicate-key errors) | `SQLITE_CONSTRAINT_UNIQUE`, `_PRIMARYKEY` |
/// | `ForeignKeyViolation` | `23503` | `1451`, `1452` | `SQLITE_CONSTRAINT_FOREIGNKEY` |
/// | `CheckViolation` | `23514`, `23502` | `3819`, MariaDB `4025`, `1048` | `SQLITE_CONSTRAINT_CHECK`, `_NOTNULL` |
/// | `SerializationFailure` | `40001`, `40P01` | `1213` (deadlock), `1205` (lock wait timeout) | `SQLITE_BUSY`, `SQLITE_LOCKED` |
/// | `Unavailable` | class `08`, `57P01`-`57P03`, `53300` | class `08`, `1040`, `1053`, `1927`, `4031` | `SQLITE_IOERR`, `SQLITE_CANTOPEN` |
///
/// Constraint names come from the driver on PostgreSQL and from the error
/// message on MySQL/MariaDB (a MySQL 8 `table.key` is reported as `key`) and
/// for SQLite `CHECK` constraints; SQLite reports no name for other
/// constraints. Recovered names must look like SQL identifiers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DbError {
    /// A unique or primary-key constraint was violated.
    #[error("unique constraint violated{}", ConstraintSuffix(.constraint.as_deref()))]
    UniqueViolation {
        /// Name of the violated constraint or index, when reported.
        constraint: Option<String>,
    },
    /// A foreign-key constraint was violated.
    #[error("foreign key constraint violated{}", ConstraintSuffix(.constraint.as_deref()))]
    ForeignKeyViolation {
        /// Name of the violated constraint, when reported.
        constraint: Option<String>,
    },
    /// A check or not-null constraint was violated.
    #[error("check constraint violated{}", ConstraintSuffix(.constraint.as_deref()))]
    CheckViolation {
        /// Name of the violated constraint, when reported.
        constraint: Option<String>,
    },
    /// The transaction lost a serialization race, was chosen as a deadlock
    /// victim, or timed out waiting for a lock (SQLite: the database was
    /// busy). Retrying the whole transaction may succeed.
    #[error("transaction aborted by a serialization failure or deadlock")]
    SerializationFailure,
    /// The requested row does not exist.
    #[error("row not found")]
    NotFound,
    /// The row changed since it was read (optimistic locking) or otherwise
    /// conflicts with current state.
    #[error("row was modified concurrently")]
    Conflict,
    /// No pooled connection became available within the acquire timeout.
    #[error("timed out waiting for a database connection")]
    PoolTimeout,
    /// The database could not be reached or dropped the connection.
    #[error("database unavailable")]
    Unavailable(#[source] sqlx::Error),
    /// Applying or reverting migrations failed.
    #[error("database migration failed")]
    Migration(#[source] MigrateError),
    /// The most recent migration has no down migration and cannot be reverted.
    #[error("migration {version} cannot be reverted because it has no down migration")]
    IrreversibleMigration {
        /// Version of the most recently applied migration.
        version: i64,
    },
    /// The connection settings are invalid or the pool cannot be created.
    #[error("invalid database configuration")]
    Configuration(#[source] sqlx::Error),
    /// Any other database failure.
    #[error("database query failed")]
    Query(#[source] sqlx::Error),
}

impl DbError {
    /// Returns the error kind for categorization.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Self::UniqueViolation { .. } => Kind::UniqueViolation,
            Self::ForeignKeyViolation { .. } => Kind::ForeignKeyViolation,
            Self::CheckViolation { .. } => Kind::CheckViolation,
            Self::SerializationFailure => Kind::SerializationFailure,
            Self::NotFound => Kind::NotFound,
            Self::Conflict => Kind::Conflict,
            Self::PoolTimeout => Kind::PoolTimeout,
            Self::Unavailable(_) => Kind::Unavailable,
            Self::Migration(_) => Kind::Migration,
            Self::IrreversibleMigration { .. } => Kind::Migration,
            Self::Configuration(_) => Kind::Configuration,
            Self::Query(_) => Kind::Query,
        }
    }

    /// Returns the stable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UniqueViolation { .. } => "unique_violation",
            Self::ForeignKeyViolation { .. } => "foreign_key_violation",
            Self::CheckViolation { .. } => "check_violation",
            Self::SerializationFailure => "serialization_failure",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::PoolTimeout => "pool_timeout",
            Self::Unavailable(_) => "unavailable",
            Self::Migration(_) => "migration_failed",
            Self::IrreversibleMigration { .. } => "irreversible_migration",
            Self::Configuration(_) => "configuration_error",
            Self::Query(_) => "query_failed",
        }
    }

    /// Returns the caller-safe error message.
    #[must_use]
    pub fn message(&self) -> &'static str {
        match self {
            Self::UniqueViolation { .. } => "unique constraint violated",
            Self::ForeignKeyViolation { .. } => "foreign key constraint violated",
            Self::CheckViolation { .. } => "check constraint violated",
            Self::SerializationFailure => {
                "transaction aborted by a serialization failure or deadlock"
            }
            Self::NotFound => "row not found",
            Self::Conflict => "row was modified concurrently",
            Self::PoolTimeout => "timed out waiting for a database connection",
            Self::Unavailable(_) => "database unavailable",
            Self::Migration(_) => "database migration failed",
            Self::IrreversibleMigration { .. } => {
                "migration cannot be reverted because it has no down migration"
            }
            Self::Configuration(_) => "invalid database configuration",
            Self::Query(_) => "database query failed",
        }
    }

    /// Returns whether retrying the operation (or the whole transaction) may
    /// succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::SerializationFailure | Self::PoolTimeout | Self::Unavailable(_)
        )
    }

    /// Returns the safe constraint identifier when a driver reported one.
    #[must_use]
    pub fn constraint(&self) -> Option<&str> {
        match self {
            Self::UniqueViolation { constraint }
            | Self::ForeignKeyViolation { constraint }
            | Self::CheckViolation { constraint } => constraint.as_deref(),
            _ => None,
        }
    }
}

struct ConstraintSuffix<'a>(Option<&'a str>);

impl<'a> fmt::Display for ConstraintSuffix<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = self.0 {
            write!(f, " ({name})")
        } else {
            Ok(())
        }
    }
}

impl DbError {
    fn kind_from_database_error(error: &sqlx::Error) -> Option<Kind> {
        if let sqlx::Error::Database(db_error) = error {
            Some(Self::classify(db_error.as_ref()))
        } else {
            // Non-database errors (migration, pool timeout, etc.)
            match error {
                sqlx::Error::PoolTimedOut => Some(Kind::PoolTimeout),
                sqlx::Error::PoolClosed => Some(Kind::Unavailable),
                sqlx::Error::WorkerCrashed => Some(Kind::Unavailable),
                sqlx::Error::Io(_) => Some(Kind::Unavailable),
                sqlx::Error::Tls(_) => Some(Kind::Unavailable),
                sqlx::Error::Configuration(_) => Some(Kind::Configuration),
                sqlx::Error::Migrate(_) => Some(Kind::Migration),
                sqlx::Error::RowNotFound => Some(Kind::NotFound),
                _ => None,
            }
        }
    }

    /// Classifies a database error into a `Kind` variant.
    ///
    /// This is the backend-specific classification logic that uses SQLSTATE codes,
    /// backend error codes, and portable fallback to determine the error kind.
    pub(crate) fn classify(database_error: &dyn DatabaseError) -> Kind {
        // PostgreSQL error codes
        if let Some(code) = database_error.code() {
            match code.as_ref() {
                "23505" => return Kind::UniqueViolation,
                "23503" => return Kind::ForeignKeyViolation,
                "23514" | "23502" => return Kind::CheckViolation,
                "40001" | "40P01" => return Kind::SerializationFailure,
                code if code.starts_with("08") => return Kind::Unavailable,
                code if code.starts_with("57P") => return Kind::Unavailable,
                "53300" => return Kind::Unavailable,
                _ => {}
            }
        }

        // SQLite error codes
        if let Some(code) = database_error.code() {
            match code.as_ref() {
                "SQLITE_CONSTRAINT_UNIQUE" | "SQLITE_CONSTRAINT_PRIMARYKEY" => {
                    return Kind::UniqueViolation;
                }
                "SQLITE_CONSTRAINT_FOREIGNKEY" => return Kind::ForeignKeyViolation,
                "SQLITE_CONSTRAINT_CHECK" | "SQLITE_CONSTRAINT_NOTNULL" => {
                    return Kind::CheckViolation;
                }
                "SQLITE_BUSY" | "SQLITE_LOCKED" => return Kind::SerializationFailure,
                "SQLITE_IOERR" | "SQLITE_CANTOPEN" => return Kind::Unavailable,
                _ => {}
            }
        }

        // MySQL error codes
        if let Some(code) = database_error.code()
            && let Ok(num) = code.as_ref().parse::<u16>()
        {
            match num {
                1062 => return Kind::UniqueViolation,
                1451 | 1452 => return Kind::ForeignKeyViolation,
                3819 | 4025 | 1048 => return Kind::CheckViolation,
                1213 | 1205 => return Kind::SerializationFailure,
                1040 | 1053 | 1927 | 4031 => return Kind::Unavailable,
                _ => {}
            }
        }

        // Fallback to portable classification
        match database_error.kind() {
            ErrorKind::UniqueViolation => Kind::UniqueViolation,
            ErrorKind::ForeignKeyViolation => Kind::ForeignKeyViolation,
            ErrorKind::NotNullViolation => Kind::CheckViolation,
            ErrorKind::CheckViolation => Kind::CheckViolation,
            ErrorKind::ExclusionViolation => Kind::Constraint,
            ErrorKind::Other => Kind::Query,
            _ => Kind::Query,
        }
    }
}

impl From<MigrateError> for DbError {
    fn from(error: MigrateError) -> Self {
        Self::Migration(error)
    }
}

impl From<sqlx::Error> for DbError {
    fn from(error: sqlx::Error) -> Self {
        if let Some(kind) = Self::kind_from_database_error(&error) {
            match kind {
                Kind::UniqueViolation => Self::UniqueViolation {
                    constraint: error
                        .as_database_error()
                        .and_then(|e| e.constraint())
                        .map(|s| s.to_owned()),
                },
                Kind::ForeignKeyViolation => Self::ForeignKeyViolation {
                    constraint: error
                        .as_database_error()
                        .and_then(|e| e.constraint())
                        .map(|s| s.to_owned()),
                },
                Kind::CheckViolation => Self::CheckViolation {
                    constraint: error
                        .as_database_error()
                        .and_then(|e| e.constraint())
                        .map(|s| s.to_owned()),
                },
                Kind::SerializationFailure => Self::SerializationFailure,
                Kind::NotFound => Self::NotFound,
                Kind::Conflict => Self::Conflict,
                Kind::PoolTimeout => Self::PoolTimeout,
                Kind::Unavailable => Self::Unavailable(error),
                Kind::Migration => {
                    if let sqlx::Error::Migrate(migrate_error) = error {
                        Self::Migration(*migrate_error)
                    } else {
                        Self::Migration(MigrateError::VersionMissing(0))
                    }
                }
                Kind::Configuration => Self::Configuration(error),
                Kind::Query => Self::Query(error),
                Kind::Constraint => Self::Query(error),
                Kind::Infrastructure => Self::Query(error),
            }
        } else {
            Self::Query(error)
        }
    }
}

/// Stable error categories for application-level matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// A unique or primary-key constraint was violated.
    UniqueViolation,
    /// A foreign-key constraint was violated.
    ForeignKeyViolation,
    /// A check or not-null constraint was violated.
    CheckViolation,
    /// The transaction lost a serialization race or timed out on a lock.
    SerializationFailure,
    /// The requested row does not exist.
    NotFound,
    /// The row changed since it was read.
    Conflict,
    /// No pooled connection became available within the acquire timeout.
    PoolTimeout,
    /// The database could not be reached or dropped the connection.
    Unavailable,
    /// Applying or reverting migrations failed.
    Migration,
    /// The connection settings are invalid or the pool cannot be created.
    Configuration,
    /// Any other database failure.
    Query,
    /// A database constraint was violated (generic, for decode/column issues).
    Constraint,
    /// Adapter-specific failure that is safe to classify only as infrastructure.
    Infrastructure,
}

impl IntoForgeError for DbError {
    fn into_forge_error(self) -> ForgeError {
        let category = match self.kind() {
            Kind::UniqueViolation => forge_core::ErrorCategory::Conflict,
            Kind::ForeignKeyViolation => forge_core::ErrorCategory::Conflict,
            Kind::CheckViolation => forge_core::ErrorCategory::Validation,
            Kind::SerializationFailure => forge_core::ErrorCategory::Infrastructure,
            Kind::NotFound => forge_core::ErrorCategory::NotFound,
            Kind::Conflict => forge_core::ErrorCategory::Conflict,
            Kind::PoolTimeout => forge_core::ErrorCategory::Infrastructure,
            Kind::Unavailable => forge_core::ErrorCategory::Infrastructure,
            Kind::Migration => forge_core::ErrorCategory::Internal,
            Kind::Configuration => forge_core::ErrorCategory::Internal,
            Kind::Query => forge_core::ErrorCategory::Internal,
            Kind::Constraint => forge_core::ErrorCategory::Validation,
            Kind::Infrastructure => forge_core::ErrorCategory::Infrastructure,
        };
        ForgeError::new(category, self.code(), self.message())
            .with_retryable(self.is_retryable())
            .with_source(self)
    }
}
