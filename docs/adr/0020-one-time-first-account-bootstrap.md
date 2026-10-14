# ADR 0020: One-time first-account bootstrap

- Status: Accepted
- Date: 2026-10-04
- Refines: [ADR 0012](0012-structured-append-only-audit-evidence.md), [ADR 0013](0013-persistent-principals-sessions-and-memberships.md), and [ADR 0014](0014-argon2-session-cookie-and-csrf.md)

## Context

Generated PostgreSQL apps can authenticate a persisted principal but start with
empty identity and tenant tables. The least-privilege runtime role must not gain
principal, tenant, or membership INSERT permission merely to make first login
possible.

## Decision

Database-enabled generated apps provide the explicit one-shot command
`bootstrap-account <login> <tenant-name>`. The operator invokes it separately
from the serving process with `FORGE_MIGRATION_DATABASE_URL`; the command reads
and confirms the password through a no-echo terminal prompt, applies the
generated Argon2id policy, and writes the first tenant, principal, active
`owner` membership, and `principal.bootstrap` / `tenant.bootstrap` system audit
events in one transaction. The role name grants no authority on its own;
application RBAC policy must explicitly map it to permissions.

The adapter obtains a transaction-scoped PostgreSQL advisory lock before checking
that `forge_principals` and `forge_tenants` are empty. This serializes concurrent
invocations of the supported command. It refuses to run when either table
already contains rows. Login, tenant name, password, and password hash are not
written into audit metadata or logs. The password buffers are zeroized after
use.

The migration credential must not be injected into the long-running server.
The bootstrap creates an active `owner` membership for the first tenant, but the
role is not privileged by itself: the application's deny-by-default RBAC policy
must explicitly grant each permission. Bootstrap is not a general
user-management or tenant-administration API.

## Consequences

- The first tenant and login can be created without weakening runtime table privileges.
- Parallel supported bootstrap invocations cannot both observe an empty table.
- Either audit failure rolls back tenant, membership and principal creation.
- The operator must manage the migration credential separately and unset it
  before starting the server.
- This command is not a general user-management or tenant-administration API.
