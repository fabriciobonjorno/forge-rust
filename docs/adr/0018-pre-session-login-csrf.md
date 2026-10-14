# ADR 0018: Exact-origin and double-submit CSRF for pre-session login

- Status: Accepted
- Date: 2026-10-04
- Refines: [ADR 0014](0014-argon2-session-cookie-and-csrf.md), [ADR 0017](0017-version-bound-login-session-issuance.md)

## Context

Session-bound CSRF cannot protect password login because the browser has no
authenticated session yet. Login therefore needs a pre-session proof and an
origin check before password verification or any state change. The expected
origin must be an operator-configured value; reflecting `Host` or forwarded
headers would let an attacker influence the trust decision.

## Decision

Before login, the application issues an unpredictable 256-bit token using the
OS CSPRNG and returns it as a readable `__Host-forge_csrf` cookie with
`Secure`, `Path=/`, `SameSite=Lax`, and no `Domain`. Same-origin client code
copies the token into exactly one `x-csrf-token` header on the login request.
The HTTP adapter validates that the cookie and header are both canonical tokens
and compares their SHA-256 digests with a constant-time equality operation.

The login request must contain exactly one `Origin` header byte-for-byte equal
to `FORGE_PUBLIC_ORIGIN`. Configuration canonicalizes and validates the value:
HTTPS is required except for localhost/loopback HTTP during local development;
credentials, paths, query strings, and fragments are rejected. An absent,
duplicate, opaque, or mismatched request origin fails closed. The adapter never
derives the expected value from incoming `Host`, `Forwarded`, or
`X-Forwarded-Host` headers. Routes must not be registered until composition
supplies this trusted setting and the pre-session proof check is wired before
login throttling and password work.

The readable cookie is safe for this use because the token is not an
authentication credential: it only proves that the request could read a
same-origin response. The `__Host-` prefix prevents parent-domain cookie
planting, while the Origin check rejects cross-origin browser requests.

## Consequences

- Login CSRF is rejected before expensive Argon2 work.
- A sibling subdomain cannot plant the pre-session CSRF cookie.
- Misconfigured or missing `FORGE_PUBLIC_ORIGIN` denies login rather than
  weakening the check.
- The token is not persisted and is rotated by issuing a fresh value for the
  pre-login bootstrap response.
- Cross-origin browser clients are unsupported by this cookie-authenticated
  login flow; APIs requiring cross-origin access need a separately reviewed
  authentication contract.
- The generated PostgreSQL scaffold registers login routes through the bootstrap
  composition root only when the trusted origin and database dependencies exist.

## Validation requirements

Tests must cover exact-origin success, missing/duplicate/mismatched origins,
missing/malformed/duplicate cookies, missing/duplicate/malformed headers,
cookie/header mismatch, cookie attributes, and proof rejection before any
password-hasher or throttle call.
