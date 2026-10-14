# ADR 0019: Atomic session revocation and logout audit

- Status: Accepted
- Date: 2026-10-04
- Refines: [ADR 0012](0012-structured-append-only-audit-evidence.md), [ADR 0014](0014-argon2-session-cookie-and-csrf.md)

## Context

Logout changes authentication state and is security-relevant. Revoking a
session without its audit record loses evidence; appending the audit record
without revocation claims an action that did not take effect. A cookie clear in
the HTTP response is not revocation because a copied bearer may still be used.

## Decision

The generated logout use case receives an `AuthenticatedPrincipal` proof from
session verification. It creates a `session.logout` audit event for that
principal and asks its persistence port to revoke the proof's exact session.
The PostgreSQL adapter updates only an active, unexpired session whose
`principal_id` matches the authenticated proof, appends the audit event in the
same transaction, and commits both together. A repeated, expired, or mismatched
session fails closed. A failed audit append rolls back revocation.

The browser `POST /auth/logout` route requires the configured exact `Origin`,
valid session cookie, and session-bound CSRF header before invoking the use
case. After commit it returns `204` and expires both host-only session and CSRF
cookies. Cookie deletion is presentation cleanup; persisted revocation is the
security boundary.

## Consequences

- Stolen session credentials stop authenticating immediately after successful
  logout, including across application replicas.
- Concurrent logout requests can produce at most one success audit for the
  session; later requests see the revoked row and fail.
- Database-enabled generated applications expose the route only when
  `FORGE_PUBLIC_ORIGIN` is configured.
- PostgreSQL integration tests must prove successful revocation/audit,
  duplicate logout rejection, and rollback when audit insertion fails.
- Account provisioning and authorization middleware remain separate concerns.
