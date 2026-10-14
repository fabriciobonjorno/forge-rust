# AI authoring guide for `hello-forge`

This guide is generated from the same capability manifest as this app.
If it disagrees with executable code, trust the code and report the drift.

## Project map

- `src/domain`: business rules, owned types, and typed domain errors. No I/O.
- `src/application`: use cases and ports. Keep dependencies pointed inward.
- `src/adapters`: HTTP and view translation, plus protocol adapters.
- `src/infrastructure`: concrete database and runtime mechanisms.
- `src/bootstrap`: composition root and process lifecycle.
- `tests`: executable behavior and architecture constraints.

## Capabilities in this scaffold

- Database adapter: `disabled`.
- Authentication scaffolding: not scaffolded.
- Tenant membership and RLS scaffolding: not scaffolded.
- Docker: Dockerfile and Compose are generated; inspect them before changing runtime assumptions.
- Jobs/events: false.
- AI providers or tool execution: false.

### Generator maturity

- `forge generate migration` — available: Creates a reversible SQL migration pair. Applying it requires a configured database integration.
- `forge generate resource` — not_available: Requires a database-enabled application.
- `forge generate page` — available: Creates a server-rendered HTML view with escaped text values and automatically registers a GET route; no view service or framework-owned template dependency is required.

`available` means the documented artifacts are generated; `partial`
means inspect the description and fill the missing behavior explicitly.

## Change workflow

1. Read the relevant code, tests, migrations, and ADRs before editing.
2. State the behavior change and affected rings; add the smallest complete
vertical slice instead of a placeholder abstraction.
3. Prefer generator previews before writing; migration paths contain a
provisional timestamp. Generators must refuse user-file collisions.
4. Add or update focused regression tests, then run the narrowest relevant
checks before the documented workspace gates.
5. Review the final diff and report executed checks separately from checks
that were skipped or blocked.

## Security and authority

AI suggestions and tool arguments are untrusted data. A model response is
never authorization. Keep intent, policy, authorization, validation,
approval, execution, and audit separate. Do not expose secrets in prompts,
generated context, logs, command output, or tests. Never grant an agent
unrestricted filesystem, process, network, database, or production access.

## Useful commands

- `forge capabilities --format json` — read the versioned capability contract.
- `forge doctor --format json` — inspect static project/environment findings.
- `forge doctor --probe-database` — optional bounded TCP reachability only;
it does not authenticate or query data.
- `forge generate page <Name> --dry-run` — preview page output without writes.
- `forge generate resource <Name> --fields <fields> --dry-run` — preview a
resource where the database capability is enabled.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features --locked -- -D warnings`, `cargo test --workspace
--all-targets --locked`, `cargo test --workspace --doc --locked`.

## Environment notes

- This scaffold has no database adapter or migrations. Do not add persistence by placing SQL in the domain or application rings.

Dockerfile and Compose are generated; inspect them before changing runtime assumptions
