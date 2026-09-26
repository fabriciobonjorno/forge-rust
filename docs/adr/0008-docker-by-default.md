# ADR 0008: Secure Docker packaging by default

- Status: Accepted
- Date: 2026-09-26
- Refined by: [ADR 0009](0009-rails-style-generated-delivery-assets.md)

## Context

A production binary is operationally useful only when teams can build and run it
predictably. Making container packaging an optional later exercise causes drift in
toolchains, configuration, health behavior, user identity, and release contents.
At the same time, requiring Docker for every local Cargo workflow would reduce
portability, and generating a database service before persistence exists would be
empty scaffolding.

## Decision

Every application created by `forge new` includes a reviewed `.dockerignore` and
multi-stage `Dockerfile`. The build uses a pinned Rust toolchain and `Cargo.lock`;
the final image contains the application binary, required runtime libraries, and CA
certificates only. It runs as a numeric non-root user, has a read-only-compatible
filesystem layout, receives predictable typed configuration at runtime, calls the
application liveness endpoint for its health check, and executes the binary
directly with signal propagation. Secrets never enter build arguments or layers.

Phase 1 owns this application-image contract and validates it end to end. Docker is
the generated default, not a requirement for ordinary `cargo` or `forge` host
workflows. Phase 2 adds a version-pinned PostgreSQL Compose profile for local
development after database support is real. That profile uses health checks,
explicit ports and configuration, a named volume, and no production credentials.
Compose is not the production deployment model.

## Consequences

- Fresh applications have a consistent container build and runtime from day one.
- Image security, size, caching, signal handling, and multi-architecture support
  become tested framework concerns.
- Teams can still deploy the same standalone binary with Kubernetes, systemd, bare
  metal, or later serverless-compatible adapters.
- The generated Dockerfile needs release discipline as Rust base and runtime images
  receive security updates.
- PostgreSQL Compose cannot be claimed until Phase 2 implements and tests it.
