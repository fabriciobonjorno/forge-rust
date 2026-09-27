# ADR 0007: SQLx and PostgreSQL-first persistence

- Status: Accepted
- Date: 2026-09-26
- Refined by: [ADR 0011](0011-postgresql-runtime-migration-roles-and-rls-context.md)

## Context

Forge needs transactions, JSONB, advisory locks, RLS, an outbox, durable jobs, and
query validation. A custom ORM would hide useful SQL and create a large correctness
surface.

## Decision

Use PostgreSQL as the first supported database and SQLx as the initial low-level
adapter. Expose Forge repository and transaction ports to applications, not SQLx
types. Keep SQL visible in infrastructure modules and use checked queries with
committed offline metadata where practical. Migrations are forward, immutable once
released, checksummed, and serialized with a PostgreSQL advisory lock.

Transactions are scoped by lifetimes and are owned by use cases. Savepoints are
explicit. External network calls do not occur inside database transactions unless
a documented consistency requirement and timeout budget justify them.

## Consequences

- PostgreSQL features can implement strong tenancy and delivery semantics.
- Another database requires a new adapter and capability review; portability is
  not promised.
- Offline query metadata adds reviewable generated artifacts and CI discipline.
- SQLx internals can evolve without becoming framework application API.
