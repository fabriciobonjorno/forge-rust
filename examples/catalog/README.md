# catalog

A [Forge](https://github.com/fabriciobonjorno/forge-rust) application organized
around hexagonal architecture boundaries:

```text
src/domain          business rules; no I/O, HTTP, database or vendor types
src/application     commands, queries, use cases and ports
src/adapters        HTTP and other protocol translation
src/infrastructure  concrete database, cache, queue and telemetry mechanisms
src/bootstrap       composition root and process lifecycle
```

`tests/architecture.rs` fails when an inner ring imports an outer one.

## Development

The application requires PostgreSQL.
Start the development database with `docker compose up -d db`, then:

```bash
export FORGE_DATABASE_URL=postgres://app:app@127.0.0.1:5432/catalog_development
forge migrate                 # or: cargo run -- migrate
```

```bash
cargo run                     # serves on http://127.0.0.1:3000
curl http://127.0.0.1:3000/health/live
forge check && forge lint && forge test
```

Binary commands: `serve` (default), `healthcheck`, `version`, `help`.
`migrate` applies pending migrations from `migrations/` (embedded in the
binary) and `rollback` reverts the latest one. Create migrations with
`forge generate migration <name>`.
Configuration uses `FORGE_*` environment variables; unknown `FORGE_*` keys are
rejected at startup. Run `cargo run -- help` for the full list.

## Container

```bash
docker build -t catalog .
docker compose up --build     # PostgreSQL, migrations, then the application
```

The image is built with `cargo build --release --locked`, so commit
`Cargo.lock`. The runtime stage is distroless and runs as uid 65532; the Docker
health check runs `catalog healthcheck`.
`compose.yaml` runs a one-shot `migrate` service before starting the
application; its PostgreSQL credentials are for local development only.

## Product catalog API

This application is Forge's reference for the request flow
HTTP → handler → use case → domain → repository port → PostgreSQL adapter.

The Forge router matches exact paths only (no path parameters yet), so the
product identifier travels as the `id` query parameter.

| Request | Body | Success | Errors |
| --- | --- | --- | --- |
| `POST /products` | `{"sku","name","price":{"cents","currency"}}` | `201` product, `Location: /products?id=<uuid>` | `422` `invalid_sku` / `invalid_name` / `invalid_price` / `invalid_currency` / `invalid_json`; `409` `sku_taken` |
| `GET /products?id=<uuid>` | | `200` product | `404` `product_not_found`; `422` `invalid_id` |
| `GET /products[?limit=1..100][&after=<uuid>]` | | `200` `{"items":[...],"next_cursor":<uuid>\|null}`, newest first | `422` `invalid_page_limit` / `invalid_id` |
| `PATCH /products?id=<uuid>` | `{"expected_version": n, "name": "..."}` or `{"expected_version": n, "price": {...}}` | `200` product with the next `version` | `409` `concurrent_modification`; `404`; `422` `invalid_patch` (not exactly one change), `invalid_version`, `currency_mismatch` |

A product is
`{"id","sku","name","price":{"cents","currency"},"version","created_at","updated_at"}`.
SKUs are 3–32 ASCII letters, digits or `-`, stored upper-case and unique;
names are 1–120 characters after trimming; prices are non-negative integer
cents in a three-letter currency that never changes for a product. Unknown
JSON fields and unknown, repeated or malformed query parameters are rejected
with `422` (`invalid_json`, `invalid_query`). Errors use Forge's report shape
`{"code","category","message","metadata"?,"retryable"}`.

```bash
curl -i -X POST http://127.0.0.1:3000/products \
  -d '{"sku":"mug-red","name":"Red mug","price":{"cents":1299,"currency":"EUR"}}'
curl "http://127.0.0.1:3000/products?id=<id>"
curl -X PATCH "http://127.0.0.1:3000/products?id=<id>" \
  -d '{"expected_version":1,"name":"Big red mug"}'
curl "http://127.0.0.1:3000/products?limit=10"
```

### Layer walk-through, in request order

1. `src/bootstrap/mod.rs` — at startup, `compose` builds
   `PgProductRepository` and `ProductCatalog` once and registers the routes.
2. `src/adapters/http/mod.rs` — `routes` binds `POST`/`GET`/`PATCH /products`
   to handlers sharing the catalog through an `Arc`.
3. `src/adapters/http/products.rs` — handlers decode JSON
   (`forge::web::decode_json`) and the query string (`query.rs`), build a
   command or query from domain value objects, call one use case and render
   the result (`forge::web::json` / `forge::web::error`).
4. `src/application/commands`, `src/application/queries` — `CreateProduct`,
   `RenameProduct`, `ChangePrice`, `GetProduct`, `ListProducts`.
5. `src/application/use_cases/product_catalog.rs` — loads, applies domain
   behavior, checks `expected_version`, stores; returns `ForgeError`s.
6. `src/domain` — `Product`/`NewProduct` (entities), `Sku`, `ProductName`,
   `Money`/`Currency` (value objects) and `ProductError` with its mapping to
   `ForgeError`.
7. `src/application/ports/product_repository.rs` — the `ProductRepository`
   port, its contract and `RepositoryError` (`sku_taken`,
   `product_not_found`, `concurrent_modification`).
8. `src/adapters/persistence/pg_product_repository.rs` — visible SQL with
   SQLx runtime queries; optimistic `UPDATE ... WHERE id = $1 AND version = $2`
   and keyset pagination `WHERE id < $after ORDER BY id DESC`.
9. `migrations/*_create_products.{up,down}.sql` — the table and CHECK
   constraints mirroring the domain rules.

### Tests

```bash
cargo test                    # unit, architecture, use-case, handler tests
docker run -d --rm --name catalog-pg -e POSTGRES_PASSWORD=forge \
  -p 127.0.0.1:55441:5432 postgres:18
FORGE_TEST_DATABASE_URL=postgres://postgres:forge@127.0.0.1:55441/postgres \
  cargo test -- --include-ignored
```

`tests/support/contract.rs` is one repository contract suite run against the
in-memory double (`tests/catalog.rs`) and against PostgreSQL
(`tests/postgres.rs`), so the double used by use-case and handler tests
cannot drift from the real adapter.

`Cargo.toml` depends on Forge through a path outside this directory, so the
Docker build context above cannot see it; the container workflow applies to
applications that depend on a published Forge release.
