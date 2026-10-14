//! Explicit tenant context for Forge applications.
//!
//! There is no default or ambient tenant. A TenantContext can be created only
//! by combining a currently authenticated principal, an active membership for
//! the same principal, and an explicit successful RBAC decision.

use async_trait::async_trait;
use forge_auth::AuthenticatedPrincipal;
use forge_core::Id;
use forge_security::{
    AuthorizationError, AuthorizationGrant, Permission, PrincipalId, RbacPolicy, Role,
};
use thiserror::Error;

/// Marker type for a tenant identity.
#[derive(Debug)]
pub enum TenantMarker {}

/// UUIDv7 tenant identifier.
pub type TenantId = Id<TenantMarker>;

/// Membership lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipState {
    /// Membership can participate in authorization.
    Active,
    /// Membership is retained for history but must not authorize access.
    Suspended,
}

/// Tenant membership and tenant-scoped roles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Membership {
    tenant_id: TenantId,
    principal_id: PrincipalId,
    roles: Vec<Role>,
    state: MembershipState,
}

impl Membership {
    /// Creates an active tenant membership.
    #[must_use]
    pub fn active(tenant_id: TenantId, principal_id: PrincipalId, roles: Vec<Role>) -> Self {
        Self {
            tenant_id,
            principal_id,
            roles,
            state: MembershipState::Active,
        }
    }

    /// Restores a persisted membership with its current lifecycle state.
    #[must_use]
    pub fn restore(
        tenant_id: TenantId,
        principal_id: PrincipalId,
        roles: Vec<Role>,
        state: MembershipState,
    ) -> Self {
        Self {
            tenant_id,
            principal_id,
            roles,
            state,
        }
    }

    /// Tenant this membership belongs to.
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Principal this membership belongs to.
    #[must_use]
    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    /// Tenant-scoped roles used by authorization policy.
    #[must_use]
    pub fn roles(&self) -> &[Role] {
        &self.roles
    }

    /// Current membership state.
    #[must_use]
    pub const fn state(&self) -> MembershipState {
        self.state
    }

    /// Suspends this membership. Suspended memberships always fail closed.
    pub fn suspend(&mut self) {
        self.state = MembershipState::Suspended;
    }
}

/// Explicit tenant/authorization context passed to tenant-sensitive use cases.
///
/// Fields are private so callers cannot construct an authorized tenant context
/// by merely possessing a tenant UUID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TenantContext {
    tenant_id: TenantId,
    principal_id: PrincipalId,
    authorization: AuthorizationGrant,
}

impl TenantContext {
    /// Resolves and authorizes tenant scope.
    pub fn authorize(
        authenticated: AuthenticatedPrincipal,
        membership: &Membership,
        policy: &RbacPolicy,
        permission: &Permission,
    ) -> Result<Self, TenantContextError> {
        if membership.state != MembershipState::Active {
            return Err(TenantContextError::InactiveMembership);
        }
        if membership.principal_id != authenticated.principal_id() {
            return Err(TenantContextError::PrincipalMismatch);
        }

        let authorization = policy
            .authorize(authenticated.principal_id(), membership.roles(), permission)
            .map_err(TenantContextError::Authorization)?;

        Ok(Self {
            tenant_id: membership.tenant_id,
            principal_id: authenticated.principal_id(),
            authorization,
        })
    }

    /// Authorized tenant.
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Authorized principal.
    #[must_use]
    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    /// Policy grant that established this context.
    #[must_use]
    pub const fn authorization(&self) -> &AuthorizationGrant {
        &self.authorization
    }
}

/// Failure while deriving tenant context.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum TenantContextError {
    /// Membership is suspended or otherwise inactive.
    #[error("tenant membership is inactive")]
    InactiveMembership,
    /// Authenticated identity does not match the membership.
    #[error("authenticated principal does not match tenant membership")]
    PrincipalMismatch,
    /// Tenant-scoped RBAC policy denied the requested permission.
    #[error("tenant authorization denied")]
    Authorization(#[source] AuthorizationError),
}

/// Safe persistence error categories for tenant memberships.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipStoreErrorKind {
    /// Membership storage is unavailable.
    Unavailable,
    /// A concurrent membership transition prevented the requested write.
    Conflict,
    /// Persisted membership data violated Forge invariants.
    Corrupt,
}

/// Membership persistence failure with a bounded safe message.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct MembershipStoreError {
    kind: MembershipStoreErrorKind,
    message: &'static str,
    retryable: bool,
}

impl MembershipStoreError {
    /// Creates a classified membership storage error.
    #[must_use]
    pub const fn new(
        kind: MembershipStoreErrorKind,
        message: &'static str,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            message,
            retryable,
        }
    }

    /// Stable storage error category.
    #[must_use]
    pub const fn kind(&self) -> MembershipStoreErrorKind {
        self.kind
    }

    /// Whether a bounded retry may be reasonable.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

