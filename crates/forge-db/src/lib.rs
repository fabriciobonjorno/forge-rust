//! Database contracts owned by Forge.
//!
//! This crate deliberately contains no SQLx or PostgreSQL types. Application
//! ports can depend on these transaction/error contracts while concrete SQLx
//! adapters remain in infrastructure.

use async_trait::async_trait;
use thiserror::Error;

/// Stable database failure categories exposed across application boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseErrorKind {
    /// The database or connection pool is unavailable.
    Unavailable,
    /// An operation exceeded its deadline.
    Timeout,
    /// Optimistic concurrency check failed.
    Conflict,
    /// A database constraint rejected the write.
    Constraint,
    /// Migration execution or validation failed.
    Migration,
    /// Adapter-specific failure that is safe to classify only as infrastructure.
    Infrastructure,
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
    pub const fn new(
        kind: DatabaseErrorKind,
        message: &'static str,
        retryable: bool,
    ) -> Self {
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

/// Monotonic optimistic-lock value stored with mutable records.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct RecordVersion(i64);

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
#[async_trait]
pub trait Transaction: Send {
    /// Commits all work in this transaction.
    async fn commit(self) -> Result<(), DatabaseError>;

    /// Explicitly rolls back all work in this transaction.
    async fn rollback(self) -> Result<(), DatabaseError>;
}

/// Starts lifetime-scoped transactions.
///
/// The generic associated transaction type prevents a transaction from
/// outliving the manager/adapter that created it.
#[async_trait]
pub trait TransactionManager: Send + Sync {
    /// Concrete transaction bound to the manager borrow.
    type Transaction<'a>: Transaction + 'a
    where
        Self: 'a;

    /// Begins a new transaction.
    async fn begin(&self) -> Result<Self::Transaction<'_>, DatabaseError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_positive_and_checked() {
        assert!(RecordVersion::new(0).is_err());
        assert_eq!(RecordVersion::INITIAL.get(), 1);
        assert_eq!(
            RecordVersion::INITIAL
                .next()
                .expect("version should increment")
                .get(),
            2
        );
        assert!(RecordVersion::new(i64::MAX)
            .expect("max positive version is valid")
            .next()
            .is_err());
    }

    #[test]
    fn error_classification_is_explicit() {
        let error = DatabaseError::new(
            DatabaseErrorKind::Unavailable,
            "database unavailable",
            true,
        );
        assert_eq!(error.kind(), DatabaseErrorKind::Unavailable);
        assert!(error.retryable());
        assert_eq!(error.to_string(), "database unavailable");
    }
}
