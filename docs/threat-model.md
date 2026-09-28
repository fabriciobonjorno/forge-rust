# Forge Threat Model

Status: Phase 0 baseline. This model defines required controls and tests; it does
not assert that those controls are implemented.

## Scope and security objectives

This model covers Forge framework code, generated applications, the `forge` CLI,
HTTP and streaming endpoints, PostgreSQL persistence, jobs/events, telemetry, and
AI provider/tool execution. It assumes an attacker can send arbitrary network
input, own a normal tenant account, influence content processed by an AI model,
and inspect publicly shipped binaries and source.

Primary objectives are:

1. Preserve tenant isolation and enforce authentication/authorization.
2. Prevent untrusted input or AI output from acquiring ambient capabilities.
3. Preserve integrity and replay safety for state changes, jobs, events, webhooks,
   migrations, and generated code.
4. Limit confidentiality leakage through responses, logs, traces, prompts, and
   third-party providers.
5. Remain available under bounded abuse and fail closed when a security decision
   cannot be made.
6. Produce tamper-evident, attributable audit evidence without recording secrets.

## Assets and actors

Protected assets include tenant data, identities and sessions, credentials and
key material, authorization policy, database integrity, code/release artifacts,
AI prompts and responses, cost budgets, audit records, and service availability.

Actors are anonymous clients, authenticated users, tenant administrators, system
operators, application developers, external services/webhook senders, AI
providers, and AI-driven agents. Infrastructure administrators are privileged but
their actions remain auditable. External services and AI providers are never
implicitly trusted merely because they are configured.

## Trust boundaries and data flow

```text
Internet
  | untrusted protocol/input
  v
HTTP adapter -- authn/limits/validation --> application policy boundary
                                               | explicit tenant/capability
                                               v
                                      persistence and external ports
                                               |
                            +------------------+------------------+
                            v                                     v
                     PostgreSQL/RLS                    external vendors

AI provider response/tool call (untrusted)
  -> policy -> authorization -> validation -> approval
  -> sandboxed executor -> filtered result -> audit

developer input -> forge CLI/generator -> reviewed source tree -> build pipeline
```

Trust transitions must use typed, validated contracts. Internal network location
does not establish identity. Tenant and principal context cannot be inferred from
global state, model output, headers accepted from the public edge, or job payloads
alone.

## Threats and required controls

| Threat | Required controls | Verification |
| --- | --- | --- |
| Credential stuffing and session theft | Argon2id password hashing with parameter policy, generic login errors, per-account and per-origin throttles, rotation on privilege change, secure/HttpOnly/SameSite cookies, short-lived credentials, revocation | authentication integration and abuse tests |
| CSRF and cross-origin abuse | SameSite defaults, unpredictable CSRF token for cookie-authenticated mutations, exact-origin CORS allowlist, reject credentialed wildcard origins | browser-protocol integration tests |
| Broken object authorization | authorization at use-case boundary, typed principal and tenant context, deny by default, no authorization based on object-ID secrecy | policy matrix and negative integration tests |
| Cross-tenant data access | mandatory `TenantContext`, transaction-local PostgreSQL setting, RLS, non-bypass application role, pooled-connection reset, audited separate admin path | two-tenant tests for every repository plus RLS tests |
| SQL/command/template injection | parameterized SQL, typed command arguments without shell interpolation, contextual output encoding, validated schemas | fuzz/property tests and static review |
| SSRF | parsed URL policy, deny loopback/link-local/private/control-plane ranges by default, DNS resolution and redirect revalidation, scheme/port allowlist, response/time limits | rebinding and redirect abuse tests |
| Path traversal and unsafe upload | logical storage keys, canonical root confinement, no user-controlled absolute paths, file type/size limits, randomized server names | traversal corpus and symlink-race tests |
| Resource exhaustion | request/body/header limits, bounded queues/concurrency, deadlines, backpressure, per-principal quotas, streaming byte limits, circuit breakers | load and saturation tests |
| Replay and duplicate mutation | scoped idempotency keys with request digest and expiry, signed webhook timestamp/nonce, persistent deduplication for jobs/events, single-use execution-plan ID | replay integration tests |
| Webhook forgery | per-source signature verification over raw bytes, constant-time comparison, timestamp window, secret rotation overlap, replay store | signature vector and replay tests |
| Sensitive-data leakage | classified fields, response allowlists, error redaction, telemetry filtering, secrets as opaque references, no prompt/body capture by default | snapshot/redaction tests and log scans |
| Audit tampering | append-only restricted writer, actor/tenant/action/outcome/policy/version/request linkage, integrity chaining or external immutable sink, retention policy | authorization and integrity-verification tests |
| Dependency/build compromise | pinned lockfile, advisory/license/source policy, minimal features, reviewed build scripts, SBOM, isolated CI, provenance and signed release artifacts | `cargo deny`, audit, reproducibility and signature gates |
| Container escape or image secret leakage | multi-stage build, immutable base digests, non-root numeric user, minimal final stage, read-only filesystem compatibility, dropped capabilities, no layer/build-arg secrets, image scanning | image inspection and container E2E tests |
| Migration compromise | checksummed immutable released migrations, dedicated migration role, advisory lock, backup/restore plan, least privilege | migration integrity and rollback-recovery exercises |
| Malicious generated input | strict generator grammar, normalized paths confined to project, no template evaluation from untrusted packages, preview/diff before overwrite | path/property tests and golden outputs |
| Unsafe Rust memory flaw | default deny for framework-written `unsafe`; isolated reviewed modules with stated invariants and Miri/fuzz coverage when unavoidable | lint policy, review, Miri/fuzz gates |

