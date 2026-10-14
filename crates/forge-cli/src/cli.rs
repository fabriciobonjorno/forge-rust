use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "forge",
    version,
    about = "Build and maintain Forge applications",
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    /// Render Forge execution errors as JSON on stderr for automation.
    #[arg(long, global = true)]
    pub(crate) json_errors: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}

impl Cli {
    pub(crate) fn parse_process() -> Result<Self, clap::Error> {
        Self::try_parse()
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Generate a sortable UUIDv7 identifier.
    Id,
    /// Create a new application with hexagonal boundaries, Docker and CI.
    New {
        /// Cargo package name and destination directory.
        name: String,
        /// Do not generate Dockerfile, .dockerignore, compose.yaml or the image CI job.
        #[arg(long)]
        skip_docker: bool,
        /// Do not generate the GitHub Actions workflow and Dependabot configuration.
        #[arg(long)]
        skip_ci: bool,
        /// Generate an application without PostgreSQL, migrations or database Compose services.
        #[arg(long)]
        skip_database: bool,
        /// Do not run `cargo generate-lockfile` after generation.
        #[arg(long)]
        skip_lockfile: bool,
        /// Depend on a local Forge checkout instead of the release tag. Relative
        /// paths are resolved by Cargo from the new application's directory.
        #[arg(long, value_name = "PATH")]
        forge_path: Option<String>,
    },
    /// Generate application artifacts.
    Generate {
        #[command(subcommand)]
        generator: GenerateCommand,
    },
    /// Type-check the current application.
    Check,
    /// Run the current application's tests.
    Test,
    /// Format the current application's Rust code.
    Format,
    /// Run Clippy with warnings treated as errors.
    Lint,
    /// Build the optimized release binary from the locked dependency graph.
    Build,
    /// Print the generated application's versioned capability manifest.
    Capabilities {
        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },
    /// Diagnose the current Forge project using read-only local checks.
    Doctor {
        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
        /// Attempt bounded TCP connection checks for configured PostgreSQL URLs.
        #[arg(long)]
        probe_database: bool,
        /// Per-host connection timeout in milliseconds (100..=10000).
        #[arg(long, default_value_t = 1500, value_parser = clap::value_parser!(u64).range(100..=10000))]
        timeout_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Versioned JSON for tools and coding agents.
    #[default]
    Json,
}

#[derive(Debug, Subcommand)]
pub(crate) enum GenerateCommand {
    /// Create a reversible SQL migration in migrations/.
    Migration {
        /// Human-readable migration name.
        name: String,
        /// Stable SQLx migration version. Supply this for reproducible previews and output.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        version: Option<u64>,
        /// Print a versioned JSON preview without writing files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate a typed resource, application port/use cases and PostgreSQL migration.
    Resource {
        /// Resource name in PascalCase (e.g., Project, UserAccount).
        name: String,
        /// Comma-separated field definitions: name:type (string, text, uuid, bool, enum).
        #[arg(long)]
        fields: String,
        /// Generate a global resource without tenant isolation.
        #[arg(long, conflicts_with = "tenant_scoped")]
        global: bool,
        /// Explicitly generate tenant-scoped storage (the default).
        #[arg(long)]
        tenant_scoped: bool,
        /// Stable SQLx migration version. Supply this for reproducible previews and output.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        version: Option<u64>,
        /// Print a versioned JSON preview without writing files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Generate an HTML page, escaped renderer and GET route registration.
    Page {
        /// PascalCase page name (e.g., Home, UserProfile).
        name: String,
        /// Print a versioned JSON preview without writing files.
        #[arg(long)]
        dry_run: bool,
    },
}
