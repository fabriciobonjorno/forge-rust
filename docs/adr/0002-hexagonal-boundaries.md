# ADR 0002: Enforced hexagonal dependency boundaries

- Status: Accepted
- Date: 2026-09-26

## Context

Framework convenience can cause transport, persistence, and vendor SDK types to
spread into business logic. Code review alone does not reliably prevent erosion.

## Decision

Generated applications use domain, application, adapters, infrastructure, and
bootstrap boundaries. Domain depends on no outer ring; application depends only
on domain and dependency-light Forge contracts; concrete adapters implement ports;
bootstrap alone selects implementations.

CI will combine Cargo dependency-graph rules with source-level forbidden-import
tests and compile-fail fixtures. Protocol types must be mapped at adapter edges.
Exceptions require an ADR, a narrow facade, and an explicit removal or stability
plan.

## Consequences

- Business behavior is independently testable and provider replacement is local.
- Mapping code and trait contracts add ceremony.
- `pub` surface and feature leakage require continuous review.
- Architecture tests become a release gate rather than advisory documentation.
