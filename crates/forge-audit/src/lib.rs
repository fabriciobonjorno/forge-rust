//! Immutable security-audit contracts.
//!
//! Audit events are structured evidence, not application log strings. Forge
//! keeps the event immutable after construction and exposes an append-only sink
//! contract. A sink failure remains explicit so privileged callers can fail
//! closed instead of silently continuing without audit evidence.

use async_trait::async_trait;
use forge_auth::UnixTimestamp;
use forge_core::Id;
use forge_security::PrincipalId;
use forge_tenancy::TenantId;
use thiserror::Error;

/// Marker type for an audit event identity.
#[derive(Debug)]
pub enum AuditEventMarker {}

/// UUIDv7 audit event identifier.
pub type AuditEventId = Id<AuditEventMarker>;

/// Actor attributable to an audit event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditActor {
    /// No authenticated identity was established.
    Anonymous,
    /// An authenticated principal performed or attempted the action.
    Principal(PrincipalId),
    /// A framework/system process performed the action.
    System,
}

/// Result recorded for an audited action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    /// Authorization or policy allowed an action.
    Allowed,
    /// Authorization or policy denied an action.
    Denied,
    /// An allowed action completed successfully.
    Succeeded,
    /// An allowed action was attempted but failed.
    Failed,
}

/// Validated stable audit action name.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AuditAction(String);

impl AuditAction {
    /// Creates an action such as auth.login, session.revoke, or invoice:approve.
    pub fn new(value: impl Into<String>) -> Result<Self, AuditValueError> {
        let value = value.into();
        if valid_name(&value, 96) {
            Ok(Self(value))
        } else {
            Err(AuditValueError::InvalidAction)
        }
    }

    /// Returns the stable action name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bounded correlation/link value such as an HTTP request ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditLink(String);

impl AuditLink {
    /// Creates a bounded ASCII linkage value.
    pub fn new(value: impl Into<String>) -> Result<Self, AuditValueError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 128
            && value.is_ascii()
            && !value.bytes().any(|byte| byte.is_ascii_control());
        if valid {
            Ok(Self(value))
        } else {
            Err(AuditValueError::InvalidLink)
        }
    }

    /// Returns the linkage value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn valid_name(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.is_ascii()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b':' | b'.' | b'_' | b'-')
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

/// Invalid audit event field.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AuditValueError {
    /// Action name violates the stable action grammar.
    #[error("invalid audit action")]
    InvalidAction,
    /// Correlation/link value is empty, too long, non-ASCII, or contains controls.
    #[error("invalid audit linkage value")]
    InvalidLink,
}

/// Immutable structured audit event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    id: AuditEventId,
    occurred_at: UnixTimestamp,
    actor: AuditActor,
    tenant_id: Option<TenantId>,
    action: AuditAction,
    outcome: AuditOutcome,
    request_link: Option<AuditLink>,
}

impl AuditEvent {
    /// Creates a new immutable event.
    #[must_use]
    pub fn new(
        occurred_at: UnixTimestamp,
        actor: AuditActor,
        tenant_id: Option<TenantId>,
        action: AuditAction,
        outcome: AuditOutcome,
    ) -> Self {
        Self {
            id: AuditEventId::new(),
            occurred_at,
            actor,
            tenant_id,
            action,
            outcome,
            request_link: None,
        }
    }

    /// Creates an event whose actor and tenant are derived from an authorized
    /// TenantContext instead of caller-provided identifiers.
    #[must_use]
    pub fn from_tenant_context(
        occurred_at: UnixTimestamp,
        context: &forge_tenancy::TenantContext,
        action: AuditAction,
        outcome: AuditOutcome,
    ) -> Self {
        Self::new(
            occurred_at,
            AuditActor::Principal(context.principal_id()),
            Some(context.tenant_id()),
            action,
            outcome,
        )
    }

    /// Adds a validated request/correlation linkage before persistence.
    #[must_use]
    pub fn with_request_link(mut self, link: AuditLink) -> Self {
        self.request_link = Some(link);
        self
    }

    /// Event identifier.
    #[must_use]
    pub const fn id(&self) -> AuditEventId {
        self.id
    }

    /// Trusted event timestamp.
    #[must_use]
    pub const fn occurred_at(&self) -> UnixTimestamp {
        self.occurred_at
    }

    /// Attributed actor.
    #[must_use]
    pub const fn actor(&self) -> AuditActor {
        self.actor
    }

    /// Tenant scope when the event is tenant-specific.
    #[must_use]
    pub const fn tenant_id(&self) -> Option<TenantId> {
        self.tenant_id
    }

    /// Stable action identifier.
    #[must_use]
    pub fn action(&self) -> &AuditAction {
        &self.action
    }

    /// Recorded outcome.
    #[must_use]
    pub const fn outcome(&self) -> AuditOutcome {
        self.outcome
    }

    /// Optional request/correlation linkage.
    #[must_use]
    pub const fn request_link(&self) -> Option<&AuditLink> {
        self.request_link.as_ref()
    }
}

/// Safe audit failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditErrorKind {
    /// Durable audit storage is unavailable.
    Unavailable,
    /// A bounded audit queue/buffer is full.
    Capacity,
    /// The sink rejected the event contract.
    Rejected,
    /// Existing audit integrity evidence could not be trusted.
    Integrity,
}

