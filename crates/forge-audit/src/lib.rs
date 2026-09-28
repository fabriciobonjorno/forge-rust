//! Append-only audit event contracts.
//!
//! Audit records are deliberately small and structured. Arbitrary request
//! bodies, credentials and secret-bearing metadata are not part of the stable
//! event contract. Concrete sinks may enrich records only with separately
//! reviewed, classified fields.

use async_trait::async_trait;
use forge_auth::{AuthenticatedPrincipal, UnixTimestamp};
use forge_core::Id;
use forge_security::PrincipalId;
use forge_tenancy::{TenantContext, TenantId};
use thiserror::Error;

/// Marker type for an audit event identity.
#[derive(Debug)]
pub enum AuditEventMarker {}

/// UUIDv7 audit event identifier.
pub type AuditEventId = Id<AuditEventMarker>;

/// Validated stable audit action, such as session.create or users:invite.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AuditAction(String);

impl AuditAction {
    /// Creates an action identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, AuditValidationError> {
        validate_label(value.into(), "action", 96).map(Self)
    }

    /// Returns the action identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bounded request/correlation identifier attached to an audit event.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AuditRequestId(String);

impl AuditRequestId {
    /// Creates a request identifier. Forge HTTP request IDs are UUIDv7 strings.
    pub fn new(value: impl Into<String>) -> Result<Self, AuditValidationError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 96
            && value.is_ascii()
            && !value.bytes().any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());
        if valid {
            Ok(Self(value))
        } else {
            Err(AuditValidationError { kind: "request id" })
        }
    }

    /// Returns the request identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Optional resource type affected by an audited action.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResourceKind(String);

impl ResourceKind {
    /// Creates a resource kind such as session or tenant_membership.
    pub fn new(value: impl Into<String>) -> Result<Self, AuditValidationError> {
        validate_label(value.into(), "resource kind", 64).map(Self)
    }

    /// Returns the resource kind.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_label(
    value: String,
    kind: &'static str,
    max_len: usize,
) -> Result<String, AuditValidationError> {
    let valid = !value.is_empty()
        && value.len() <= max_len
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b':' | b'.' | b'_' | b'-'))
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric);

    if valid {
        Ok(value)
    } else {
        Err(AuditValidationError { kind })
    }
}

/// Result recorded for an audited operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    /// Policy or authentication denied the operation.
    Denied,
    /// Authorized operation completed successfully.
    Succeeded,
    /// Authorized operation started but failed.
    Failed,
}

/// Stable target reference without embedding resource contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditTarget {
    kind: ResourceKind,
    id: Option<String>,
}

impl AuditTarget {
    /// Creates a target with an optional bounded opaque identifier.
    pub fn new(
        kind: ResourceKind,
        id: Option<String>,
    ) -> Result<Self, AuditValidationError> {
        if id.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > 200
                || value.bytes().any(|byte| byte.is_ascii_control())
        }) {
            return Err(AuditValidationError { kind: "target id" });
        }
        Ok(Self { kind, id })
    }

    /// Target resource kind.
    #[must_use]
    pub fn kind(&self) -> &ResourceKind {
        &self.kind
    }

    /// Optional opaque target identifier.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

/// One append-only audit record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    id: AuditEventId,
    occurred_at: UnixTimestamp,
    principal_id: Option<PrincipalId>,
    tenant_id: Option<TenantId>,
    action: AuditAction,
    outcome: AuditOutcome,
    target: Option<AuditTarget>,
    request_id: Option<AuditRequestId>,
}

impl AuditEvent {
    /// Creates an event not yet associated with authenticated identity.
    #[must_use]
    pub fn anonymous(
        occurred_at: UnixTimestamp,
        action: AuditAction,
        outcome: AuditOutcome,
        target: Option<AuditTarget>,
    ) -> Self {
        Self {
            id: AuditEventId::new(),
            occurred_at,
            principal_id: None,
            tenant_id: None,
            action,
            outcome,
            target,
            request_id: None,
        }
    }

    /// Creates an event associated with an authenticated principal.
    #[must_use]
    pub fn for_principal(
        occurred_at: UnixTimestamp,
        authenticated: AuthenticatedPrincipal,
        action: AuditAction,
        outcome: AuditOutcome,
        target: Option<AuditTarget>,
    ) -> Self {
        Self {
            id: AuditEventId::new(),
            occurred_at,
            principal_id: Some(authenticated.principal_id()),
            tenant_id: None,
            action,
            outcome,
            target,
            request_id: None,
        }
    }

    /// Creates an event from an authorized tenant context.
    #[must_use]
    pub fn for_tenant(
        occurred_at: UnixTimestamp,
        context: &TenantContext,
        action: AuditAction,
        outcome: AuditOutcome,
        target: Option<AuditTarget>,
    ) -> Self {
        Self {
            id: AuditEventId::new(),
            occurred_at,
            principal_id: Some(context.principal_id()),
            tenant_id: Some(context.tenant_id()),
            action,
            outcome,
            target,
            request_id: None,
        }
    }

