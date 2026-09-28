# ADR 0012: Structured append-only security audit evidence

- Status: Accepted
- Date: 2026-09-27

## Context

Security-sensitive actions need attributable evidence that is independent from
ordinary application logs. Free-form log strings are difficult to validate,
query, retain, or protect from accidental mutation. The threat model also
requires privileged operations to fail closed when mandatory audit evidence
cannot be persisted.

Audit records may contain security metadata, so the normal serving role should
not receive broad read/update/delete access merely because it can write events.

## Decision

Forge defines framework-owned immutable audit contracts in `forge-audit`:

- UUIDv7 audit event IDs;
- trusted Unix timestamp;
- typed actor: anonymous, authenticated principal, or system;
- optional typed tenant identity;
- validated bounded action name;
- explicit outcome: allowed, denied, succeeded, or failed;
- optional bounded request/correlation linkage;
- classified safe audit errors;
- an append-only `AuditSink` application port.

When a `TenantContext` exists, `AuditEvent::from_tenant_context` derives the
principal and tenant directly from that authorized context rather than accepting
caller-supplied identifiers.

Database-enabled generated applications include a PostgreSQL audit sink and a
framework migration for `forge_audit_events`. The generated development
runtime role receives only INSERT on this table. SELECT, UPDATE, and DELETE are
revoked. A database trigger also rejects UPDATE/DELETE to preserve append-only
semantics even if broader table privileges are accidentally granted later.

The application-facing sink returns an `AuditReceipt` only after the adapter
accepts the event for durable storage. Sink failure remains explicit; higher-risk
operations can therefore fail closed according to policy.

This slice does not claim tamper-evident cryptographic chaining, external WORM
storage, retention enforcement, or privileged audit-reader workflows. Those are
separate infrastructure controls and must not be implied by the append-only API.

## Consequences

- Security evidence has stable structured fields instead of ad-hoc log text.
- Tenant-scoped attribution can be tied to the same authorized context used by
  persistence and RLS.
- Ordinary runtime database credentials can append audit events without being
  able to inspect or mutate the audit history.
- Audit readers, retention/archival jobs, integrity verification, and support
  tooling require explicit separate roles and APIs.
- Privileged use cases must decide which audit failures are fail-closed and may
  not silently discard required evidence.
