//! Job queue contracts for Forge applications.
//!
//! This crate defines the job queue abstraction. The PostgreSQL implementation
//! is provided by the infrastructure layer in generated applications.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use forge_core::Id;
use forge_tenancy::TenantId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A job identifier.
pub type JobId = Id<JobMarker>;

/// Marker type for job identifiers.
pub enum JobMarker {}

/// A job's payload — opaque to the queue, meaningful to the handler.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobPayload(pub Vec<u8>);

impl JobPayload {
    /// Creates a new job payload from JSON-serializable data.
    pub fn new<T: Serialize>(value: &T) -> Result<Self, JobError> {
        serde_json::to_vec(value)
            .map(Self)
            .map_err(|e| JobError::Serialization(e.to_string()))
    }

    /// Decodes the payload as JSON.
    pub fn decode<T: for<'de> Deserialize<'de>>(&self) -> Result<T, JobError> {
        serde_json::from_slice(&self.0).map_err(|e| JobError::Deserialization(e.to_string()))
    }
}

/// Job status in the queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Waiting to be claimed.
    Pending,
    /// Currently being processed.
    Running,
    /// Completed successfully.
    Completed,
    /// Failed permanently after exhausting retries.
    DeadLetter,
    /// Scheduled for future execution.
    Scheduled,
}

/// Job priority — higher values are claimed first.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct JobPriority(i16);

impl JobPriority {
    /// Default priority for ordinary jobs.
    pub const DEFAULT: Self = Self(0);
    /// Maximum priority.
    pub const MAX: Self = Self(i16::MAX);
    /// Minimum priority.
    pub const MIN: Self = Self(i16::MIN);

    /// Creates a new priority in the valid range.
    pub fn new(value: i16) -> Self {
        Self(value.clamp(Self::MIN.0, Self::MAX.0))
    }

    /// Returns the raw priority value.
    pub const fn get(self) -> i16 {
        self.0
    }
}

impl Default for JobPriority {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Job queue configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobQueueConfig {
    /// Maximum number of retry attempts before dead-lettering.
    pub max_attempts: u32,
    /// Base delay for exponential backoff (milliseconds).
    pub base_delay_ms: u64,
    /// Maximum delay between retries (milliseconds).
    pub max_delay_ms: u64,
    /// Jitter factor (0.0 to 1.0) to prevent thundering herd.
    pub jitter_factor: f64,
    /// Concurrency key for fair scheduling (optional).
    pub concurrency_key: Option<String>,
    /// Claim timeout after which the job becomes available again (seconds).
    pub claim_timeout_secs: u64,
}

impl Default for JobQueueConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 1000,
            max_delay_ms: 300_000,
            jitter_factor: 0.1,
            concurrency_key: None,
            claim_timeout_secs: 300,
        }
    }
}

/// A job enqueued for execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    /// Unique job identifier.
    pub id: JobId,
    /// Tenant that owns this job (for multi-tenant queues).
    pub tenant_id: Option<TenantId>,
    /// Job type/handler name.
    pub kind: String,
    /// Opaque payload.
    pub payload: JobPayload,
    /// Current status.
    pub status: JobStatus,
    /// Priority — higher values claimed first.
    pub priority: JobPriority,
    /// Number of attempts made.
    pub attempts: u32,
    /// Maximum attempts allowed.
    pub max_attempts: u32,
    /// Scheduled execution time (None = immediate).
    pub scheduled_at: Option<DateTime<Utc>>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job was last updated.
    pub updated_at: DateTime<Utc>,
    /// Last error message (if any).
    pub last_error: Option<String>,
    /// Concurrency key for fair scheduling.
    pub concurrency_key: Option<String>,
    /// Claim timeout.
    pub claimed_at: Option<DateTime<Utc>>,
    /// Claimed by worker ID.
    pub claimed_by: Option<String>,
}

/// Job queue errors.
#[derive(Debug, Error)]
pub enum JobError {
    /// Job not found.
    #[error("job not found: {0}")]
    NotFound(JobId),
    /// Job already claimed by another worker.
    #[error("job already claimed")]
    AlreadyClaimed,
    /// Job is in a terminal state and cannot be modified.
    #[error("job is in terminal state: {0:?}")]
    TerminalState(JobStatus),
    /// Maximum retry attempts exceeded.
    #[error("max attempts exceeded: {0}")]
    MaxAttemptsExceeded(u32),
    /// Concurrency key conflict.
    #[error("concurrency key conflict: {0}")]
    ConcurrencyConflict(String),
    /// Serialization error.
    #[error("serialization failed: {0}")]
    Serialization(String),
    /// Deserialization error.
    #[error("deserialization failed: {0}")]
    Deserialization(String),
    /// Database error.
    #[error(transparent)]
    Database(#[from] forge_db::DbError),
    /// Configuration error.
    #[error("configuration error: {0}")]
    Configuration(String),
    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl JobError {
    /// Returns whether this error is retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            JobError::Database(e) if e.is_retryable() => true,
            JobError::AlreadyClaimed => true,
            JobError::ConcurrencyConflict(_) => true,
            _ => false,
        }
    }
}

