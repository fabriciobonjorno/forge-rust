//! Versioned, machine-readable capabilities embedded in generated projects.

use serde::{Deserialize, Serialize};

pub(crate) const CAPABILITY_SCHEMA_VERSION: u32 = 1;

/// Schema describing the framework features actually scaffolded into an app.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CapabilityManifest {
    /// Version of this JSON contract.
    pub(crate) schema_version: u32,
    /// Forge CLI version that generated the project.
    pub(crate) forge_version: String,
    /// Cargo package name.
    pub(crate) application: String,
    /// Capabilities selected by `forge new`.
    pub(crate) features: Features,
    /// Generator commands and their implementation status.
    pub(crate) generators: Vec<GeneratorCapability>,
}

/// Framework capabilities included in the generated project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Features {
    /// Primary database adapter, or `null` for a database-free scaffold.
    pub(crate) database: Option<String>,
    /// PostgreSQL-backed authentication/session scaffolding.
    pub(crate) authentication: bool,
    /// Explicit tenant membership and RLS scaffolding.
    pub(crate) tenancy: bool,
    /// Append-only security audit scaffolding.
    pub(crate) audit: bool,
    /// Docker and Compose assets.
    pub(crate) docker: bool,
    /// GitHub Actions and Dependabot assets.
    pub(crate) ci: bool,
    /// Whether a functional jobs/events implementation is scaffolded.
    pub(crate) jobs_and_events: bool,
    /// Whether executable AI providers and tool execution are scaffolded.
    pub(crate) ai_providers_and_execution: bool,
}

/// A generator exposed by the CLI and the degree of implementation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct GeneratorCapability {
    /// Stable CLI command name.
    pub(crate) command: String,
    /// Whether this generator is usable as a complete feature.
    pub(crate) status: GeneratorStatus,
    /// Short description of the generated artifacts or missing scope.
    pub(crate) description: String,
}

/// Maturity state emitted in the capability contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GeneratorStatus {
    /// Produces the documented executable artifact set.
    Available,
    /// Produces a limited subset; consumers must read the description.
    Partial,
    /// The command is not available for this project or not implemented.
    NotAvailable,
}

impl GeneratorStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Partial => "partial",
            Self::NotAvailable => "not_available",
        }
    }
}

impl CapabilityManifest {
    pub(crate) fn for_application(
        application: &str,
        database: bool,
        docker: bool,
        ci: bool,
    ) -> Self {
        Self {
            schema_version: CAPABILITY_SCHEMA_VERSION,
            forge_version: env!("CARGO_PKG_VERSION").to_owned(),
            application: application.to_owned(),
            features: Features {
                database: database.then(|| "postgresql".to_owned()),
                authentication: database,
                tenancy: database,
                audit: database,
                docker,
                ci,
                jobs_and_events: false,
                ai_providers_and_execution: false,
            },
            generators: vec![
                GeneratorCapability {
                    command: "forge generate migration".to_owned(),
                    status: GeneratorStatus::Available,
                    description: "Creates a reversible SQL migration pair. Applying it requires a configured database integration.".to_owned(),
                },
                GeneratorCapability {
                    command: "forge generate resource".to_owned(),
                    status: if database {
                        GeneratorStatus::Partial
                    } else {
                        GeneratorStatus::NotAvailable
                    },
                    description: if database {
                        "Creates a versioned entity, exact-permission tenant-aware use cases, PostgreSQL adapter/migration, and an ignored real-PostgreSQL CRUD/RLS integration test. Authenticated HTTP routes and views are not included yet.".to_owned()
                    } else {
                        "Requires a database-enabled application.".to_owned()
                    },
                },
                GeneratorCapability {
                    command: "forge generate page".to_owned(),
                    status: GeneratorStatus::Available,
                    description: "Creates a server-rendered HTML view with escaped text values and automatically registers a GET route; no view service or framework-owned template dependency is required.".to_owned(),
                },
            ],
        }
    }

    pub(crate) fn render_agent_guide(&self) -> String {
        format!(
            "# Agent guide for `{}`\n\n\
             Start with `docs/for-ai.md` for the project map, verified commands, and\n\
             security constraints. Treat `.forge/capabilities.json` as the machine-\n\
             readable source of truth; do not infer a feature from a directory name.\n\n\
             ## Non-negotiable boundaries\n\n\
             - Keep domain and application code independent of HTTP, SQLx, Tokio, and\n\
               concrete infrastructure.\n\
             - Never weaken authentication, authorization, tenant context, or RLS.\n\
             - Treat model output, tool requests, generated code, and shell commands\n\
               as untrusted input. Never run destructive or production operations\n\
               without explicit human authorization.\n\
             - Read the relevant implementation and migration history before editing;\n\
               preserve unrelated user changes.\n\n\
             ## Machine-readable discovery\n\n\
             - `forge capabilities --format json` lists scaffolded capabilities.\n\
             - `forge doctor --format json` reports read-only local diagnostics.\n\
             - Add `--json-errors` for versioned command errors on stderr.\n",
            self.application
        )
    }

