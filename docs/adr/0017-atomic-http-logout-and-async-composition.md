# ADR 0017: Atomic HTTP logout and asynchronous composition

- Status: Accepted
- Date: 2026-09-30
- Refines: [ADR 0012](0012-structured-append-only-audit-evidence.md), [ADR 0013](0013-persistent-principals-sessions-and-memberships.md), [ADR 0014](0014-argon2-session-cookie-and-csrf.md)

## Context

Generated applications already have durable sessions, secure cookie/CSRF helpers,
and an append-only audit sink. They need an executable HTTP flow without moving
SQLx into application contracts. Synchronous route registration cannot open an
asynchronous PostgreSQL pool before the runtime exists. A successful logout must
not be persisted separately from its required audit evidence.

## Decision

`App::setup` runs asynchronous composition inside the process-owned runtime,
after synchronous route registration and before binding the listener. Setup
handlers run sequentially under a total deadline (30 seconds by default,
configurable with `App::startup_timeout`). Shutdown signals are installed before
setup; shutdown cancels initialization. Setup errors, timeouts, duplicate routes,
and missing dependencies prevent serving and readiness.

Database-enabled generated applications connect a bounded runtime PostgreSQL
pool, check session/principal schema and required mutation/audit privileges, and
capture the pool-backed stores in the registered handlers. Informational commands,
health probes and migration commands do not run serving setup. The migration
credential is never selected for serving.

`POST /auth/logout` accepts an empty body. It authenticates the host-only session
cookie and requires exactly one matching `x-csrf-token` before invoking the
application use case. Duplicate session cookies are rejected both within one
Cookie header and across headers. Session lookup and CSRF verification reject
disabled principals.

The application constructs a `session.logout` success audit event from the
authenticated identity and the server-generated request link. Its `LogoutStore`
port requires atomic revocation and evidence. The PostgreSQL adapter locks the
principal/session rows, revalidates enabled state, principal identity, issuance,
expiry and revocation, then revokes and appends the event in one transaction.
No new privileges or dependencies are introduced.

Only a committed transaction yields HTTP 204 and a Secure, HttpOnly, SameSite=Lax,
Path=/ host-only removal cookie. Every logout handler response uses Cache-Control:
no-store. Missing/invalid identity is 401, CSRF denial is 403, an unexpected body
is 400, and dependency/audit failure is 503. Public responses contain neither
credential values nor database diagnostics.

## Consequences

- Audit-write rejection rolls back revocation; the client may retry its session.
- Concurrent requests cannot commit two success records for one active session.
- Principal disable, session rotation and revocation are rechecked at mutation.
- A connection failure or cancellation after COMMIT was sent can have an unknown
  client-visible outcome. Revocation and evidence still commit together; a retry
  of a revoked session returns 401 and is not a second successful logout.
- This route has no ambient tenant state and grants no tenant authority.
- Login issuance and tenant-selection routes remain separate delivery slices.
- Rejected protocol requests are not yet covered by a comprehensive audit policy.

## Verification

Framework tests cover runtime setup, errors, deadlines/resource drop, duplicate
registration, informational-command isolation and shutdown cancellation.
`tests/auth_logout.rs` in database-enabled generated applications exercises the
real HTTP server and distinct PostgreSQL roles, including duplicate cookies/CSRF,
expiry, revocation, disabled principals, a disable after authentication,
audit-failure rollback, concurrent logout, and unavailable storage. It is ignored
without a disposable migrated database and explicitly run by
`scripts/e2e-generated-app.sh`.
