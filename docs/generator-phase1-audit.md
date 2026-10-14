# Phase 1 Audit and Plan: Forge Generators

## 1. Audit Snapshot and Evidence Boundary

This file records an audit snapshot, not proof that implementation or tests are
complete. Git state and validation results change over time; check the current
worktree and CI before relying on them. The resource generator is still partial:
it now emits a domain entity, permission-aware application port/use cases,
PostgreSQL adapter/migration, and an ignored database-backed CRUD/RLS test. It
still has no authenticated HTTP layer. The HTTP router now executes named path
captures using the existing segment trie, typed percent-decoded path extraction,
static-route precedence and ambiguity rejection. No claim in this document
substitutes for an executed command or a reviewed diff.

---

## 2. Current CLI Capabilities Matrix

| Command | Status | Description | Gaps |
|---------|--------|-------------|------|
| `forge id` | ✅ Implemented | Prints UUIDv7 | None |
| `forge new <name>` | ✅ Implemented | Creates hexagonal app with Docker, CI, PostgreSQL | Presets missing; no `api`/`web`/`saas` composition |
| `forge generate migration` | ✅ Implemented | Creates reversible `.up.sql`/`.down.sql`; `--version` makes paths reproducible | Default timestamp path remains provisional |
| `forge check/test/format/lint/build` | ✅ Implemented | Thin Cargo wrappers | None |
| `forge generate resource` | 🟡 Partial | Typed entity, exact-permission tenant-aware use cases, PostgreSQL adapter/migration and generated DB test | Authenticated resource HTTP handlers and automatic DB test provisioning |
| `forge generate page` | ✅ Implemented | Escaped server-rendered page and GET route registration | General templates, layouts and forms remain open |
| Presets (`api`, `web`, `saas`) | ❌ Missing | Composed capabilities | None implemented |

---

## 3. Architecture & Generator Infrastructure State

| Layer | Status | Notes |
|-------|--------|-------|
| Hexagonal enforcement | ✅ | `tests/architecture.rs` scans imports; CI gate |
| PostgreSQL + migrations | ✅ | Reversible, `app_migrator`/`app_runtime` roles |
| RLS + Tenancy | ✅ | Fail-closed, transaction-local `forge.tenant_id`/`forge.principal_id` |
| Authentication | 🟡 | PostgreSQL scaffold composes CSRF/login/logout; `bootstrap-account` creates the first tenant, principal, active owner membership, and audit records atomically. `forge-tenancy::authorize_tenant_context` resolves current membership and exact RBAC permission; generated apps now have `TenantRequestAuthorizer` to combine session, unsafe-method Origin/CSRF, explicit tenant header and that resolver. Resource routes and a generated owner-role policy remain open |
| Session rehash | ✅ | Optimistic version + concurrent change detection |
| Audit sink | ✅ | Append-only, runtime role cannot read/update/delete |
| Jobs/Events crates | ✅ Skeleton | `forge-jobs`, `forge-events` exist but no generators |
| OpenAPI | ❌ | Not started |
| AI subsystem | ❌ | Ports only (`forge-ai` not created) |

---

## 4. Generator Template State (51 templates)

**Always rendered**: Core app structure (Cargo.toml, rings, architecture test, README)
**Database group**: Migrations (audit, identity, login throttle), HTTP auth adapter and infrastructure modules (audit, auth, tenancy, security, database), `build.rs`, `.gitattributes`
**DatabaseDocker group**: `docker/postgres/init.sql` (least-privilege roles)
**Docker group**: `Dockerfile`, `.dockerignore`, `compose.yaml`, `tests/container.rs`
**CI group**: `.github/workflows/ci.yml`, `.github/dependabot.yml`

**Template conditionals**: `%%IF_DOCKER%%`, `%%if postgresql%%`, `%%if mysql%%`, `%%if sqlite%%` — MySQL/SQLite branches always `false` currently.

**Reserved modules**: 19 empty `mod.rs` files with doc comments (domain/entities, application/commands, etc.) — created for database apps but skipped for infra modules when database enabled.

---

## 5. Resource Vertical Slice Contract and Remaining Gaps

### Critical Missing: `forge generate resource`

This is the **primary vertical generator** per the product vision. Completion requires a complete, tenant-scoped resource:

| Artifact | Ring | Description |
|----------|------|-------------|
| Typed ID | Domain | `ResourceId(uuid::Uuid)` with UUIDv7 validation |
| Entity | Domain | Nominal struct with invariants |
| Value objects | Domain | E.g., `ResourceName`, `ResourceStatus` enum |
| Domain events | Domain | `ResourceCreated`, `ResourceUpdated` |
| Domain errors | Domain | Typed business rule violations |
| Command | Application | `CreateResource`, `UpdateResource`, `DeleteResource` |
| Query | Application | `GetResource`, `ListResources` (cursor pagination) |
| Use cases | Application | Orchestration + transaction boundary |
| Ports | Application | `ResourceRepository`, `TenantResourceRepository` |
| HTTP adapter | Adapters | Routes, extractors, validation, response mapping |
| Persistence adapter | Adapters | SQLx implementation of `TenantResourceRepository` |
| Migration | Infra | Reversible SQL with RLS policies |
| Tests | Tests | Architecture, contract, integration |

