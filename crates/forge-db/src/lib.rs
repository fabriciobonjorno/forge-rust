//! Database contracts owned by Forge.
//!
//! This crate deliberately contains no SQLx or PostgreSQL types. Application
//! ports can depend on these transaction/error contracts while concrete SQLx
//! adapters remain in infrastructure.

use async_trait::async_trait;
use forge_tenancy::TenantContext;
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
#[async_trait]
pub trait Repository: Send + Sync {
    /// Nominal identifier type, typically forge_core::Id<Marker>.
    type Id: Send + Sync;
    /// Aggregate/entity returned by the repository.
    type Entity: Send + Sync;
    /// Opaque deterministic cursor owned by the application/adapter contract.
    type Cursor: Send + Sync;

    /// Loads an entity by typed identifier.
    async fn find(&self, id: &Self::Id) -> Result<Option<Self::Entity>, DatabaseError>;

    /// Inserts a new entity at RecordVersion::INITIAL.
    async fn insert(&self, entity: &Self::Entity) -> Result<(), DatabaseError>;

    /// Persists an optimistic update and returns the new record version.
    ///
    /// Adapters must classify a zero-row optimistic update as
    /// DatabaseErrorKind::Conflict rather than silently overwriting newer data.
    async fn update(
        &self,
        entity: &Self::Entity,
        expected_version: RecordVersion,
    ) -> Result<RecordVersion, DatabaseError>;

    /// Deletes an entity only when its persisted version matches.
    async fn delete(
        &self,
        id: &Self::Id,
        expected_version: RecordVersion,
    ) -> Result<(), DatabaseError>;

    /// Reads a bounded deterministic page.
    ///
    /// Concrete adapters should use an indexed stable ordering tuple (normally
    /// created_at plus id) and must not emulate cursors with unbounded OFFSET.
    async fn page(
        &self,
        after: Option<&Self::Cursor>,
        limit: PageLimit,
    ) -> Result<CursorPage<Self::Entity, Self::Cursor>, DatabaseError>;
}

/// Typed repository contract for tenant-owned aggregates.
///
/// Every operation requires an already authenticated and authorized
/// TenantContext. There is intentionally no overload that accepts only a raw
/// tenant identifier, so tenant-sensitive application code cannot accidentally
/// omit the authorization boundary.
#[async_trait]
pub trait TenantRepository: Send + Sync {
    /// Nominal identifier type, typically forge_core::Id<Marker>.
    type Id: Send + Sync;
    /// Aggregate/entity returned by the repository.
    type Entity: Send + Sync;
    /// Opaque deterministic cursor owned by the application/adapter contract.
    type Cursor: Send + Sync;

    /// Loads an entity within the authorized tenant.
    async fn find(
        &self,
        context: &TenantContext,
        id: &Self::Id,
    ) -> Result<Option<Self::Entity>, DatabaseError>;

    /// Inserts an entity within the authorized tenant.
    async fn insert(
        &self,
        context: &TenantContext,
        entity: &Self::Entity,
    ) -> Result<(), DatabaseError>;

    /// Persists an optimistic tenant-scoped update.
    async fn update(
        &self,
        context: &TenantContext,
        entity: &Self::Entity,
        expected_version: RecordVersion,
    ) -> Result<RecordVersion, DatabaseError>;

    /// Deletes within the authorized tenant when the version matches.
    async fn delete(
        &self,
        context: &TenantContext,
        id: &Self::Id,
        expected_version: RecordVersion,
    ) -> Result<(), DatabaseError>;

    /// Reads a bounded deterministic page within the authorized tenant.
    async fn page(
        &self,
        context: &TenantContext,
        after: Option<&Self::Cursor>,
        limit: PageLimit,
    ) -> Result<CursorPage<Self::Entity, Self::Cursor>, DatabaseError>;
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
