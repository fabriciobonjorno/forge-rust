//! Framework-owned authorization primitives.
//!
//! Forge keeps authorization decisions explicit and fail-closed. Roles and
//! permissions are application-defined names; the framework provides validated
//! names, a small RBAC policy, and an authorization grant that can only be
//! produced by a successful policy decision.

use std::collections::{BTreeMap, BTreeSet};

use forge_core::Id;
use thiserror::Error;

/// Marker type for a principal identity.
#[derive(Debug)]
pub enum PrincipalMarker {}

/// UUIDv7 identity of an authenticated principal.
pub type PrincipalId = Id<PrincipalMarker>;

/// Validated tenant-scoped role name.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Role(String);

impl Role {
    /// Creates a role such as admin or billing_manager.
    pub fn new(value: impl Into<String>) -> Result<Self, SecurityNameError> {
        validate_name(value.into(), "role").map(Self)
    }

    /// Returns the stable role name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validated permission/capability name.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Permission(String);

impl Permission {
    /// Creates a permission such as users:read or invoice.approve.
    pub fn new(value: impl Into<String>) -> Result<Self, SecurityNameError> {
        validate_name(value.into(), "permission").map(Self)
    }

    /// Returns the stable permission name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_name(value: String, kind: &'static str) -> Result<String, SecurityNameError> {
    let valid = !value.is_empty()
        && value.len() <= 64
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
            .is_some_and(u8::is_ascii_alphanumeric);

    if valid {
        Ok(value)
    } else {
        Err(SecurityNameError { kind })
    }
}

/// Invalid role or permission name.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("invalid {kind} name")]
pub struct SecurityNameError {
    kind: &'static str,
}

/// Explicit deny-by-default RBAC policy.
#[derive(Clone, Debug, Default)]
pub struct RbacPolicy {
    grants: BTreeMap<Role, BTreeSet<Permission>>,
}

impl RbacPolicy {
    /// Creates an empty policy. Empty policies deny every authorization check.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Grants one permission to one role.
    pub fn allow(&mut self, role: Role, permission: Permission) {
        self.grants.entry(role).or_default().insert(permission);
    }

    /// Evaluates a principal's tenant-scoped roles for one permission.
    pub fn authorize(
        &self,
        principal_id: PrincipalId,
        roles: &[Role],
        permission: &Permission,
    ) -> Result<AuthorizationGrant, AuthorizationError> {
        let role = roles.iter().find(|role| {
            self.grants
                .get(*role)
                .is_some_and(|permissions| permissions.contains(permission))
        });

        match role {
            Some(role) => Ok(AuthorizationGrant {
                principal_id,
                role: role.clone(),
                permission: permission.clone(),
            }),
            None => Err(AuthorizationError::Denied),
        }
    }
}

/// Proof that an RBAC policy explicitly allowed one permission.
///
/// Fields are private so application code cannot manufacture a grant without a
/// successful policy evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationGrant {
    principal_id: PrincipalId,
    role: Role,
    permission: Permission,
}

impl AuthorizationGrant {
    /// Principal authorized by the policy.
    #[must_use]
    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    /// Role that satisfied the policy.
    #[must_use]
    pub fn role(&self) -> &Role {
        &self.role
    }

    /// Permission authorized by the policy.
    #[must_use]
    pub fn permission(&self) -> &Permission {
        &self.permission
    }
}

/// Authorization failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AuthorizationError {
    /// No role explicitly granted the requested permission.
    #[error("authorization denied")]
    Denied,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(value: &str) -> Role {
        Role::new(value).expect("fixture role should be valid")
    }

    fn permission(value: &str) -> Permission {
        Permission::new(value).expect("fixture permission should be valid")
    }

    #[test]
    fn empty_policy_denies_by_default() {
        let policy = RbacPolicy::new();
        let principal = PrincipalId::new();

        assert_eq!(
            policy.authorize(principal, &[role("admin")], &permission("users:read")),
            Err(AuthorizationError::Denied)
        );
    }

    #[test]
    fn exact_mapping_produces_grant() {
        let principal = PrincipalId::new();
        let admin = role("admin");
        let read_users = permission("users:read");
        let mut policy = RbacPolicy::new();
        policy.allow(admin.clone(), read_users.clone());

        let grant = policy
            .authorize(principal, &[role("member"), admin.clone()], &read_users)
            .expect("admin should be authorized");

        assert_eq!(grant.principal_id(), principal);
        assert_eq!(grant.role(), &admin);
        assert_eq!(grant.permission(), &read_users);
    }

    #[test]
    fn names_are_strict_and_not_normalized() {
        for invalid in [
            "",
            "Admin",
            " users:read",
            "users/read",
            "users:read:",
            "a b",
        ] {
            assert!(
                Role::new(invalid).is_err(),
                "accepted invalid role {invalid:?}"
            );
            assert!(
                Permission::new(invalid).is_err(),
                "accepted invalid permission {invalid:?}"
            );
        }

        assert_eq!(
            Permission::new("invoice.approve")
                .expect("valid permission")
                .as_str(),
            "invoice.approve"
        );
    }
}
