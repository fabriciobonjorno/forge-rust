# Architecture Decision Records

ADRs record decisions whose reversal would affect public contracts, security, or
multiple crates. `Accepted` means the design is approved for implementation; it
does not mean implementation exists.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-incremental-modular-workspace.md) | Incremental modular Cargo workspace | Accepted |
| [0002](0002-hexagonal-boundaries.md) | Enforced hexagonal dependency boundaries | Accepted |
| [0003](0003-uuidv7-default-identifiers.md) | UUIDv7 typed identifiers by default | Accepted |
| [0004](0004-tenancy-and-rls.md) | Explicit tenant context plus PostgreSQL RLS | Accepted |
| [0005](0005-ai-provider-and-execution-boundary.md) | Separate AI generation from controlled execution | Accepted |
| [0006](0006-tokio-and-supervised-concurrency.md) | Tokio with supervised structured lifecycles | Accepted |
| [0007](0007-sqlx-postgresql-first.md) | SQLx and PostgreSQL-first persistence | Accepted |
| [0008](0008-docker-by-default.md) | Secure Docker packaging by default | Accepted (refined by 0009) |
| [0009](0009-rails-style-generated-delivery-assets.md) | Rails-style generated delivery assets (distroless, self-probe, CI) | Accepted |
| [0010](0010-session-and-tenant-authorization-context.md) | Server-side sessions and tenant-scoped authorization context | Accepted |

New ADRs use the next four-digit number. A changed decision receives a new ADR
that marks the prior record `Superseded`; accepted records are not rewritten to
hide history. A decision that is extended without being reversed receives a
new ADR that refines the prior one; the prior record stays `Accepted` and gains
only a `Refined by` note.
