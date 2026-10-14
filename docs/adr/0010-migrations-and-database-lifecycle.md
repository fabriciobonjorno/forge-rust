# ADR 0010: Migrations and database lifecycle

- Status: Accepted
- Date: 2026-09-26
- Refines: [ADR 0007](0007-sqlx-postgresql-first.md) and
  [ADR 0009](0009-rails-style-generated-delivery-assets.md) (both remain Accepted)

## Context

ADR 0007 chose PostgreSQL and SQLx. ADR 0009 defined what `forge new` generates
and deferred the Compose database service to Phase 2. Implementing Phase 2
required decisions that neither record covers:

- how schema migrations are packaged, applied, reverted and ordered relative to
  the application starting;
- when the process connects to PostgreSQL and how database health affects the
  liveness and readiness probes;
- whether a new application includes PostgreSQL by default;
- how driver errors reach clients without leaking SQL or constraint internals;
- which repository primitives Forge provides now, and which ports wait for the
  features that need them.

## Decision

### Embedded, reversible SQLx migrations

Migrations are plain SQL files in the application's `migrations/` directory,
named `<version>_<name>.up.sql` and `<version>_<name>.down.sql`.
`forge generate migration <name>` creates the pair with a UTC
`YYYYMMDDHHMMSS` version, requires a snake_case name, and refuses to overwrite
existing files. `sqlx::migrate!()` embeds the directory into the binary at
compile time. A generated `build.rs` (`cargo:rerun-if-changed=migrations`)
makes Cargo rebuild when a migration changes. The deployed artifact therefore
carries exactly the schema it was built and tested against. No SQL files need
to be shipped next to it.

`forge_db::Database::migrate` applies every pending migration and returns the
versions it applied. `Database::rollback` reverts only the most recently
applied migration (the highest applied version) and returns its version, or
`None` when nothing is applied. Both operations:

- hold SQLx's migration advisory lock on a single connection for their whole
  duration, so concurrent runners are serialized and the before/after
  bookkeeping is exact;
- use SQLx's `_sqlx_migrations` table and lock, so they interoperate with
  `sqlx migrate` and other SQLx-based runners;
- verify checksums of already-applied migrations. SQLx rejects a modified
  migration with `VersionMismatch`, and a migration that is applied in the
  database but missing from the binary with `VersionMissing`.

Each migration runs in its own transaction unless it opts out with SQLx's
`-- no-transaction` first line. Rolling back a migration that has no
`.down.sql` fails with `DbError::IrreversibleMigration` and changes nothing.

This refines ADR 0007's "forward, immutable once released" rule. Released
migrations stay immutable, and schema changes still ship as new migrations.
Down migrations exist so that a developer can iterate on an unreleased
migration and an operator can revert the latest deployment step deliberately.
They are not an automatic recovery mechanism.

### Explicit `migrate` and `rollback` commands; no migration on `serve`

An application that calls `App::postgres(sqlx::migrate!())` gains two binary
subcommands, `<binary> migrate` and `<binary> rollback`. They connect eagerly,
log each applied or reverted version, and exit `0` on success, including when
there is nothing to do. They exit `1` when the database cannot be reached or a
migration fails, `2` when the application did not declare PostgreSQL, and
`78` when `FORGE_DATABASE_URL` is missing or the `FORGE_*` configuration fails
validation. The
`forge migrate` and `forge rollback` CLI commands run
`cargo run --quiet -- migrate|rollback` for local development.

`serve` never applies migrations. Reasons:

- **Many replicas, one schema change.** With N replicas starting at once, every
  instance would race for the migration lock. A slow migration would block all
  of them, and startup probes and deadlines would kill instances mid-migration.
- **Privileges.** Schema changes need DDL rights that the serving role should
  not hold. A separate command can run with a separate, more privileged
  credential ([threat model](../threat-model.md): dedicated migration role).
- **Failure isolation.** A failed migration must stop the rollout visibly, as a
  failed Job or a failed Compose service. It must not become a crash loop of
  serving pods.
- **Operator control.** Expand/contract changes, backups and maintenance
  windows need migrations to be a deliberate, observable step.

Deployment runs migrations as a one-shot step before the new version serves
traffic:

- **Compose.** The generated `compose.yaml` has a one-shot `migrate` service. It
  uses the application image with `command: ["migrate"]`, depends on
  `db: service_healthy`, and `app` depends on
  `migrate: service_completed_successfully`. The order is db, then migrate,
  then app.
