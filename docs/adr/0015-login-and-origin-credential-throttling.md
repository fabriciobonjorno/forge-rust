# ADR 0015: Durable login and origin credential throttling

- Status: Accepted
- Date: 2026-09-28
- Refines: [ADR 0014](0014-argon2-session-cookie-and-csrf.md)
- Refined by: [ADR 0016](0016-explicit-trusted-proxy-client-addresses.md)

## Context

Argon2id intentionally makes password verification expensive, so unbounded login
attempts are both a credential-guessing risk and a CPU/memory exhaustion vector.
Process-local counters are insufficient for multi-process/multi-instance
applications, and a check-then-increment sequence can be bypassed with concurrent
requests.

A successful login also must not reset the entire source-address budget: an
attacker with one valid account could otherwise repeatedly clear an origin-wide
limit while guessing other accounts.

## Decision

Forge defines framework-owned login throttling contracts:

- `LoginThrottleKey` is an opaque 256-bit persistence key;
- `LoginThrottlePolicy` validates positive attempt/window/block values;
- `LoginThrottleStore::reserve` atomically consumes one attempt or returns a
  bounded retry interval;
- `LoginThrottleStore::clear` removes state for one key after successful
  authentication.

Generated applications derive two independent SHA-256 namespaced keys before
password verification:

1. a canonical case-insensitive login key;
2. a client-IP origin key that excludes the ephemeral port. By default the
   client IP is the TCP peer address; explicit trusted-proxy CIDRs may enable
   right-to-left `X-Forwarded-For` resolution as described in ADR 0016.

The throttle table stores only opaque keys, counters and timestamps; raw login
identifiers and addresses are not required by the persistence contract. These
hashes reduce accidental disclosure but are not an anonymity boundary against
offline guessing of low-entropy identifiers.

The generated PostgreSQL adapter serializes operations for one key with
transaction-scoped advisory locks. The attempt is reserved before credential
lookup and Argon2id work. This prevents parallel requests from exceeding the
configured budget through lost updates.

Initial generated policies are:

- login: 5 attempts per 300 seconds, then 900 seconds blocked;
- origin: 50 attempts per 300 seconds, then 900 seconds blocked.

A successful login clears only its login-scoped key. The origin key is retained.
Unknown, malformed, disabled and wrong-password attempts consume both budgets
when the store permits password work.

Throttle-store failure is fail-closed as authentication unavailable. A denial
does not reveal whether the login identifier exists; callers receive one
throttled outcome with a retry interval.

## Consequences

- Brute-force and credential-stuffing attempts are bounded across application
  processes that share PostgreSQL.
- Concurrent requests for the same throttle key are serialized.
- A valid account cannot reset an attacker's origin-wide budget.
- Throttling adds a PostgreSQL write/lock before every password verification.
- `authenticate_password` receives the resolved client IP from the HTTP
  request. Applications that enable proxies must configure only networks that
  overwrite `X-Forwarded-For`; otherwise clients could spoof origin budgets.
- Turnkey HTTP login/logout response mapping and Retry-After headers remain a
  later adapter-composition slice.
