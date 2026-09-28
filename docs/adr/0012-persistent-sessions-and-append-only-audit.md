# ADR 0012: Persistent sessions and append-only audit storage

- Status: Accepted
- Date: 2026-09-27
- Refines: [ADR 0010](0010-session-and-tenant-authorization-context.md)

## Context

Phase 3 already defines server-side session lifecycle and explicit tenant
authorization, but those contracts need durable storage. Persisting bearer
credentials directly would turn a database read into session theft. Audit records
also need an application-facing contract that does not encourage dumping request
bodies, credentials, or other secret-bearing metadata.

Audit storage must resist ordinary application-level mutation. At the same time,
cryptographic integrity chaining should not be improvised merely to claim
tamper-evidence.

## Decision

Forge session persistence stores a fixed 256-bit credential digest, never the raw
bearer credential. The framework-owned SessionStore supports insert, lookup by
digest, revocation, and atomic rotation. Restoring a persisted Session re-validates
lifetime/revocation invariants before it can authenticate a principal.

Database-enabled generated applications receive a SQLx PostgresSessionStore and
an initial Forge migration containing:

- forge_principals;
- forge_sessions;
- forge_tenant_memberships;
- forge_audit_log.

The tenant membership table uses RLS. Before tenant selection, an authenticated
principal can resolve its own memberships from transaction-local principal
context. After tenant authorization, tenant-local context can scope membership
administration.

Forge owns a small structured audit contract: event ID, trusted timestamp,
principal/tenant attribution, stable action, outcome, optional target, and optional
request/correlation ID. Arbitrary maps, request bodies and secret-bearing payloads
are intentionally absent from the stable contract.

Generated applications receive a SQLx PostgresAuditSink. The audit table has
forced RLS with an INSERT-only policy. Anonymous pre-authentication events may
have no principal/tenant. Authenticated events must match transaction-local
principal/tenant settings. Ordinary runtime access has no RLS policy for SELECT,
UPDATE, or DELETE, making the writer append-only through the application role.

The first generated migration also removes runtime access to SQLx migration
metadata when the local app_runtime role exists.

## Consequences

- A database dump of forge_sessions contains credential digests rather than
  replayable bearer credentials.
- Session rotation can be persisted atomically.
- Corrupt persisted session timestamps fail closed when restored.
- Audit attribution is bound to framework-owned authenticated/tenant context.
- Runtime application code cannot normally read, edit, or delete audit rows.
- Operators need a separate privileged/read path for audit export and retention.
- The generated identity schema is framework-owned and precedes user migrations.

This ADR does **not** claim cryptographic tamper-evidence. Integrity chaining or an
external immutable audit sink remains required before Phase 3 audit integrity is
complete. Password hashing, random bearer-token generation, secure cookies,
CSRF protection, throttling, and HTTP authentication middleware also remain
separate slices requiring security/dependency review.