- **Kubernetes.** Run `<binary> migrate` from the same image as a Job (for
  example a Helm pre-upgrade hook or a pipeline step that waits for completion)
  or as an init container. A Job runs once per rollout. An init container runs
  once per Pod, which is safe because runners are serialized by the advisory
  lock and a no-op run exits `0`, but it ties migration credentials and
  duration to every Pod start. The [deployment guide](../deployment.md) has
  examples.

### Lazy pool; readiness depends on the database, liveness does not

`App::postgres` makes `FORGE_DATABASE_URL` mandatory for `serve`, which exits
`78` without it. `serve` creates the pool with `Database::connect_lazy`, so the
process starts, binds and answers `/health/live` while PostgreSQL is still
unavailable. Connections open on first use. The pool is bounded by
`FORGE_DATABASE_MAX_CONNECTIONS` (default `10`), and an acquire waits at most
`FORGE_DATABASE_ACQUIRE_TIMEOUT_SECS` (default `5`), including connecting.

`/health/ready` runs a `database` readiness check (`SELECT 1`) with a
two-second timeout on every request. It answers
`200 {"status":"ready","checks":{"database":"ok"}}` or
`503 {"status":"unavailable","checks":{"database":"unavailable"}}`. The
response never includes the error, which is logged as a warning.
`/health/live` never runs checks. A database outage takes instances out of
rotation instead of restarting them in a loop, and they return to rotation
when the database recovers. The pool is closed, with a five-second bound,
after the server drains on shutdown.

### PostgreSQL included by default in `forge new`

`forge new` now generates a database-enabled application:

- `sqlx` is a direct dependency (`sqlx::migrate!()` expands to `::sqlx` paths)
  with the `postgres`, `runtime-tokio`, `macros`, `migrate`, `uuid`, `chrono`
  and `json` features;
- `build.rs`, `migrations/.gitkeep` and `.postgres(sqlx::migrate!())` in
  `src/bootstrap/mod.rs`;
- in `compose.yaml`, a `db` service (`postgres:18`, `pg_isready` health check,
  named volume `db-data`, port published on `127.0.0.1:5432`, credentials
  marked development-only), the one-shot `migrate` service and
  `FORGE_DATABASE_URL` on `app`;
- when both Docker and CI assets are generated, the CI workflow starts the
  whole Compose stack, waits for the application to be healthy, requires
  `/health/ready` to succeed, and checks for a graceful stop.

`--skip-database` omits all of this. The `forge` dependency is then declared
with `default-features = false` (without the `postgres` feature), `compose.yaml`
contains only the application, and the CI job smoke-tests the image alone.

The framework default follows what most long-lived services need, as Rails
defaults to a database. Skipping stays explicit and cheap.

### Error classification

`DbError` is built from `sqlx::Error` by inspecting the SQLSTATE and error
kind. It converts into a `ForgeError` with a stable code and a caller-safe
message. SQL text and driver messages never reach the client. A constraint name
is attached only as `constraint` metadata. `ErrorCategory::Infrastructure`
maps to HTTP `503` and `Internal` to `500`.

| Source | `DbError` | Code | Category (HTTP) | Retryable |
| --- | --- | --- | --- | --- |
| SQLSTATE `23505` | `UniqueViolation` | `already_exists` | Conflict (409) | no |
| SQLSTATE `23503` | `ForeignKeyViolation` | `reference_conflict` | Conflict (409) | no |
| SQLSTATE `23514`, `23502` | `CheckViolation` | `constraint_violation` | Validation (422) | no |
| SQLSTATE `40001`, `40P01` | `SerializationFailure` | `transaction_conflict` | Infrastructure (503) | yes |
| `RowNotFound`, or passed to `expect_rows` | `NotFound` | `not_found` | NotFound (404) | no |
| passed to `expect_rows` (stale version) | `Conflict` | `concurrent_modification` | Conflict (409) | no |
| pool acquire timeout | `PoolTimeout` | `database_busy` | Infrastructure (503) | yes |
| I/O, TLS, pool closed, worker crashed; SQLSTATE class `08`, `57P01`, `57P02`, `57P03`, `53300` | `Unavailable` | `database_unavailable` | Infrastructure (503) | yes |
| migration failure | `Migration` | `database_migration_failed` | Internal (500) | no |
| rollback without `.down.sql` | `IrreversibleMigration` | `database_migration_failed` | Internal (500) | no |
| invalid URL or pool settings | `Configuration` | `database_misconfigured` | Internal (500) | no |
| anything else | `Query` | `database_error` | Internal (500) | no |

