# ADR 0018: Version-bound atomic login session issuance

- Status: Accepted
- Date: 2026-10-03
- Refines: [ADR 0014](0014-argon2-session-cookie-and-csrf.md), [ADR 0017](0017-atomic-http-logout-and-async-composition.md)

## Decision

The generated password verifier returns `VerifiedPassword`, not merely an ID.
This in-process, non-serializable, non-cloneable value retains principal, observed
credential version and hash. Its constructor is crate-private; it is not a wire
credential or a protection against malicious trusted application code. Debug
uses the existing redacted password-hash representation.

After optimistic rehash succeeds, the proof uses the incremented version and
replacement hash. Current-policy passwords retain the observed version/hash.
`LoginSession` consumes the proof and creates a fixed `session.login` success
audit event from trusted identity and request correlation. Secret generation
remains in infrastructure, outside application contracts.

The PostgreSQL issuance adapter locks the enabled principal and requires both
version and hash to match before inserting session digests and audit evidence
in one transaction. Checking the hash also rejects an administrative password
change that omitted the version increment. Disable or a credential change that
committed before the lock is acquired rejects issuance; a later mutation waits
for the issuance transaction. This does not revoke previously issued sessions
on password changes: that is a separate explicit policy.

Audit rejection rolls back issuance. An interrupted COMMIT can leave the client
uncertain while both records still persist atomically. Retries may create a
separate session; no exactly-once or durable single-use proof claim is made.

## Boundaries and compatibility

This changes the unreleased generated `authenticate_password` return type;
callers must pass its proof to `issue_authenticated_session`, not turn its ID
into an unaudited session. Existing generated applications require explicit code
updates; templates never silently rewrite their files. `issue_persisted_session`
remains a low-level provisioning/test helper, not a login composition path.

No dependency, migration or privilege expansion is needed. HTTP login is not
registered yet: pre-session CSRF/origin policy and bounded Argon2 concurrency
remain required before exposing that endpoint.

## Verification

Generated `tests/auth_login_session.rs` runs against disposable PostgreSQL with
separate owner/runtime roles. It covers successful issuance/audit, stale proofs,
disabled principals, hash changes without version increments, policy rehash and
audit failure rollback. The full generated-app E2E also protects existing logout,
rotation, migration and host/container lifecycle behavior.