/// Persistence contract for tenant membership and current tenant-scoped roles.
///
/// Memberships are resolved independently from sessions so role changes and
/// suspension take effect on the next authorization decision.
#[async_trait]
pub trait MembershipStore: Send + Sync {
    /// Loads one principal's membership for one tenant.
    async fn find(
        &self,
        tenant_id: TenantId,
        principal_id: PrincipalId,
    ) -> Result<Option<Membership>, MembershipStoreError>;

    /// Inserts or replaces the current roles/state for a membership.
    async fn upsert(&self, membership: &Membership) -> Result<(), MembershipStoreError>;

    /// Idempotently suspends a membership.
    async fn suspend(
        &self,
        tenant_id: TenantId,
        principal_id: PrincipalId,
    ) -> Result<(), MembershipStoreError>;
}

/// Resolves the current membership and requires an explicit permission before
/// producing a tenant context for application use cases.
///
/// Memberships are loaded for every call so role changes and suspensions take
/// effect without waiting for the authenticated session to expire. A missing,
/// mismatched, inactive, or unauthorized membership is deliberately reported
/// as the same denial.
pub async fn authorize_tenant_context(
    store: &dyn MembershipStore,
    authenticated: AuthenticatedPrincipal,
    tenant_id: TenantId,
    policy: &RbacPolicy,
    permission: &Permission,
) -> Result<TenantContext, TenantAuthorizationError> {
    let membership = store
        .find(tenant_id, authenticated.principal_id())
        .await
        .map_err(TenantAuthorizationError::Store)?
        .ok_or(TenantAuthorizationError::Denied)?;

    if membership.tenant_id() != tenant_id
        || membership.principal_id() != authenticated.principal_id()
    {
        return Err(TenantAuthorizationError::Denied);
    }

    TenantContext::authorize(authenticated, &membership, policy, permission)
        .map_err(|_| TenantAuthorizationError::Denied)
}

/// Failure while resolving and authorizing a tenant membership.
#[derive(Debug, Error)]
pub enum TenantAuthorizationError {
    /// No current membership or the requested permission was not granted.
    #[error("tenant access denied")]
    Denied,
    /// Membership storage could not safely resolve the current authorization state.
    #[error(transparent)]
    Store(#[from] MembershipStoreError),
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use forge_auth::{Session, UnixTimestamp};
    use forge_security::{Permission, RbacPolicy, Role};

    use super::*;

    #[derive(Default)]
    struct MemoryMembershipStore {
        membership: Mutex<Option<Membership>>,
        fail: bool,
    }

    #[async_trait]
    impl MembershipStore for MemoryMembershipStore {
        async fn find(
            &self,
            tenant_id: TenantId,
            principal_id: PrincipalId,
        ) -> Result<Option<Membership>, MembershipStoreError> {
            if self.fail {
                return Err(MembershipStoreError::new(
                    MembershipStoreErrorKind::Unavailable,
                    "membership store unavailable",
                    true,
                ));
            }
            Ok(self
                .membership
                .lock()
                .map_err(|_| {
                    MembershipStoreError::new(
                        MembershipStoreErrorKind::Unavailable,
                        "membership store unavailable",
                        true,
                    )
                })?
                .as_ref()
                .filter(|membership| {
                    membership.tenant_id() == tenant_id && membership.principal_id() == principal_id
                })
                .cloned())
        }

        async fn upsert(&self, membership: &Membership) -> Result<(), MembershipStoreError> {
            *self.membership.lock().map_err(|_| {
                MembershipStoreError::new(
                    MembershipStoreErrorKind::Unavailable,
                    "membership store unavailable",
                    true,
                )
            })? = Some(membership.clone());
            Ok(())
        }

        async fn suspend(
            &self,
            tenant_id: TenantId,
            principal_id: PrincipalId,
        ) -> Result<(), MembershipStoreError> {
            if let Some(membership) = self
                .membership
                .lock()
                .map_err(|_| {
                    MembershipStoreError::new(
                        MembershipStoreErrorKind::Unavailable,
                        "membership store unavailable",
                        true,
                    )
                })?
                .as_mut()
                .filter(|membership| {
                    membership.tenant_id() == tenant_id && membership.principal_id() == principal_id
                })
            {
                membership.suspend();
            }
            Ok(())
        }
    }

    fn authenticated(principal: PrincipalId) -> AuthenticatedPrincipal {
        Session::new(
            principal,
            UnixTimestamp::from_secs(10),
            UnixTimestamp::from_secs(20),
        )
        .expect("valid session")
        .authenticate(UnixTimestamp::from_secs(11))
        .expect("session should authenticate")
    }

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should start")
    }

