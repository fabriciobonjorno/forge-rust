# ADR 0006: Tokio with supervised structured lifecycles

- Status: Accepted
- Date: 2026-09-26

## Context

Forge requires asynchronous HTTP, database, streaming, jobs, and shutdown support.
Unowned spawned tasks and unbounded queues make failure, overload, and deployment
shutdown unpredictable.

## Decision

Use Tokio as the initial runtime behind Forge lifecycle contracts. Framework-owned
tasks must be registered with a supervisor, have a name, cancellation path,
failure policy, and drain deadline. Queues and concurrency are bounded. Admission
control and backpressure occur before spawning work. Request deadlines propagate
through application and adapter calls.

The runtime handle is not part of domain/application contracts. Direct detached
`tokio::spawn` is prohibited in generated production paths; low-level exceptions
must document ownership and shutdown invariants.

## Consequences

- The ecosystem is mature and operational behavior is explicit.
- Tokio remains an implementation dependency and runtime replacement is not an
  early goal.
- Supervisors and cancellation-safe code require deliberate testing.
- Blocking work must use a bounded blocking pool or external worker.
