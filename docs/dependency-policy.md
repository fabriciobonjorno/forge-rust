# Dependency Policy

Status: Accepted Phase 0 policy. Concrete dependency versions are selected and
recorded when the first consuming vertical slice is implemented.

## Principles

Dependencies are security-sensitive implementation choices, not shortcuts around
Forge's architecture. Forge prefers mature, focused crates for protocols,
cryptography, runtimes, parsing, and database drivers while owning its lifecycle,
contracts, policy, and developer experience.

Do not introduce a dependency until code in the same change uses it. Avoid a
framework-wide dependency when a leaf adapter can own it. Domain and application
contract crates keep the smallest practical graph and do not depend on vendor SDKs.

## Admission review

Every new direct dependency must document in the pull request:

- the capability it provides and why existing code or dependencies do not;
- maintenance activity, ownership concentration, release history, and issue
  responsiveness;
- advisory history and handling, use of `unsafe`, build scripts, proc macros,
  native code, network/build-time behavior, and transitive graph impact;
- license compatibility, default features, MSRV, platform support, binary-size and
  compile-time impact;
- whether its public types would leak into Forge APIs and the exit/replacement
  strategy.

Cryptography, authentication, parsers for hostile input, sandboxing, and release
tooling require security-owner review. Git dependencies are prohibited in releases
unless an incident exception pins an immutable commit, records provenance and
review, and has an expiry. Dependencies fetched from arbitrary registries are
prohibited unless that registry is explicitly approved and integrity-pinned.

## Allowed dependency roles

The architectural choices currently permit, but do not yet install:

- Tokio as the initial async runtime, behind lifecycle contracts.
- SQLx as the PostgreSQL implementation, behind repository/transaction ports.
- Maintained HTTP and TLS protocol crates behind Forge HTTP contracts.
- `serde`-ecosystem serialization at adapters and deliberately approved value
  boundaries.
- `tracing` and OpenTelemetry ecosystem crates behind Forge telemetry contracts.
- Maintained, audited password-hashing, cookie, signature, and TLS crates; no Forge
  cryptographic primitives.
- CLI and diagnostic crates that do not require application runtime linkage.

An allowed role is not blanket approval of any crate or feature set. Version and
feature choices still undergo admission review.

## Phase 2 PostgreSQL dependency decision

Phase 2 generated applications use SQLx 0.9 as the concrete PostgreSQL adapter.
The generated dependency disables SQLx default features and enables only the
capabilities Forge currently consumes: PostgreSQL, Tokio runtime integration,
migrations/macros, UUID, JSON, chrono, and Rustls with the ring/WebPKI root
backend.

SQLx remains an outer-layer implementation detail: Forge-owned database errors,
optimistic-version values and transaction contracts live in `forge-db`; domain
and application rings must not import SQLx. Generated bootstrap/infrastructure
code may use SQLx directly because those layers are adapters.

Rustls is enabled explicitly rather than relying on a runtime feature to imply
TLS. This keeps production PostgreSQL URLs that require TLS within the supported
generated configuration while avoiding a native OpenSSL dependency.

The dependency is not added to the framework workspace merely for convenience:
only generated applications that enable the database capability consume it.
`--skip-database` removes SQLx entirely from a generated application's graph.

## Features and public API containment

- Set `default-features = false` when defaults add unused protocols, native
  dependencies, or runtime behavior; otherwise record why defaults are safe.
- Feature flags are additive capabilities. They must not silently weaken security
  or change data semantics.
- Platform/provider integrations live in leaf crates or private modules.
- Public Forge contracts use standard library or Forge-owned types unless an ADR
  deliberately commits to an ecosystem type.
- Multiple versions of security-sensitive or foundational crates are investigated
  and resolved when practical.

## Versioning, lockfiles, and MSRV

The workspace commits `Cargo.lock`, including libraries, so CI, examples, tools,
and releases evaluate one reviewable graph. Automated update pull requests run the
full policy and test gates; security updates are prioritized but never merged
without compatibility checks.

Initial development tracks stable Rust. Before the first supported release, the
workspace declares an exact MSRV in `rust-version` and CI tests both MSRV and the
current stable toolchain. Raising MSRV is a documented compatibility change and
must not happen accidentally through a dependency update.

Dependencies use compatible semver requirements and the lockfile pins resolution.
Release builds use `--locked`. Yanked or unmaintained packages require replacement
or a time-bounded exception.

## Supply-chain gates

CI and release automation will enforce:

1. `cargo metadata --locked` succeeds and unapproved sources are absent.
2. `cargo deny check` enforces advisories, licenses, bans, and sources.
3. RustSec-compatible advisory scanning runs on the locked graph; suppressions
   name the advisory, affected reachability, compensating control, owner, and
   expiry.
4. Unused and duplicate dependencies are reviewed with workspace tooling; findings
   are not blindly auto-removed.
5. Release artifacts include an SPDX or CycloneDX SBOM, source revision, toolchain,
   and dependency lock digest.
6. Releases build in an isolated, network-denied build step after dependencies are
   fetched and verified, then receive provenance and signatures.
7. Container images contain the application binary, certificates, and only
   required runtime files, run as non-root, and are scanned before publication.

Generated Dockerfiles pin builder and runtime images by immutable digest for
release/certification builds. Automated updates preserve human-readable version
annotations and run the image build, vulnerability scan, health check, and
generated-application end-to-end suite. Base images follow the same source,
advisory, owner, and expiry rules as Rust crates. The final stage does not contain
Cargo, a shell, compiler, source tree, package manager cache, or build credentials
unless a documented platform requirement makes a narrower exception necessary.

The Phase 2 PostgreSQL Compose profile pins an explicit supported PostgreSQL major
and image digest. It is development infrastructure, is excluded from production
artifact claims, and uses disposable local credentials documented as unsuitable
for deployment.

Reproducible builds are a target: timestamps, paths, and build metadata are
controlled. Until byte-for-byte reproduction is verified on documented builders,
release notes must not claim reproducibility.

## License policy

Permissive licenses commonly compatible with Apache-2.0/MIT distribution are
allowed through explicit `cargo-deny` configuration. Copyleft, source-available,
unknown, or unlicensed code requires legal review before inclusion. License text
and notices are preserved in source and binary distributions as required.

## Unsafe code and native dependencies

Framework-authored code defaults to `#![forbid(unsafe_code)]`. If `unsafe` is
unavoidable, isolate it in a small crate/module, change the lint locally, document
invariants and soundness reasoning, add Miri/property/fuzz tests as applicable,
and obtain a dedicated review. Transitive `unsafe` is assessed by role and attack
surface rather than assumed safe.

Native dependencies are avoided unless they provide a material security or
platform benefit. They require supported-platform, cross-compilation, patching,
and minimal-container analysis.

## Exceptions and removal

Exceptions are checked-in records containing scope, reason, risk, compensating
controls, owner, approval date, and expiry. Expired exceptions fail CI. Removing a
dependency also removes features, configuration, documentation, and transitive
allowlist entries made solely for it.

See [ADR 0001](adr/0001-incremental-modular-workspace.md),
[ADR 0006](adr/0006-tokio-and-supervised-concurrency.md), and
[ADR 0007](adr/0007-sqlx-postgresql-first.md), and
[ADR 0008](adr/0008-docker-by-default.md).
