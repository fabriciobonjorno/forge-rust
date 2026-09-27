//! Authentication and server-side session contracts.
//!
//! This crate models session lifecycle and authenticated identity. It does not
//! implement password hashing, cookie signing, or bearer-token cryptography;
//! those are adapter mechanisms that require separate dependency/security
//! review. A session proves identity only. Tenant roles are resolved separately
//! so authorization changes do not remain stale inside credentials.

use forge_core::Id;
use forge_security::PrincipalId;
use thiserror::Error;

/// Marker type for a session identity.
#[derive(Debug)]
pub enum SessionMarker {}

/// UUIDv7 identifier for a server-side session record.
pub type SessionId = Id<SessionMarker>;

/// UTC Unix timestamp in whole seconds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct UnixTimestamp(u64);

impl UnixTimestamp {
    /// Creates a timestamp from seconds since the Unix epoch.
    #[must_use]
    pub const fn from_secs(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Returns seconds since the Unix epoch.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.0
    }
}

/// Server-side session record.
///
/// The record deliberately contains no bearer secret. Concrete adapters store
/// only an appropriate verifier/digest for credentials presented by clients.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    id: SessionId,
    principal_id: PrincipalId,
    issued_at: UnixTimestamp,
    expires_at: UnixTimestamp,
    revoked_at: Option<UnixTimestamp>,
}

impl Session {
    /// Creates an active session with a strictly later expiry.
    pub fn new(
        principal_id: PrincipalId,
        issued_at: UnixTimestamp,
        expires_at: UnixTimestamp,
    ) -> Result<Self, SessionError> {
        if expires_at <= issued_at {
            return Err(SessionError::InvalidLifetime);
        }

        Ok(Self {
            id: SessionId::new(),
            principal_id,
            issued_at,
            expires_at,
            revoked_at: None,
        })
    }

    /// Session identifier.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// Principal identity bound to this session.
    #[must_use]
    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    /// Timestamp at which the session was issued.
    #[must_use]
    pub const fn issued_at(&self) -> UnixTimestamp {
        self.issued_at
    }

    /// Timestamp after which the session must not authenticate.
    #[must_use]
    pub const fn expires_at(&self) -> UnixTimestamp {
        self.expires_at
    }

    /// Revocation timestamp, when revoked.
    #[must_use]
    pub const fn revoked_at(&self) -> Option<UnixTimestamp> {
        self.revoked_at
    }

    /// Authenticates this session at a trusted current time.
    ///
    /// Expiry is exclusive: authentication exactly at expires_at is denied.
    pub fn authenticate(
        &self,
        now: UnixTimestamp,
    ) -> Result<AuthenticatedPrincipal, SessionError> {
        if self.revoked_at.is_some() {
            return Err(SessionError::Revoked);
        }
        if now < self.issued_at {
            return Err(SessionError::NotYetValid);
        }
        if now >= self.expires_at {
            return Err(SessionError::Expired);
        }

        Ok(AuthenticatedPrincipal {
            principal_id: self.principal_id,
            session_id: self.id,
            authenticated_at: now,
        })
    }

    /// Revokes this session. Revocation is idempotent and keeps the earliest
    /// revocation timestamp.
    pub fn revoke(&mut self, now: UnixTimestamp) {
        if self.revoked_at.is_none() {
            self.revoked_at = Some(now);
        }
    }

    /// Rotates to a new session and revokes this one.
    pub fn rotate(
        &mut self,
        now: UnixTimestamp,
        new_expires_at: UnixTimestamp,
    ) -> Result<Self, SessionError> {
        self.authenticate(now)?;
        let replacement = Self::new(self.principal_id, now, new_expires_at)?;
        self.revoke(now);
        Ok(replacement)
    }
}

/// Identity proven by a currently valid server-side session.
///
/// Fields are private; callers obtain this proof only through Session::authenticate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthenticatedPrincipal {
    principal_id: PrincipalId,
    session_id: SessionId,
    authenticated_at: UnixTimestamp,
}

impl AuthenticatedPrincipal {
    /// Authenticated principal identifier.
    #[must_use]
    pub const fn principal_id(self) -> PrincipalId {
        self.principal_id
    }

    /// Session that established the identity.
    #[must_use]
    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    /// Trusted time at which the session was evaluated.
    #[must_use]
    pub const fn authenticated_at(self) -> UnixTimestamp {
        self.authenticated_at
    }
}

/// Session validation failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SessionError {
    /// Expiry was not strictly later than issuance.
    #[error("invalid session lifetime")]
    InvalidLifetime,
    /// Current time predates issuance.
    #[error("session is not yet valid")]
    NotYetValid,
    /// Session reached its expiry.
    #[error("session expired")]
    Expired,
    /// Session was revoked.
    #[error("session revoked")]
    Revoked,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(seconds: u64) -> UnixTimestamp {
        UnixTimestamp::from_secs(seconds)
    }

    #[test]
    fn lifetime_is_fail_closed() {
        let principal = PrincipalId::new();
        assert_eq!(
            Session::new(principal, time(10), time(10)),
            Err(SessionError::InvalidLifetime)
        );

        let session = Session::new(principal, time(10), time(20)).expect("valid session");
        assert_eq!(
            session.authenticate(time(9)),
            Err(SessionError::NotYetValid)
        );
        assert!(session.authenticate(time(10)).is_ok());
        assert!(session.authenticate(time(19)).is_ok());
        assert_eq!(session.authenticate(time(20)), Err(SessionError::Expired));
    }

    #[test]
    fn revocation_denies_and_rotation_changes_session_id() {
        let principal = PrincipalId::new();
        let mut session = Session::new(principal, time(10), time(30)).expect("valid session");
        let original_id = session.id();

        let replacement = session
            .rotate(time(20), time(40))
            .expect("rotation should succeed");

        assert_eq!(session.authenticate(time(21)), Err(SessionError::Revoked));
        assert_ne!(replacement.id(), original_id);
        assert_eq!(replacement.principal_id(), principal);
        assert!(replacement.authenticate(time(21)).is_ok());
    }

    #[test]
    fn failed_rotation_does_not_revoke_current_session() {
        let principal = PrincipalId::new();
        let mut session = Session::new(principal, time(10), time(40)).expect("valid session");
        let original_id = session.id();

        assert_eq!(
            session.rotate(time(20), time(20)),
            Err(SessionError::InvalidLifetime)
        );
        assert_eq!(session.id(), original_id);
        assert!(session.authenticate(time(21)).is_ok());
    }

    #[test]
    fn expired_or_revoked_session_cannot_rotate() {
        let principal = PrincipalId::new();
        let mut expired = Session::new(principal, time(10), time(20)).expect("valid session");
        assert_eq!(
            expired.rotate(time(20), time(30)),
            Err(SessionError::Expired)
        );

        let mut revoked = Session::new(principal, time(10), time(30)).expect("valid session");
        revoked.revoke(time(15));
        assert_eq!(
            revoked.rotate(time(16), time(40)),
            Err(SessionError::Revoked)
        );
    }

    #[test]
    fn revocation_keeps_first_timestamp() {
        let principal = PrincipalId::new();
        let mut session = Session::new(principal, time(10), time(40)).expect("valid session");

        session.revoke(time(20));
        session.revoke(time(30));

        assert_eq!(session.revoked_at(), Some(time(20)));
    }
}
