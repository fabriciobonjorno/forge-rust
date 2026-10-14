# ADR 0017: Version-bound login with atomic session and audit persistence

- Status: Accepted
- Date: 2026-10-04
- Refines: [ADR 0012](0012-structured-append-only-audit-evidence.md), [ADR 0014](0014-argon2-session-cookie-and-csrf.md)

## Context

Password verification, optional Argon2id policy upgrade, session persistence,
and success auditing must not be separate success paths. A password may change or
a principal may be disabled after the hash is verified; a session write may
succeed while its mandatory audit write fails. Returning client credentials
before all those decisions are durable would permit stale or unaudited login.

## Decision

The trusted verifier produces a non-cloneable, non-serializable in-process proof
containing the principal, observed credential version and hash, and an optional
replacement hash. Password verification and hash computation remain outside the
database transaction.

The PostgreSQL login-session adapter then performs one transaction:

1. Lock the principal row and require it to remain enabled with the observed
   version and exact hash.
2. Apply an optional Argon2id rehash and increment the credential version.
3. Insert the new session's bearer and CSRF digests.
4. Append the successful `session.login` audit event.
5. Commit before returning the bearer or CSRF secret to the caller.

Any stale credential, failed insert, audit rejection, or commit failure returns
no client secret and persists neither session nor success event. Audit writes
continue to use the append-only executor helper and remain immutable even in
tests. The standalone unaudited session issuance helper is removed from generated
applications; password login must consume the verified proof.

## Consequences

- A credential update or account disable committed before the lock invalidates a
  previously verified proof.
- A successful login's rehash, session, and audit evidence share one commit.
- Hashing cost does not hold a database connection or row lock.
- The generated PostgreSQL scaffold composes login/logout only with a validated
  public origin and the required CSRF/session dependencies. Account provisioning
  and tenant-aware authorization remain separate work.
- The generated PostgreSQL contract test must exercise success, stale credentials,
  and audit failure rollback using a disposable database.