## AI-specific abuse cases

AI prompts, retrieved documents, provider responses, structured outputs, and tool
arguments are untrusted. Prompt instructions cannot grant authority.

| Abuse case | Required response |
| --- | --- |
| Prompt injection requests secrets or policy bypass | Secret data is unavailable to the model unless an explicit scoped capability permits it; policy is evaluated outside the model. |
| Model invents or broadens a tool argument | Validate against a closed schema and compare the executed plan byte-for-byte with the authorized plan. Unknown fields and ambiguous paths fail. |
| Model requests shell/network/filesystem access | Deny unless an explicit capability and sandbox profile grants the exact operation. Default sandbox has no network and minimal filesystem visibility. |
| Tool output injects a second instruction | Treat output as data, label provenance, filter secrets, and require a new policy decision for every subsequent action. |
| Provider fallback violates data residency | Route only among providers permitted by the request's data-classification policy; otherwise return a policy error. |
| Retry repeats a side effect | Generation may retry within budget; execution requires an idempotency contract and a single-use execution-plan identifier. |
| CLI provider escapes process controls | Use argv without a shell, sanitized environment, fixed executable resolution, bounded stdio, working-directory confinement, timeout, cancellation, and sandbox. |
| Model drains token/cost budget | Enforce per-request, principal, tenant, and provider token/cost ceilings before and during streaming; cancel when exceeded. |
| Sensitive prompt content reaches telemetry | Record identifiers, model, timing, token counts, policy outcome, and cost by default; content capture is separate opt-in with classification and retention. |

Approval is a policy action, not a UI confirmation alone. Approval records bind a
principal to the exact execution-plan digest and expire. High-risk capabilities
(credential access, external mutation, broad filesystem writes, or privileged
commands) must not be auto-approved.

## Cryptography and secret management

Forge will use maintained cryptographic crates and platform/provider KMS systems;
it will not implement primitives. Keys have identifiers and versions, and
ciphertexts record the version required to decrypt. Rotation supports an overlap
window and background re-encryption. Comparisons of authenticators use
constant-time library functions.

Secrets are resolved at the infrastructure boundary, never stored in ordinary
configuration values, and redacted from `Debug`, errors, telemetry, CLI output,
and generated diagnostic bundles. Secret access is capability-scoped and audited.

## Fail-closed rules

- Missing or invalid identity, tenant, policy, sandbox support, key material, or
  replay state denies the operation.
- Failure of the audit sink denies high-risk AI execution and privileged admin
  actions; normal read availability may use a bounded durable local buffer only if
  policy explicitly permits it.
- A database transaction that cannot establish tenant-local settings is aborted.
- Unknown configuration keys fail startup in production mode.
- Provider fallback is disabled if classification policy cannot be evaluated.

## Residual risks and assumptions

- A fully privileged host administrator can inspect process memory; deployment
  hardening and managed secret/KMS services reduce but do not remove this risk.
- RLS cannot protect data copied to logs, caches, search systems, or third parties;
  every adapter needs equivalent tenant scoping.
- OS sandbox guarantees differ. An executor must publish its supported capability
  level and refuse policies it cannot enforce.
- Compromised allowed dependencies or toolchains remain a risk despite audits and
  provenance controls.
- Models can produce unsafe content even when system access is denied. Product
  policy and output moderation are application concerns exposed through hooks.

## Phase 3 review: identity and PostgreSQL tenant isolation

Review date: 2026-09-27.

The Phase 3 slices implement typed authenticated principals, deny-by-default
RBAC, explicit tenant membership/TenantContext, separate PostgreSQL migration and
runtime roles, transaction-local tenant/principal settings for RLS, server-side
session persistence by non-replayable credential digest contract, membership RLS,
and structured append-only audit persistence. The long-running runtime role is
generated without schema ownership or BYPASSRLS, and migration commands require a
credential that is distinct from the runtime URL.

`scripts/e2e-generated-app.sh` is the linked negative integration test for the
database boundary. It verifies that the runtime credential cannot be used for
migration dispatch, the runtime database role cannot create schema objects,
missing tenant context sees no RLS-protected rows, one tenant cannot see another
tenant's row, a cross-tenant write is rejected, persisted sessions are located by
digest, memberships fail closed without context, forged audit tenant attribution
is denied, and ordinary runtime access cannot read/update/delete audit rows.

Remaining Phase 3 threat-model controls are not claimed complete: Argon2id
password hashing, bearer-token generation/digest implementation, credential
throttling, secure cookie and CSRF behavior, HTTP authentication/policy
integration, cryptographic audit integrity or an external immutable sink, and
administrative cross-tenant role separation still require their own implementation
and abuse tests.

## Review cadence

Update this model when adding a trust boundary, privileged capability, protocol,
provider, sensitive data class, or deployment target. Phase 3, Phase 7, Phase 8,
and production certification require explicit threat-model review with linked
security tests. Security fixes receive a regression test unless disclosure or
environment constraints make that unsafe, in which case the exception is recorded.

Related decisions: [tenancy and RLS](adr/0004-tenancy-and-rls.md),
[AI execution boundary](adr/0005-ai-provider-and-execution-boundary.md), and
[dependency policy](dependency-policy.md). Container defaults are recorded in
[ADR 0008](adr/0008-docker-by-default.md).
