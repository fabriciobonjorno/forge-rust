# Production delivery roadmap

Forge aims to make secure application development straightforward and production
behavior explicit. Progress is measured by executable applications and retained
evidence, not crate counts or unsupported comparisons with other frameworks.
This plan refines the canonical
[delivery sequence](architecture.md#delivery-sequence-and-exit-criteria).

## Current boundary

The framework is unreleased and is not production-certified. Runtime, Docker and
PostgreSQL behavior exist; Phase 3 remains in progress. The current logout slice
adds atomic revocation/audit persistence and asynchronous composition. On
2026-10-03, locked workspace formatting, Clippy, tests and doctests passed, as did
`scripts/e2e-generated-app.sh`, including PostgreSQL logout failure/concurrency
checks, host serving and Docker lifecycle checks. This verifies that slice, not
Phase 3 or production certification. HTTP login, tenant selection and stronger
audit controls remain open.

Session rotation now revalidates the enabled principal, preserves identity and
requires an active replacement under transactional row locks. Before exposing
HTTP login, define pre-session CSRF protection. Argon2 adapter work now has a
shared process-wide admission budget and stored-parameter ceilings, as described
in [ADR 0019](adr/0019-bounded-password-work.md); capacity benchmarking is pending.
Version-bound proof and atomic session/success-audit issuance are implemented
as described in [ADR 0018](adr/0018-version-bound-login-session-issuance.md),
but do not revoke existing sessions after password changes or expose an HTTP
login endpoint. Rotation alone does not solve those login requirements.

Performance numbers are [measurement budgets](performance-targets.md), not results.
Remote job failures before runner steps execute do not establish code failures or
successful validation. Retain local command results while the billing-related
runner limitation persists; supported-platform and release gates remain open
until they actually run. Update this boundary when evidence changes.

## Delivery rules

Each delivery includes a concrete generated application flow, maintained examples,
documentation, relevant [test gates](test-strategy.md) and reviewed diffs. Record
the revision, commands, exit results and artifacts with the delivery. Security
changes update the [threat model](threat-model.md); new dependencies follow the
[admission policy](dependency-policy.md). Earlier phases must remain executable.

Operational safety is mandatory for every capability as it arrives: explicit
tenant scope, bounded work, deadlines, cancellation, least privilege, safe errors
and redaction. Phase 8 consolidates hardening; it does not defer these controls.
Optional AI capabilities must not be required to build, run or understand an
ordinary application. Add no placeholder implementation for a future milestone.

## 1. Complete identity and tenant operations — Phase 3

Deliver a generated application that logs in, selects an authorized tenant,
performs a protected operation and logs out through documented HTTP contracts.

- Compose bounded password verification, durable throttles, independent session
  and CSRF secrets, safe cookie handling and generic public authentication errors.
- Resolve active memberships on tenant operations; role changes and suspension
  take effect without waiting for session expiry. Administrative access uses a
  separate explicit authorization path.
- Complete audit outcomes for sensitive success, denial and failure. Add a
  reviewed integrity/retention design and privileged reader workflow without
  granting the runtime role audit mutation authority.
- Exit evidence: real HTTP/PostgreSQL/container flows; duplicate/malformed
  credentials, CSRF failure, stale sessions, concurrent revocation, role changes,
  cross-tenant reads/writes and pooled-connection reuse. Storage/audit failures
  must preserve the documented atomicity and fail-closed rules.

## 2. Durable asynchronous work — Phase 4

Deliver one business transaction that changes tenant data and writes an outbox
event, then a supervised worker that publishes and handles it idempotently.

- Use PostgreSQL first, transaction-scoped enqueue, bounded batch claims, leases,
  heartbeat/reclaim, retry ceilings, jitter and explicit dead-letter inspection.
- Carry validated tenant context and versioned payloads. Define deduplication
  scope and retention; a repeated key with conflicting content fails explicitly.
- Add scheduling only with explicit time-zone, missed-run and overlap semantics.
- Exit evidence: rollback publishes nothing; worker death after side effects,
  lease expiry, duplicate delivery, poison messages, competing workers and
  shutdown recover correctly. Saturation gives backpressure with bounded memory.
  Document at-least-once delivery; do not promise exactly-once side effects.

## 3. Diagnose the complete request — Phase 5

Deliver correlated HTTP → use case → PostgreSQL → outbox → worker traces with
structured logs, bounded metric labels and useful readiness for each process role.

- Provide request/error rates, latency, pool saturation, queue age, retry and
  dead-letter counts, plus example alerts and operator troubleshooting steps.
- Keep credentials, bodies and personal data out of default telemetry; exporters
  use bounded buffers and deadlines. Tenant/request IDs are not metric labels.
- Exit evidence: trace linkage and redaction tests; collector outage does not
  crash requests or grow memory without bound. Database failure changes readiness
  appropriately while liveness retains its documented meaning.

## 4. Make the secure path the easy path — Phase 6

Deliver a typed resource workflow from generator input to migration, use case,
HTTP endpoint, validation, pagination and matching OpenAPI in a running container.

- Keep domain/application contracts independent of HTTP and SQLx. Generate
  readable code; introduce macros only for measured repetitive work.
- Derive routing, validation and OpenAPI from one contract. Provide safe error
  mapping, useful diagnostics, route inspection and concrete environment checks.
- Exit evidence: deterministic golden output, forbidden-import tests, hostile
  names/paths, overwrite refusal and partial-write recovery. A fresh application
  builds, migrates, serves, documents and shuts down using documented commands.
- Measure clean/incremental builds and the edit/check loop before claiming better
  productivity. Publish an upgrade example for previously generated applications.

## 5. Optional controlled AI capability — Phase 7

Deliver the planned API and CLI adapters behind provider-neutral contracts, with
streaming cancellation, usage accounting and a separate execution policy boundary.

- Enforce request/tenant cost budgets, classified routing, bounded retry/output,
  immutable approval binding and capabilities outside model-generated content.
- Exit evidence: shared adapter contracts and deterministic protocol fixtures;
  injection, replay, expired/mutated plans, output leakage and unsupported sandbox
  profiles fail safely. Live tests are explicit, credential-gated and spend-capped.
- Keep this capability optional in generated applications and dependency graphs.

## 6. Harden the supported surface — Phase 8

Deliver an abuse campaign mapped to implemented trust boundaries, with regression
tests and a reviewed threat model for every supported feature.

- Fuzz hostile protocol/generator parsers and validate resource limits under
  malformed and slow clients. Exercise restore, migration failure and rollback
  recovery with disposable data before documenting operator procedures.
- Run advisory/license/source checks on the locked graph and image scans. Record
  reachable findings and time-bounded exceptions; never suppress unknown findings.
- Exit evidence: retained minimized failures, patched regressions, independent
  security review of identity/tenancy and explicit residual risks. Verify non-root,
  read-only container execution and absence of build secrets in image layers.

## 7. Prove scalability before optimizing — Phase 9

Deliver reproducible workloads for authenticated tenant reads, transactional
create/outbox and worker recovery, alongside HTTP/JSON baseline measurements.

- Preserve authorization, RLS, telemetry and correctness while measuring p50/p95/
  p99, throughput, errors, CPU, RSS, allocations, queue depth and recovery time.
- Use separate load generation and publish hardware, data, revision, toolchain,
  raw artifacts and variance. Profile bottlenecks before changing implementation.
- Exit evidence: saturation reaches a resource plateau and rejects excess work;
  at least two-hour soak has no unexplained growth/task leaks. Demonstrate behavior
  with multiple instances and one failing instance. Compare alternatives only on
  equivalent declared workloads and retain results that do not favor Forge.

## 8. Ship and maintain a supported release — Phase 10

Deliver an installable version whose generated application resolves released
dependencies and passes the full documented host/container path.

- Declare MSRV, supported platforms, public API/versioning policy, migration and
  generated-app upgrade rules, deprecation periods and security-reporting process.
- Publish checksummed artifacts, SBOM, provenance, signatures and minimal scanned
  images. Verify the claimed build reproducibility on documented builders.
- Exit evidence: supported-platform/MSRV gates, clean-install walkthrough,
  previous-version upgrade, backup/restore exercise, release checklist and rollback
  instructions. Demonstrate reference tenant API and async/outbox applications.
- Production claims require recorded operating evidence and a defined support
  policy. Unexecuted checks, unresolved findings and deployment assumptions remain
  explicit release limitations.
