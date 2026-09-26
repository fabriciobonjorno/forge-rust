# ADR 0001: Incremental modular Cargo workspace

- Status: Accepted
- Date: 2026-09-26

## Context

Forge covers runtime, HTTP, persistence, security, jobs, telemetry, OpenAPI, AI,
and developer tooling. A monolithic crate would couple dependency-heavy features,
while creating every suggested crate immediately would produce empty scaffolding
and unstable public APIs.

## Decision

Use a Cargo workspace with responsibility-oriented crates, but add a crate only
with the vertical phase that executes and tests its behavior. Keep dependency-light
contract crates separate from adapters with large transitive graphs. Applications
consume a curated prelude/facade and can opt into features without depending on
adapter internals.

A crate split requires a real consumer, a stable ownership boundary, and either an
independent release/feature need or meaningful dependency isolation.

## Consequences

- Features can avoid unrelated database, AI, or protocol dependencies.
- The public facade can remain coherent while internals evolve.
- More crates increase release coordination and compile graph management.
- The repository will not initially contain every crate shown in the architecture.
