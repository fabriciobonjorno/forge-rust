# ADR 0019: Bounded password work

- Status: Accepted
- Date: 2026-10-03
- Refines: [ADR 0014](0014-argon2-session-cookie-and-csrf.md), [ADR 0015](0015-login-and-origin-credential-throttling.md)

## Decision

Generated Argon2 adapters share one process-wide budget of two admitted blocking
jobs, including queued and running jobs. This covers hashing, verification,
enumeration-dummy work and rehash through the same adapter. Saturation fails
immediately as `PasswordHashError::Unavailable`; login maps it to the existing
generic authentication-unavailable result. It does not queue waiting callers or
change durable account/origin throttles. Unknown/invalid input does not bypass
capacity admission.

Admission precedes `spawn_blocking`. A scoped permit is moved into the blocking
closure and released on completion or panic. Cancelling the awaiting request
does not free capacity while a started blocking job continues. Tokio cannot
abort started synchronous Argon2 work; it remains bounded and can outlive its
caller. Runtime/process shutdown still owns teardown, and its existing deadline
does not promise immediate termination of a started hash.

Stored PHC parameters are parsed using the maintained Argon2 crate and rejected
before verification if m exceeds 65,536 KiB, t exceeds 6, or p exceeds 4. The
generated hashing policy remains m=19,456 KiB, t=2, p=1. Accepted legacy hashes
below the ceilings can still verify and upgrade. Imported hashes above these
limits require an explicit reviewed migration policy, not silent resource-limit
relaxation. The bound is on Argon2's memory parameter, not total process RSS:
two 64-MiB jobs still require overhead and unrelated service memory.

No dependency or feature addition is needed. The atomic counter is an internal
resource-admission mechanism, not custom cryptography or a public runtime type.
The initial budget is conservative, not a measured throughput recommendation;
deployment sizing and workload benchmarks remain required. Multiple processes
have independent local budgets and shared durable attempt throttles.

## Verification and remaining scope

Generated security tests exercise public hash/verify overload, recovery,
request-future cancellation while a controlled blocking worker remains active,
panic recovery and oversized stored parameters. Channel synchronization avoids
sleep-based ordering; test runtime teardown joins the controlled worker.
Existing password/rehash PostgreSQL E2E and host/container lifecycle checks
must remain green.

HTTP login remains unregistered pending explicit pre-session CSRF/origin policy.
This limit does not establish production capacity, whole-system fairness,
distributed resource admission, or comprehensive abuse resistance.