    /// Audit event ID.
    #[must_use]
    pub const fn id(&self) -> AuditEventId {
        self.id
    }

    /// Trusted event timestamp.
    #[must_use]
    pub const fn occurred_at(&self) -> UnixTimestamp {
        self.occurred_at
    }

    /// Authenticated actor, when known.
    #[must_use]
    pub const fn principal_id(&self) -> Option<PrincipalId> {
        self.principal_id
    }

    /// Authorized tenant scope, when present.
    #[must_use]
    pub const fn tenant_id(&self) -> Option<TenantId> {
        self.tenant_id
    }

    /// Stable action.
    #[must_use]
    pub fn action(&self) -> &AuditAction {
        &self.action
    }

    /// Operation result.
    #[must_use]
    pub const fn outcome(&self) -> AuditOutcome {
        self.outcome
    }

    /// Optional resource target.
    #[must_use]
    pub const fn target(&self) -> Option<&AuditTarget> {
        self.target.as_ref()
    }

    /// Attaches the request/correlation identifier observed at the transport boundary.
    #[must_use]
    pub fn with_request_id(mut self, request_id: AuditRequestId) -> Self {
        self.request_id = Some(request_id);
        self
    }

    /// Request/correlation identifier, when available.
    #[must_use]
    pub const fn request_id(&self) -> Option<&AuditRequestId> {
        self.request_id.as_ref()
    }
}

/// Append-only audit sink.
///
/// Implementations must never translate append into update/upsert semantics.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Appends exactly one event.
    async fn append(&self, event: &AuditEvent) -> Result<(), AuditError>;
}

/// Safe audit persistence failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AuditError {
    /// Durable sink is temporarily unavailable.
    #[error("audit sink unavailable")]
    Unavailable,
    /// Event violates sink-side integrity constraints.
    #[error("audit event rejected")]
    Rejected,
}

/// Invalid audit label or target.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("invalid {kind}")]
pub struct AuditValidationError {
    kind: &'static str,
}

#[cfg(test)]
mod tests {
    use forge_auth::Session;
    use forge_security::{Permission, RbacPolicy, Role};
    use forge_tenancy::{Membership, TenantContext};

    use super::*;

    fn time(seconds: u64) -> UnixTimestamp {
        UnixTimestamp::from_secs(seconds)
    }

    #[test]
    fn labels_are_strict_and_bounded() {
        assert_eq!(
            AuditAction::new("session.create")
                .expect("valid action")
                .as_str(),
            "session.create"
        );

        for invalid in ["", "Session.Create", "session/create", " session.create", "a b"] {
            assert!(AuditAction::new(invalid).is_err(), "accepted {invalid:?}");
        }

        assert!(AuditAction::new("a".repeat(97)).is_err());
    }

    #[test]
    fn target_rejects_control_characters_and_large_ids() {
        let kind = ResourceKind::new("session").expect("valid kind");
        assert!(AuditTarget::new(kind.clone(), Some("id\nsecret".to_owned())).is_err());
        assert!(AuditTarget::new(kind, Some("x".repeat(201))).is_err());
    }

    #[test]
    fn request_id_is_bounded_and_attached_explicitly() {
        assert!(AuditRequestId::new("bad request id").is_err());
        let request_id = AuditRequestId::new("01941f29-7c00-7000-8000-000000000010")
            .expect("UUID-shaped request id should be accepted");
        let event = AuditEvent::anonymous(
            time(1),
            AuditAction::new("login.denied").expect("valid action"),
            AuditOutcome::Denied,
            None,
        )
        .with_request_id(request_id.clone());

        assert_eq!(event.request_id(), Some(&request_id));
    }

    #[test]
    fn tenant_event_derives_actor_and_tenant_from_authorized_context() {
        let principal = PrincipalId::new();
        let tenant = TenantId::new();
        let session = Session::new(principal, time(10), time(30)).expect("valid session");
        let authenticated = session.authenticate(time(11)).expect("authenticated");
        let admin = Role::new("admin").expect("valid role");
        let permission = Permission::new("audit:write").expect("valid permission");
        let membership = Membership::active(tenant, principal, vec![admin.clone()]);
        let mut policy = RbacPolicy::new();
        policy.allow(admin, permission.clone());
        let context = TenantContext::authorize(
            authenticated,
            &membership,
            &policy,
            &permission,
        )
        .expect("authorized context");

        let event = AuditEvent::for_tenant(
            time(12),
            &context,
            AuditAction::new("session.revoke").expect("valid action"),
            AuditOutcome::Succeeded,
            None,
        );

        assert_eq!(event.principal_id(), Some(principal));
        assert_eq!(event.tenant_id(), Some(tenant));
    }
}
