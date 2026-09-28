# ADR 0004: Explicit tenant context plus PostgreSQL RLS

- Status: Accepted
- Date: 2026-09-26
- Refined by: [ADR 0010](0010-session-and-tenant-authorization-context.md), [ADR 0011](0011-postgresql-runtime-migration-roles-and-rls-context.md)

## Context

An omitted tenant predicate can expose another organization's data. Thread-local
or global tenant state is unsafe in asynchronous request processing.

## Decision

Tenant-sensitive use cases and repository methods require a typed `TenantContext`
derived from an authenticated membership and authorization decision. There is no
default tenant. The PostgreSQL adapter starts a transaction and sets tenant and
principal values with transaction-local settings. RLS policies apply those values
to tenant tables; the application role cannot bypass RLS. Administrative
cross-tenant operations use a separate explicit capability and database role and
are fully audited.

Connection checkout/reset hooks and tests verify that context cannot leak across
pooled connections. Background jobs carry a signed/reference tenant identity and
re-authorize it when executed.

## Consequences

- Application mistakes face a database-enforced containment layer.
- Every tenant operation and test must provide context.
- Migrations, maintenance, analytics, and support tooling need carefully separated
  roles.
- RLS policy correctness and connection-pool hygiene become critical security
  gates.