**Security requirements for the remaining vertical slice**:
- Repository methods must receive authorized `TenantContext` for tenant-scoped resources.
- HTTP handlers must derive that context from authenticated and authorized request state.
- Tenant-scoped transactions must set `forge.tenant_id` before accessing data.
- Keep the current generator's domain entity and RLS migration clearly labeled as partial until these pieces exist.

---

## 6. Phase/Dependencies for Next Increment

```
Phase 1 (complete): Runtime, CLI, config, HTTP, health, Docker, CI
Phase 2 (complete): UUIDv7, PostgreSQL, migrations, repositories, transactions
Phase 3 (in progress): Auth, authorization, tenancy, RLS, audit
Phase 4 (next target): Jobs, scheduler, events, outbox, idempotent consumers
Phase 5: Observability (logs, traces, metrics, correlation)
Phase 6: OpenAPI, code generation, macros
Phase 7: AI providers, routing, streaming, execution pipeline
```

**Next increment fits between Phase 3 and Phase 4**: Resource generation is the composition surface that ties Phase 2/3 capabilities together into a usable application building block.

---

## 7. Implemented Resource Generator Slice

The HTTP router supports whole-segment captures such as `/products/{id}`.
`Request::path_param_as` percent-decodes captures before parsing them into Rust
types. Static routes take precedence, equivalent capture patterns are rejected
as ambiguous, and method mismatches include `Allow`. This is transport plumbing:
the generated resource still has no HTTP routes until its handlers can compose
authenticated session state, CSRF checks, `TenantRequestAuthorizer` and the
exact permission required by each use case. The authorizer is transport plumbing
only; no resource routes are registered by it yet.

Current command:

```sh
forge generate resource Project --fields "name:string,status:enum"
forge generate resource PlatformSetting --fields "key:string,value:text" --global
```

It generates a UUIDv7-backed domain entity, registers domain and application
modules, emits a repository port plus create/get/update/delete/page use cases,
and creates reversible PostgreSQL migrations. The default migration is
tenant-scoped and enables/forces RLS using the transaction-local
`forge.tenant_id` setting. `--global` creates a global table.

The generated entity carries the typed `forge::core::Version` corresponding to
the migration's optimistic-locking column, supports adapter-side state
reconstitution from already validated identifiers, and exposes `revise` for
changing declared fields without allowing callers to mutate identity, tenant
ownership, or version. Tenant-scoped repository methods require `TenantContext`;
the generator does not accept a raw tenant ID in those contracts.

This remains an incomplete web CRUD vertical slice: authenticated HTTP routes
and full page templates/layouts remain open. The generated database test is opt-in and only
proves its specific CRUD/security contract when run. Never expose these use cases over HTTP until authenticated
request state can supply an authorized `TenantContext`.

---

## 8. Remaining Generator Work

1. Generate authenticated HTTP handlers and register routes without overwriting application code.
2. Wire the generated PostgreSQL contract test into the complete E2E database path and extend its edge cases.
3. Extend page generation with a reviewed server-side template/layout system and forms only after choosing the view adapter.
4. Add `api`, `web`, and `saas` presets only when each composes implemented capabilities.

---

## 9. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Template complexity explosion | High | Single `ResourceGenerator` orchestrator; shared field parser; composable template partials |
| Tenant-scoped vs global confusion | High | Tenant-scoped by default; require explicit `--global` to omit RLS |
| RLS migration correctness | High | Keep policy aligned with the app transaction helper and verify against PostgreSQL |
| Generated code architectural drift | Medium | Reuse `tests/architecture.rs` as contract test; golden `hello-forge` pattern |
| Field parser limitations | Medium | Start with `string`, `enum`, `uuid`, `bool`, `text`; defer complex types |
| Duplicate template logic | Medium | Extract common patterns (`repository_impl`, `use_case_impl`) to macros |

---

## 10. Acceptance Criteria for This Increment

| Criterion | Verification |
|-----------|--------------|
| Entity and migration generation is safe | CLI integration test plus identifier and overwrite cases |
| Generated entity compiles inside a generated app | `cargo check` on generated app |
| Tenant-scoped migration fails closed | PostgreSQL integration test with no tenant and cross-tenant cases |
| Complete CRUD resource is delivered | Generated app compile + route, persistence and tenant contract tests |

---

## 11. Next Step

Continue the resource vertical slice with generated authenticated HTTP routes
and integration tests. Do not add MySQL or SQLite as
supported primary databases without the separate capability review required by
`AGENTS.md` and ADR 0007.

---

*Status: audit refreshed 2026-10-04. Explicit `--version` makes migration previews repeatable; migration/resource/page targets reject symlink traversal under the app root. Initial tenant and owner membership bootstrap are implemented locally. A reusable async tenant authorization resolver reloads membership on each decision, rejects missing/inactive/mismatched memberships, and applies exact deny-by-default RBAC policy before producing `TenantContext`. Generated apps also expose `TenantRequestAuthorizer`, which composes session validation, Origin/CSRF checks for unsafe methods, a unique typed tenant header and the resolver. It is not yet connected to generated resource routes, and no owner-role grants are generated. Authenticated resource HTTP handlers, generated RBAC policy, full templates/layouts, automatic database test provisioning, and presets remain unfinished.*
