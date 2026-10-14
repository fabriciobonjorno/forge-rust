//! Event outbox and consumer contracts for Forge applications.
//!
//! This crate provides the transactional outbox pattern for reliable event
//! publishing and idempotent consumer contracts for at-least-once delivery.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use forge_core::Id;
use forge_db::DbError;
use forge_jobs::JobError;
use forge_tenancy::TenantId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// An event identifier.
pub type EventId = Id<EventMarker>;

/// Marker type for event identifiers.
pub enum EventMarker {}

/// An event payload — opaque to the outbox, meaningful to consumers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventPayload(pub Vec<u8>);

impl EventPayload {
    /// Creates a new event payload from JSON-serializable data.
    pub fn new<T: Serialize>(value: &T) -> Result<Self, EventError> {
        serde_json::to_vec(value)
            .map(Self)
            .map_err(|e| EventError::Serialization(e.to_string()))
    }

    /// Decodes the payload as JSON.
    pub fn decode<T: for<'de> Deserialize<'de>>(&self) -> Result<T, EventError> {
        serde_json::from_slice(&self.0).map_err(|e| EventError::Deserialization(e.to_string()))
    }
}

/// Event status in the outbox.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    /// Pending publication.
    Pending,
    /// Published successfully.
    Published,
    /// Failed permanently after exhausting retries.
    DeadLetter,
}

/// An event in the outbox.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboxEvent {
    /// Unique event identifier.
    pub id: EventId,
    /// Tenant that owns this event (for multi-tenant outboxes).
    pub tenant_id: Option<TenantId>,
    /// Event type (e.g., "user.created", "order.completed").
    pub event_type: String,
    /// Event payload.
    pub payload: EventPayload,
    /// Current status.
    pub status: EventStatus,
    /// Number of publication attempts.
    pub attempts: u32,
    /// Maximum publication attempts.
    pub max_attempts: u32,
    /// When the event was created (transaction commit time).
    pub created_at: DateTime<Utc>,
    /// When the event was last updated.
    pub updated_at: DateTime<Utc>,
    /// Last error message (if any).
    pub last_error: Option<String>,
    /// Correlation ID for tracing.
    pub correlation_id: Option<String>,
    /// Causation ID for tracing.
    pub causation_id: Option<String>,
}

/// Event outbox errors.
#[derive(Debug, Error)]
pub enum EventError {
    /// Event not found.
    #[error("event not found: {0}")]
    NotFound(EventId),
    /// Event is in a terminal state and cannot be modified.
    #[error("event is in terminal state: {0:?}")]
    TerminalState(EventStatus),
    /// Maximum publication attempts exceeded.
    #[error("max attempts exceeded: {0}")]
    MaxAttemptsExceeded(u32),
    /// Serialization error.
    #[error("serialization failed: {0}")]
    Serialization(String),
    /// Deserialization error.
    #[error("deserialization failed: {0}")]
    Deserialization(String),
    /// Database error.
    #[error(transparent)]
    Database(#[from] DbError),
    /// Configuration error.
    #[error("configuration error: {0}")]
    Configuration(String),
    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
    /// Job queue error.
    #[error(transparent)]
    JobQueue(#[from] JobError),
}

impl EventError {
    /// Returns whether this error is retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            EventError::Database(e) if e.is_retryable() => true,
            EventError::JobQueue(e) if e.is_retryable() => true,
            _ => false,
        }
    }
}

/// Port for writing events to the outbox within a transaction.
#[async_trait]
pub trait OutboxWriter: Send + Sync {
    /// Writes an event to the outbox within the current transaction.
    /// The event will only be visible to the publisher after the transaction commits.
    async fn write(&self, event: OutboxEvent) -> Result<EventId, EventError>;

    /// Writes multiple events to the outbox within the current transaction.
    async fn write_batch(&self, events: Vec<OutboxEvent>) -> Result<Vec<EventId>, EventError>;
}

/// Port for publishing events from the outbox.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publishes pending events, returning the count of published events.
    async fn publish_pending(&self, limit: usize) -> Result<u64, EventError>;

    /// Gets statistics about the outbox.
    async fn stats(&self) -> Result<OutboxStats, EventError>;
}

/// Port for consuming events.
#[async_trait]
pub trait EventConsumer: Send + Sync {
    /// Consumes an event, returning whether it was processed successfully.
    /// The implementation must be idempotent - the same event may be delivered multiple times.
    async fn consume(&self, event: &OutboxEvent) -> Result<ConsumeResult, EventError>;

    /// Returns the event types this consumer handles.
    fn handles(&self) -> &[String];
}

/// Result of consuming an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumeResult {
    /// Event processed successfully.
    Success,
    /// Event processing failed but should be retried.
    Retry,
    /// Event processing failed permanently - move to dead letter.
    DeadLetter,
    /// Event was already processed (idempotent skip).
    AlreadyProcessed,
}

/// Statistics about the outbox.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboxStats {
    /// Total events in outbox.
    pub total: u64,
    /// Pending publication.
    pub pending: u64,
    /// Published successfully.
    pub published: u64,
    /// Dead letter events.
    pub dead_letter: u64,
}

/// Idempotency key for deduplication.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Creates a new idempotency key.
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// Returns the raw key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Port for checking and recording idempotency keys.
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    /// Checks if a key exists and records it atomically.
    /// Returns true if this is the first time seeing the key.
    async fn try_acquire(&self, key: &IdempotencyKey, ttl: Duration) -> Result<bool, EventError>;

    /// Checks if a key exists without recording it.
    async fn exists(&self, key: &IdempotencyKey) -> Result<bool, EventError>;
}

/// Complete event bus combining outbox and publishing.
#[async_trait]
pub trait EventBus: OutboxWriter + EventPublisher + Send + Sync {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_payload_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct TestEvent {
            user_id: u64,
            action: String,
        }

        let original = TestEvent {
            user_id: 42,
            action: "login".to_string(),
        };
        let payload = EventPayload::new(&original).unwrap();
        let decoded: TestEvent = payload.decode().unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn idempotency_key_creation() {
        let key = IdempotencyKey::new("user-123-action-456");
        assert_eq!(key.as_str(), "user-123-action-456");
    }
}