    #[test]
    fn context_requires_matching_identity_membership_and_permission() {
        let principal = PrincipalId::new();
        let tenant = TenantId::new();
        let admin = Role::new("admin").expect("valid role");
        let permission = Permission::new("users:read").expect("valid permission");
        let membership = Membership::active(tenant, principal, vec![admin.clone()]);
        let mut policy = RbacPolicy::new();
        policy.allow(admin, permission.clone());

        let context =
            TenantContext::authorize(authenticated(principal), &membership, &policy, &permission)
                .expect("authorized membership should create context");

        assert_eq!(context.tenant_id(), tenant);
        assert_eq!(context.principal_id(), principal);
        assert_eq!(context.authorization().permission(), &permission);
    }

    #[test]
    fn another_principal_cannot_reuse_membership() {
        let member = PrincipalId::new();
        let attacker = PrincipalId::new();
        let membership = Membership::active(
            TenantId::new(),
            member,
            vec![Role::new("admin").expect("valid role")],
        );
        let permission = Permission::new("users:read").expect("valid permission");
        let mut policy = RbacPolicy::new();
        policy.allow(Role::new("admin").expect("valid role"), permission.clone());

        assert_eq!(
            TenantContext::authorize(authenticated(attacker), &membership, &policy, &permission,),
            Err(TenantContextError::PrincipalMismatch)
        );
    }

    #[test]
    fn restored_membership_preserves_current_state() {
        let tenant = TenantId::new();
        let principal = PrincipalId::new();
        let role = Role::new("member").expect("valid role");

        let membership = Membership::restore(
            tenant,
            principal,
            vec![role.clone()],
            MembershipState::Suspended,
        );

        assert_eq!(membership.tenant_id(), tenant);
        assert_eq!(membership.principal_id(), principal);
        assert_eq!(membership.roles(), &[role]);
        assert_eq!(membership.state(), MembershipState::Suspended);
    }

    #[test]
    fn suspended_membership_and_missing_permission_deny() {
        let principal = PrincipalId::new();
        let permission = Permission::new("users:write").expect("valid permission");
        let mut membership = Membership::active(
            TenantId::new(),
            principal,
            vec![Role::new("member").expect("valid role")],
        );
        let policy = RbacPolicy::new();

        assert_eq!(
            TenantContext::authorize(authenticated(principal), &membership, &policy, &permission,),
            Err(TenantContextError::Authorization(
                AuthorizationError::Denied
            ))
        );

        membership.suspend();
        assert_eq!(
            TenantContext::authorize(authenticated(principal), &membership, &policy, &permission,),
            Err(TenantContextError::InactiveMembership)
        );
    }

    #[test]
    fn async_authorization_uses_current_membership_and_exact_permission() {
        let runtime = test_runtime();
        runtime.block_on(async {
            let principal = PrincipalId::new();
            let tenant = TenantId::new();
            let role = Role::new("owner").expect("valid role");
            let permission = Permission::new("projects:read").expect("valid permission");
            let store = MemoryMembershipStore::default();
            store
                .upsert(&Membership::active(tenant, principal, vec![role.clone()]))
                .await
                .expect("membership should persist");

            let mut policy = RbacPolicy::new();
            policy.allow(role, permission.clone());
            let context = authorize_tenant_context(
                &store,
                authenticated(principal),
                tenant,
                &policy,
                &permission,
            )
            .await
            .expect("current active membership should authorize");
            assert_eq!(context.tenant_id(), tenant);
            assert_eq!(context.authorization().permission(), &permission);

            store
                .suspend(tenant, principal)
                .await
                .expect("membership should suspend");
            assert!(matches!(
                authorize_tenant_context(
                    &store,
                    authenticated(principal),
                    tenant,
                    &policy,
                    &permission,
                )
                .await,
                Err(TenantAuthorizationError::Denied)
            ));
        });
    }

    #[test]
    fn async_authorization_denies_missing_membership_and_propagates_store_failure() {
        let runtime = test_runtime();
        runtime.block_on(async {
            let principal = PrincipalId::new();
            let tenant = TenantId::new();
            let permission = Permission::new("projects:read").expect("valid permission");
            let policy = RbacPolicy::new();

            assert!(matches!(
                authorize_tenant_context(
                    &MemoryMembershipStore::default(),
                    authenticated(principal),
                    tenant,
                    &policy,
                    &permission,
                )
                .await,
                Err(TenantAuthorizationError::Denied)
            ));

            let unavailable = MemoryMembershipStore {
                fail: true,
                ..MemoryMembershipStore::default()
            };
            assert!(matches!(
                authorize_tenant_context(
                    &unavailable,
                    authenticated(principal),
                    tenant,
                    &policy,
                    &permission,
                )
                .await,
                Err(TenantAuthorizationError::Store(_))
            ));
        });
    }
}
