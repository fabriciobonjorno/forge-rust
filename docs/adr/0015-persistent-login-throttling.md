# ADR 0015: Shared PostgreSQL login-attempt throttling

- Status: Accepted
- Date: 2026-10-04
- Refines: [ADR 0014](0014-argon2-session-cookie-and-csrf.md)
- Refined by: [ADR 0016](0016-trusted-proxy-client-addresses.md)

## Context

Argon2id deliberately consumes CPU and memory. A process-local limiter can be
restarted or bypassed by sending requests to another replica, so it does not
bound authentication work in a horizontally scaled generated application.
Throttle state must be atomic across replicas and must not persist raw login
identifiers or client IP addresses.

## Decision

Database-enabled generated applications use the `LoginAttemptStore` contract
and a PostgreSQL adapter backed by `forge_login_attempt_buckets`. Each row holds
one scope, a fixed-width 32-byte digest, a window start and a positive attempt
count. The default policy permits five attempts per normalized login and thirty
attempts per observed client peer address in a shared fifteen-minute fixed
window. Successful password verification clears only the login bucket; it does
not reset the client-wide bucket.

The adapter updates both buckets in one PostgreSQL transaction using
`INSERT ... ON CONFLICT`, so concurrent replicas serialize on the unique keys.
The `app_runtime` role receives only the table privileges needed for those
operations. One replica at a time opportunistically removes at most 500 rows
older than the maximum supported seven-day window under a transaction-scoped
advisory lock. Storage failure denies authentication before Argon2 work.

Keys are currently plain SHA-256 digests of the normalized login and socket
peer address. This prevents direct disclosure in ordinary row inspection but
does not make the values anonymous or resist a dictionary attack against a
database snapshot. Do not treat these digests as anonymized data. A keyed HMAC
with an operator-managed secret remains a hardening requirement before claiming
resistance to offline identifier guessing.

Client identity uses `Request::remote_addr()`, the peer observed by the listener.
Forge ignores forwarded-IP headers because no trusted-proxy configuration is
implemented yet. Applications behind a shared reverse proxy can therefore
aggregate many users under one client bucket; such deployments must not enable
this default route until trusted proxy handling is added or the app selects a
reviewed policy suitable for that topology.

## Consequences

- Attempt state is durable and shared across replicas without Redis or another
  service.
- Login and client limits are independently enforced, and parallel requests
  cannot all pass by racing a process-local counter.
- A PostgreSQL outage blocks password authentication rather than allowing an
  unbounded Argon2 workload.
- Successful login does not erase client-wide abuse history.
- Raw login and IP values do not enter the throttle table; digests still count
  as pseudonymous personal data and require the application's normal retention
  and database-access controls.
- Trusted proxy support and keyed digesting remain open production-readiness
  requirements; the current behavior is not a universal proxy-ready policy.