/// Audit persistence failure with a bounded safe message.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct AuditError {
    kind: AuditErrorKind,
    message: &'static str,
    retryable: bool,
}

impl AuditError {
    /// Creates a safe classified audit error.
    #[must_use]
    pub const fn new(kind: AuditErrorKind, message: &'static str, retryable: bool) -> Self {
        Self {
            kind,
            message,
            retryable,
        }
    }

    /// Stable audit error category.
    #[must_use]
    pub const fn kind(&self) -> AuditErrorKind {
        self.kind
    }

    /// Whether a bounded retry may be reasonable.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

/// Receipt proving that a sink accepted an event for durable audit storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuditReceipt {
    event_id: AuditEventId,
}

impl AuditReceipt {
    /// Constructs a receipt for an event persisted by an AuditSink implementation.
    #[must_use]
    pub const fn stored(event_id: AuditEventId) -> Self {
        Self { event_id }
    }

    /// Event accepted by the sink.
    #[must_use]
    pub const fn event_id(self) -> AuditEventId {
        self.event_id
    }
}

/// Append-only durable audit sink.
///
/// Implementations must never update or delete an event through this contract.
/// Retention, integrity chaining, archival and administrative reads belong to
/// infrastructure-specific controls outside this write-only application port.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Durably appends one immutable audit event.
    async fn append(&self, event: &AuditEvent) -> Result<AuditReceipt, AuditError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_names_are_strict_and_bounded() {
        for valid in [
            "auth.login",
            "session.revoke",
            "invoice:approve",
            "user_2fa.enable",
        ] {
            assert_eq!(
                AuditAction::new(valid)
                    .expect("fixture action should be valid")
                    .as_str(),
                valid
            );
        }

        for invalid in [
            "",
            "Auth.Login",
            " auth.login",
            "auth/login",
            "auth.login.",
            "a b",
        ] {
            assert_eq!(
                AuditAction::new(invalid),
                Err(AuditValueError::InvalidAction),
                "accepted invalid action {invalid:?}"
            );
        }
    }

    #[test]
    fn linkage_rejects_controls_and_unbounded_values() {
        assert!(AuditLink::new("01941f29-7c00-7000-8000-000000000001").is_ok());
        assert_eq!(AuditLink::new(""), Err(AuditValueError::InvalidLink));
        assert_eq!(
            AuditLink::new("request\nsmuggle"),
            Err(AuditValueError::InvalidLink)
        );
        assert_eq!(
            AuditLink::new("x".repeat(129)),
            Err(AuditValueError::InvalidLink)
        );
    }

    #[test]
    fn event_keeps_security_attribution_immutable() {
        let principal = PrincipalId::new();
        let tenant = TenantId::new();
        let action = AuditAction::new("invoice:approve").expect("valid action");
        let link = AuditLink::new("request-123").expect("valid link");

        let event = AuditEvent::new(
            UnixTimestamp::from_secs(42),
            AuditActor::Principal(principal),
            Some(tenant),
            action.clone(),
            AuditOutcome::Succeeded,
        )
        .with_request_link(link);

        assert_eq!(event.actor(), AuditActor::Principal(principal));
        assert_eq!(event.tenant_id(), Some(tenant));
        assert_eq!(event.action(), &action);
        assert_eq!(event.outcome(), AuditOutcome::Succeeded);
        assert_eq!(event.occurred_at().as_secs(), 42);
        assert_eq!(
            event.request_link().expect("request link").as_str(),
            "request-123"
        );
    }

    #[test]
    fn tenant_context_controls_event_attribution() {
        use forge_auth::Session;
        use forge_security::{Permission, RbacPolicy, Role};
        use forge_tenancy::{Membership, TenantContext};

        let principal = PrincipalId::new();
        let tenant = TenantId::new();
        let permission = Permission::new("invoice:approve").expect("valid permission");
        let role = Role::new("approver").expect("valid role");
        let membership = Membership::active(tenant, principal, vec![role.clone()]);
        let mut policy = RbacPolicy::new();
        policy.allow(role, permission.clone());
        let authenticated = Session::new(
            principal,
            UnixTimestamp::from_secs(10),
            UnixTimestamp::from_secs(20),
        )
        .expect("valid session")
        .authenticate(UnixTimestamp::from_secs(11))
        .expect("valid authentication");
        let context = TenantContext::authorize(authenticated, &membership, &policy, &permission)
            .expect("authorized tenant context");

        let event = AuditEvent::from_tenant_context(
            UnixTimestamp::from_secs(12),
            &context,
            AuditAction::new("invoice:approve").expect("valid action"),
            AuditOutcome::Succeeded,
        );

        assert_eq!(event.actor(), AuditActor::Principal(principal));
        assert_eq!(event.tenant_id(), Some(tenant));
    }

    #[test]
    fn audit_errors_are_classified_without_internal_details() {
        let error = AuditError::new(AuditErrorKind::Unavailable, "audit sink unavailable", true);

        assert_eq!(error.kind(), AuditErrorKind::Unavailable);
        assert!(error.retryable());
        assert_eq!(error.to_string(), "audit sink unavailable");
    }
}
