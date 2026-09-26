# ADR 0009: Rails-style generated delivery assets

- Status: Accepted
- Date: 2026-09-26
- Refines: [ADR 0008](0008-docker-by-default.md) (which remains Accepted)

## Context

ADR 0008 decided that every generated application ships a secure multi-stage
Docker build. Implementing Phase 1 left several questions open: which runtime
image to use, how a health check runs inside it, what other delivery files a new
application should receive, and how strictly configuration is validated.

Rails 8 is a useful reference. `rails new` generates a production `Dockerfile`,
`.dockerignore`, `bin/docker-entrypoint`, a CI workflow and a Dependabot
configuration by default, and offers flags to skip them. A new application is
buildable, checkable and deployable as a container from its first commit.
Forge wants the same default for Rust services.

The smallest practical runtime images, such as distroless, contain no shell and
no `curl`. A `HEALTHCHECK CMD curl ...` or a shell entrypoint script cannot
work there, while adding a shell or `curl` only for probing brings back the
attack surface the image is meant to remove.

Configuration typos in containerized deployments are easy to miss. An ignored
`FORGE_BIDN` silently leaves the default loopback bind in place, and the
container never becomes reachable.

## Decision

`forge new <name>` generates, by default:

1. **A Cargo package.** It has no `[workspace]` table, uses the hexagonal layout
   `src/{domain,application,adapters,infrastructure,bootstrap}`, names the
   library `app` and the binary after the package, and includes a
   `rust-toolchain.toml` pinned to the framework toolchain. Dependencies are
   `forge` from the repository's Git tag `v<cli version>` (or a path dependency
   with `--forge-path`, similar to `rails new --dev`) and `serde`.
2. **`tests/architecture.rs`.** A real test that fails when `src/domain` imports
   application, adapters, infrastructure, bootstrap, `forge::http`, or
   protocol/runtime crates such as `tokio`, `hyper` and `sqlx`, or when
   `src/application` imports adapters, infrastructure, bootstrap or
   `forge::http`.
3. **Docker assets** (skippable with `--skip-docker`):
   - **`Dockerfile`.** A multi-stage build.
     - Builder: `rust:<pinned>-slim-trixie` (Debian 13, version set by
       `ARG RUST_VERSION`), BuildKit cache mounts, and
       `cargo build --release --locked` against the committed `Cargo.lock`.
     - Runtime: `gcr.io/distroless/cc-debian13:nonroot`, with no shell or
       package manager and numeric uid `65532`.
     - `ENV FORGE_ENV=production FORGE_BIND=0.0.0.0:3000`, `EXPOSE 3000`, and
       an exec-form `HEALTHCHECK` running `<binary> healthcheck`.
     - `ENTRYPOINT` is the binary itself, so it is PID 1 and receives
       `SIGTERM`.
   - **`.dockerignore`.** Excludes `rust-toolchain.toml`, so the builder uses
     its preinstalled toolchain instead of downloading components.
   - **`tests/container.rs`.** Fails when the Dockerfile's `ARG RUST_VERSION`
     differs from `rust-toolchain.toml`.
   - **`compose.yaml`.** For local convenience only: build, port published on
     `127.0.0.1:3000`, `stop_grace_period: 20s`, `read_only`, `tmpfs` `/tmp`,
     `cap_drop: ALL`, `no-new-privileges`, `init`. No database service until
     Phase 2.
4. **CI assets** (skippable with `--skip-ci`): `.github/workflows/ci.yml` (format
   check, Clippy with warnings denied, tests, Docker image build) and
   `.github/dependabot.yml` (cargo, docker, github-actions).
5. **A lockfile.** `cargo generate-lockfile` runs after generation (skippable
   with `--skip-lockfile`). If it fails, the application is kept and a warning
   explains how to create the lockfile before `docker build`.

Rails' `bin/docker-entrypoint` has no generated equivalent. The binary is the
entrypoint, and runtime preparation belongs in the application's typed
startup, not a shell script.

The runtime owns the container contract:

- **Health check.** `forge::App` binaries implement a `healthcheck` subcommand
  that sends `GET /health/live` to the loopback address on the configured port
  and exits `0` or `1`. The image health check needs nothing beyond the
  application binary. The `serve` (default), `version` and `help` subcommands
  complete the binary interface.
- **Fail-closed configuration.** Unknown `FORGE_*` environment variables are
  rejected at startup in every environment, not only production. This refines
  the "unknown keys fail in production mode" rule in the
  [architecture](../architecture.md#configuration-and-secrets) for the
  environment-variable source.

Forge does **not** generate a deployment tool configuration (such as Kamal in
Rails 8) yet. Every current option would add a non-Rust toolchain dependency to
new applications. Deployment-tool integration is deferred to a later ADR.
Deployment guidance is documented instead ([deployment](../deployment.md)).

`examples/hello-forge` is the committed output of
`forge new hello-forge --skip-ci --skip-lockfile --forge-path ../../crates/forge`.
It is a standalone package excluded from the framework workspace, with a
separately generated, committed `Cargo.lock`. A golden test keeps it in sync
with the generator, and framework CI runs format, Clippy and tests against it. `scripts/e2e-generated-app.sh`
exercises a generated application as a host binary and as a container.

## Consequences

- New applications can be built, tested, scanned and run as hardened containers
  immediately. CI and dependency updates are configured from the first commit.
- Base images are referenced by tag in generated files. Release and
  certification builds must pin digests per the
  [dependency policy](../dependency-policy.md). Generated Dependabot
  configuration proposes updates, but updating the generator's own defaults is
  a framework release task.
- The distroless runtime removes interactive debugging with `docker exec sh`.
  Operators use logs, health endpoints or ephemeral debug containers.
- The `healthcheck` subcommand and the fail-closed variable check are public
  runtime contracts and need compatibility review when they change. Deployments
  with stale or misspelled `FORGE_*` variables fail at startup instead of running
  misconfigured.
- Generated applications depend on a Git tag until Forge is published to a
  registry. Tags are mutable, while the generated `Cargo.lock` records the
  resolved commit. Before the first tag exists, the default dependency cannot
  resolve, and contributors use `--forge-path`.
- A `--forge-path` pointing outside the application is not visible to
  `docker build`. The end-to-end script vendors the framework to exercise the
  container path.
- The generator now owns more files (Docker, Compose, CI, Dependabot), which
  increases golden-fixture and end-to-end maintenance.
- Teams that deploy without containers or GitHub can opt out with
  `--skip-docker` and `--skip-ci`. The runtime contracts (configuration, health,
  signals) stay the same for Kubernetes, systemd and bare metal.