    /// Renders the detailed, feature-aware playbook for coding agents.
    pub(crate) fn render_ai_guide(&self) -> String {
        let database = self.features.database.as_deref().unwrap_or("disabled");
        let auth = if self.features.authentication {
            "security primitives are scaffolded, but universal HTTP login/logout routes are not generated"
        } else {
            "not scaffolded"
        };
        let tenancy = if self.features.tenancy {
            "included"
        } else {
            "not scaffolded"
        };
        let docker = if self.features.docker {
            "Dockerfile and Compose are generated; inspect them before changing runtime assumptions"
        } else {
            "Docker assets were not requested for this scaffold"
        };
        let database_checks = if self.features.database.is_some() {
            "- Database integration tests must use a disposable database. Never point a test command at a production or developer database.\n\
             - PostgreSQL migrations are versioned; inspect their up/down pair and migration runner before editing schema.\n"
        } else {
            "- This scaffold has no database adapter or migrations. Do not add persistence by placing SQL in the domain or application rings.\n"
        };
        let available_generators = self
            .generators
            .iter()
            .map(|generator| {
                format!(
                    "- `{}` — {}: {}\n",
                    generator.command,
                    generator.status.as_str(),
                    generator.description
                )
            })
            .collect::<String>();
        format!(
            "# AI authoring guide for `{}`\n\n\
             This guide is generated from the same capability manifest as this app.\n\
             If it disagrees with executable code, trust the code and report the drift.\n\n\
             ## Project map\n\n\
             - `src/domain`: business rules, owned types, and typed domain errors. No I/O.\n\
             - `src/application`: use cases and ports. Keep dependencies pointed inward.\n\
             - `src/adapters`: HTTP and view translation, plus protocol adapters.\n\
             - `src/infrastructure`: concrete database and runtime mechanisms.\n\
             - `src/bootstrap`: composition root and process lifecycle.\n\
             - `tests`: executable behavior and architecture constraints.\n\n\
             ## Capabilities in this scaffold\n\n\
             - Database adapter: `{database}`.\n\
             - Authentication scaffolding: {auth}.\n\
             - Tenant membership and RLS scaffolding: {tenancy}.\n\
             - Docker: {docker}.\n\
             - Jobs/events: {}.\n\
             - AI providers or tool execution: {}.\n\n\
             ### Generator maturity\n\n\
             {available_generators}\n\
             `available` means the documented artifacts are generated; `partial`\n\
             means inspect the description and fill the missing behavior explicitly.\n\n\
             ## Change workflow\n\n\
             1. Read the relevant code, tests, migrations, and ADRs before editing.\n\
             2. State the behavior change and affected rings; add the smallest complete\n\
                vertical slice instead of a placeholder abstraction.\n\
             3. Prefer generator previews before writing; migration paths contain a\n\
                provisional timestamp. Generators must refuse user-file collisions.\n\
             4. Add or update focused regression tests, then run the narrowest relevant\n\
                checks before the documented workspace gates.\n\
             5. Review the final diff and report executed checks separately from checks\n\
                that were skipped or blocked.\n\n\
             ## Security and authority\n\n\
             AI suggestions and tool arguments are untrusted data. A model response is\n\
             never authorization. Keep intent, policy, authorization, validation,\n\
             approval, execution, and audit separate. Do not expose secrets in prompts,\n\
             generated context, logs, command output, or tests. Never grant an agent\n\
             unrestricted filesystem, process, network, database, or production access.\n\n\
             ## Useful commands\n\n\
             - `forge capabilities --format json` — read the versioned capability contract.\n\
             - `forge doctor --format json` — inspect static project/environment findings.\n\
             - `forge doctor --probe-database` — optional bounded TCP reachability only;\n\
               it does not authenticate or query data.\n\
             - `forge generate page <Name> --dry-run` — preview page output without writes.\n\
             - `forge generate resource <Name> --fields <fields> --dry-run` — preview a\n\
               resource where the database capability is enabled.\n\
             - `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets\n\
               --all-features --locked -- -D warnings`, `cargo test --workspace\n\
               --all-targets --locked`, `cargo test --workspace --doc --locked`.\n\n\
             ## Environment notes\n\n\
             {database_checks}\n\
             {docker}\n",
            self.application,
            self.features.jobs_and_events,
            self.features.ai_providers_and_execution,
        )
    }
}