A migration error caused by an unreachable database or a pool timeout keeps
the `Unavailable` or `PoolTimeout` classification, so operators can tell
connectivity problems from a broken migration.

### Persistence primitives now; generic unit-of-work later

Phase 2 ships primitives, not a repository framework:

- `forge_core::Id<T>` binds and decodes as PostgreSQL `uuid`, and decoding
  rejects non-v7 UUIDs. `forge_core::Version` binds as `BIGINT` and decoding
  rejects values below 1. Both use the `sqlx` feature of `forge-core`, which
  `forge-db` enables.
- Optimistic locking: `Version::INITIAL`, `Version::next`, and
  `expect_rows(result, DbError::Conflict)` on
  `UPDATE ... WHERE id = $n AND version = $m`.
- Cursor pagination over UUIDv7 ids: `PageLimit` (1 to 100, default 20),
  `PageRequest { limit, after }`, and `Page::from_overfetched` with
  `WHERE id < $after ORDER BY id DESC LIMIT limit.fetch_size()`.
- `Database::begin` returns a `forge_db::Transaction`, which rolls back when
  dropped without `commit`. `advisory_xact_lock` and `advisory_key` (64-bit
  FNV-1a of a name) provide transaction-scoped advisory locks.
- `forge::db::sqlx` re-exports SQLx for infrastructure adapters, and
  `forge_db::testing::TestDatabase` provides disposable databases for tests.

This refines ADR 0007's "expose Forge repository and transaction ports, not
SQLx types". SQLx types are visible to the adapter and infrastructure rings,
where SQL lives. The generated architecture test still rejects `sqlx` imports
in `src/domain` and `src/application`. Applications define their own
repository port traits in `application/ports` and implement them in
`adapters/persistence`.

A generic, framework-provided unit-of-work port is deferred to Phase 4, where
the transactional outbox and durable jobs are the first framework features that
must enlist in an application's transaction. Phase 3 tenancy also needs
transaction-local settings. Designing the port before those consumers exist
would probably produce the wrong abstraction. Until then:

- an operation that must be atomic across several tables is one port method,
  and its PostgreSQL adapter opens a single transaction with `Database::begin`
  and commits it before returning;
- application code never receives or stores a `Transaction`;
- external network calls stay outside the transaction (ADR 0007).

## Consequences

- One binary carries the server, its schema and the tools to apply and revert
  it. Deployments need a pre-serve migration step: a Compose service, a
  Kubernetes Job or init container, or a pipeline command.
- A started process with an unreachable database is live but not ready. Probes
  must use `/health/live` for restarts and `/health/ready` for traffic.
- New applications need PostgreSQL to `serve`, and their CI needs Docker for
  the Compose smoke test. `--skip-database` restores the Phase 1 shape.
- The `migrate`/`rollback` subcommands, their exit codes, the `FORGE_DATABASE_*`
  variables, the readiness JSON and the error codes above are public contracts
  that need compatibility review when they change.
- Down migrations are only as good as their authors. A rollback can destroy
  data (for example `DROP TABLE`), so production reverts need the same review
  as forward migrations.

## Known risks

- **SQLx semi-internal API.** `forge-db` reads `Migrator::table_name` and
  `Migrator::create_schemas` and calls the `Migrate` trait's lock and
  bookkeeping methods directly, to report applied versions and revert exactly
  one migration. `table_name` and `create_schemas` are public but
  `#[doc(hidden)]` in SQLx 0.9, so a SQLx minor release can change them. The
  PostgreSQL integration tests for migrate and rollback are the guard, and
  SQLx upgrades must run them.
- **Pool timeout can mask the root cause.** When PostgreSQL is unreachable,
  SQLx keeps retrying the connection until the acquire timeout. The error can
  then surface as `PoolTimeout` (`database_busy`) without the underlying I/O
  or authentication error, instead of `Unavailable`. Both are retryable `503`
  responses, but the log may not say why the connection failed. Operators
  should check reachability, credentials and TLS directly when `migrate` or
  readiness reports timeouts.
- **Init-container migrations** run on every Pod start and extend startup by
  the lock wait. Prefer a Job for large fleets or long migrations.