/// Port for enqueueing jobs.
#[async_trait]
pub trait JobEnqueuer: Send + Sync {
    /// Enqueues a new job for immediate execution.
    async fn enqueue(&self, job: Job) -> Result<JobId, JobError>;

    /// Enqueues a new job for scheduled execution.
    async fn enqueue_at(&self, job: Job, scheduled_at: DateTime<Utc>) -> Result<JobId, JobError>;

    /// Enqueues multiple jobs atomically.
    async fn enqueue_batch(&self, jobs: Vec<Job>) -> Result<Vec<JobId>, JobError>;
}

/// Port for claiming and completing jobs.
#[async_trait]
pub trait JobWorker: Send + Sync {
    /// Claims the next available job matching the worker's capabilities.
    /// Returns None if no jobs are available.
    async fn claim(
        &self,
        worker_id: &str,
        kinds: &[String],
        limit: usize,
    ) -> Result<Vec<Job>, JobError>;

    /// Marks a job as completed successfully.
    async fn complete(&self, job_id: &JobId, worker_id: &str) -> Result<(), JobError>;

    /// Marks a job as failed, scheduling a retry if attempts remain.
    async fn fail(&self, job_id: &JobId, worker_id: &str, error: &str) -> Result<(), JobError>;

    /// Releases a claim without completing or failing (returns to pending).
    async fn release(&self, job_id: &JobId, worker_id: &str) -> Result<(), JobError>;

    /// Extends the claim timeout for a running job.
    async fn heartbeat(&self, job_id: &JobId, worker_id: &str) -> Result<(), JobError>;
}

/// Port for administrative job operations.
#[async_trait]
pub trait JobAdmin: Send + Sync {
    /// Gets a job by ID.
    async fn get(&self, job_id: &JobId) -> Result<Option<Job>, JobError>;

    /// Lists jobs with filters.
    async fn list(
        &self,
        filter: JobFilter,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Job>, JobError>;

    /// Cancels a pending or scheduled job.
    async fn cancel(&self, job_id: &JobId) -> Result<(), JobError>;

    /// Requeues a dead-letter job for retry.
    async fn requeue(&self, job_id: &JobId) -> Result<(), JobError>;

    /// Gets queue statistics.
    async fn stats(&self) -> Result<QueueStats, JobError>;

    /// Purges completed jobs older than the given timestamp.
    async fn purge_completed(&self, older_than: DateTime<Utc>) -> Result<u64, JobError>;
}

/// Filter for listing jobs.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct JobFilter {
    /// Filter by tenant.
    pub tenant_id: Option<TenantId>,
    /// Filter by job kind.
    pub kind: Option<String>,
    /// Filter by status.
    pub status: Option<JobStatus>,
    /// Filter by concurrency key.
    pub concurrency_key: Option<String>,
    /// Filter by scheduled before.
    pub scheduled_before: Option<DateTime<Utc>>,
    /// Filter by created after.
    pub created_after: Option<DateTime<Utc>>,
}

/// Queue statistics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueStats {
    /// Total jobs in queue.
    pub total: u64,
    /// Pending jobs.
    pub pending: u64,
    /// Running jobs.
    pub running: u64,
    /// Completed jobs.
    pub completed: u64,
    /// Dead letter jobs.
    pub dead_letter: u64,
    /// Scheduled jobs.
    pub scheduled: u64,
}

/// Combined job queue port.
#[async_trait]
pub trait JobQueue: JobEnqueuer + JobWorker + JobAdmin + Send + Sync {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_priority_clamping() {
        assert_eq!(JobPriority::new(i16::MAX).get(), i16::MAX);
        assert_eq!(JobPriority::new(i16::MIN).get(), i16::MIN);
        assert_eq!(JobPriority::new(100).get(), 100);
    }

    #[test]
    fn job_payload_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct TestPayload {
            value: u64,
            name: String,
        }

        let original = TestPayload {
            value: 42,
            name: "test".to_string(),
        };
        let payload = JobPayload::new(&original).unwrap();
        let decoded: TestPayload = payload.decode().unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn job_queue_config_defaults() {
        let config = JobQueueConfig::default();
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.base_delay_ms, 1000);
        assert_eq!(config.claim_timeout_secs, 300);
    }
}
