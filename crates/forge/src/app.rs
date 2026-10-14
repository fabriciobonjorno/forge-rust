//! Application builder and process lifecycle.

use std::{
    error::Error,
    ffi::OsString,
    fmt,
    future::Future,
    io::{self, Write},
    net::SocketAddr,
    pin::Pin,
    process::ExitCode,
    time::Duration,
};

use forge_config::{AppConfig, DatabaseConfig, Environment, SecretString};
use forge_http::{RouteError, Router, Server, ServerError};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::{
    cli::{self, Command},
    compose::{ComposeError, Composition},
    database::{Declaration, MigrationCommand},
    logging, probe, signal,
};

/// `sysexits.h` `EX_USAGE`-style code for an invalid command line.
const EXIT_USAGE: u8 = 2;
/// `sysexits.h` `EX_CONFIG`: the configuration is invalid.
const EXIT_CONFIG: u8 = 78;
/// Upper bound for runtime teardown after the server has already drained.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const UNKNOWN_VERSION: &str = "unknown";

type RouteRegistration = Box<dyn FnOnce(&mut Router) -> Result<(), RouteError> + Send + 'static>;
type DatabaseCommandFuture =
    Pin<Box<dyn Future<Output = Result<(), Box<dyn Error + Send + Sync>>> + Send + 'static>>;
type DatabaseCommandHandler =
    Box<dyn Fn(SecretString) -> DatabaseCommandFuture + Send + Sync + 'static>;
type ComposeHook =
    Box<dyn FnOnce(&mut Composition<'_>) -> Result<(), ComposeError> + Send + 'static>;

/// A Forge application process.
///
/// `App` is the entry point of generated applications. It parses the command
/// line, loads [`AppConfig`] from the environment, installs logging, builds
/// the Tokio runtime and serves HTTP until `SIGTERM`/`SIGINT`. Applications do
/// not use `#[tokio::main]`; the runtime is a framework detail.
pub struct App {
    name: &'static str,
    version: &'static str,
    registrations: Vec<RouteRegistration>,
    migrate: Option<DatabaseCommandHandler>,
    rollback: Option<DatabaseCommandHandler>,
    declaration: Option<Declaration>,
    compose_hooks: Vec<ComposeHook>,
}

impl fmt::Debug for App {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("App")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("route_registrations", &self.registrations.len())
            .field("database_migrations", &self.migrate.is_some())
            .finish()
    }
}

impl App {
    /// Creates an application.
    ///
    /// `name` is used in logs and `--version` output.
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            version: UNKNOWN_VERSION,
            registrations: Vec::new(),
            migrate: None,
            rollback: None,
            declaration: None,
            compose_hooks: Vec::new(),
        }
    }

    /// Sets the version reported in logs and `--version` output, typically
    /// `env!("CARGO_PKG_VERSION")`. Defaults to `"unknown"`.
    #[must_use]
    pub fn version(mut self, version: &'static str) -> Self {
        self.version = version;
        self
    }

    /// Registers application routes.
    ///
    /// Standard health routes (`/health`, `/health/live`, `/health/ready`) are
    /// always added by the framework before `register` runs. May be called
    /// several times; registrations run in call order. A [`RouteError`] (for
    /// example a duplicate route) aborts startup with exit code 1.
    #[must_use]
    pub fn routes(
        mut self,
        register: impl FnOnce(&mut Router) -> Result<(), RouteError> + Send + 'static,
    ) -> Self {
        self.registrations.push(Box::new(register));
        self
    }

    /// Registers database migration handlers.
    ///
    /// Generated PostgreSQL applications use this hook to keep SQLx confined to
    /// their infrastructure/bootstrap boundary while the Forge process owns
    /// command dispatch and runtime lifecycle.
    #[must_use]
    pub fn migrations<M, MFut, ME, R, RFut, RE>(mut self, migrate: M, rollback: R) -> Self
    where
        M: Fn(SecretString) -> MFut + Send + Sync + 'static,
        MFut: Future<Output = Result<(), ME>> + Send + 'static,
        ME: Error + Send + Sync + 'static,
        R: Fn(SecretString) -> RFut + Send + Sync + 'static,
        RFut: Future<Output = Result<(), RE>> + Send + 'static,
        RE: Error + Send + Sync + 'static,
    {
        self.migrate = Some(Box::new(move |url| {
            let future = migrate(url);
            Box::pin(async move {
                future
                    .await
                    .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
            })
        }));
        self.rollback = Some(Box::new(move |url| {
            let future = rollback(url);
            Box::pin(async move {
                future
                    .await
                    .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
            })
        }));
        self
    }

    /// Declares PostgreSQL as the database backend with embedded migrations.
    ///
    /// This makes `FORGE_DATABASE_URL` mandatory for `serve` and adds the
    /// `migrate` and `rollback` subcommands. The `migrator` is created by
    /// `sqlx::migrate!()` and embedded into the binary.
    ///
    /// Requires the `postgres` feature on `forge-db`.
    #[cfg(feature = "postgres")]
    #[must_use]
    pub fn postgres(mut self, migrator: forge_db::sqlx::migrate::Migrator) -> Self {
        self.declaration = Some(Declaration::Postgres(migrator));
        self
    }

    /// Declares MySQL/MariaDB as the database backend with embedded migrations.
    ///
    /// This makes `FORGE_DATABASE_URL` mandatory for `serve` and adds the
    /// `migrate` and `rollback` subcommands. The `migrator` is created by
    /// `sqlx::migrate!()` and embedded into the binary.
    ///
    /// Requires the `mysql` feature on `forge-db`.
    #[cfg(feature = "mysql")]
    #[must_use]
    pub fn mysql(mut self, migrator: forge_db::sqlx::migrate::Migrator) -> Self {
        self.declaration = Some(Declaration::MySql(migrator));
        self
    }

    /// Declares SQLite as the database backend with embedded migrations.
    ///
    /// This makes `FORGE_DATABASE_URL` mandatory for `serve` and adds the
    /// `migrate` and `rollback` subcommands. The `migrator` is created by
    /// `sqlx::migrate!()` and embedded into the binary.
    ///
    /// Requires the `sqlite` feature on `forge-db`.
    #[cfg(feature = "sqlite")]
    #[must_use]
    pub fn sqlite(mut self, migrator: forge_db::sqlx::migrate::Migrator) -> Self {
        self.declaration = Some(Declaration::Sqlite(migrator));
        self
    }

    /// Registers a composition hook that runs at startup to wire adapters and routes.
    ///
    /// Composition hooks run inside the Tokio runtime, after the standard health
    /// routes are registered and before the server accepts connections. They receive
    /// a `Composition` context with access to the router, validated configuration,
    /// and the database pool (if a backend was declared with `postgres`, `mysql`,
    /// or `sqlite`).
    ///
    /// Multiple hooks can be registered; they run in registration order.
    #[must_use]
    pub fn compose(
        mut self,
        hook: impl FnOnce(&mut Composition<'_>) -> Result<(), ComposeError> + Send + 'static,
    ) -> Self {
        self.compose_hooks.push(Box::new(hook));
        self
    }

    /// Parses process arguments, runs the selected command and returns the
    /// process exit code.
    ///
    /// Commands: serve (default), healthcheck, migrate, rollback, version
    /// (--version, -V) and help (--help, -h). Exit codes: 0 success, 1 runtime
    /// failure or unhealthy probe, `2` invalid command line, `78` invalid
    /// configuration.
    #[must_use]
    pub fn run(self) -> ExitCode {
        self.run_with_args(std::env::args_os().skip(1))
    }

    fn run_with_args(self, args: impl IntoIterator<Item = OsString>) -> ExitCode {
        match cli::parse(args) {
            Ok(Command::Serve) => self.serve(),
            Ok(Command::Healthcheck) => healthcheck(),
            Ok(Command::Migrate) => self.run_database_command(DatabaseCommand::Migrate),
            Ok(Command::Rollback) => self.run_database_command(DatabaseCommand::Rollback),
            Ok(Command::Version) => write_stdout(&format!("{} {}\n", self.name, self.version)),
            Ok(Command::Help) => write_stdout(&cli::usage(self.name)),
            Err(error) => {
                write_stderr(&format!("error: {error}\n\n{}", cli::usage(self.name)));
                ExitCode::from(EXIT_USAGE)
            }
        }
    }

    fn serve(self) -> ExitCode {
        let Self {
            name,
            version,
            registrations,
            migrate: _,
            rollback: _,
            declaration,
            compose_hooks,
        } = self;

        let config = match AppConfig::from_env() {
            Ok(config) => config,
            Err(error) => return config_error(&error),
        };
        if let Err(error) = logging::init(&config.log) {
            return config_error(&error);
        }

        // Validate database scheme matches declaration
        if let Some(ref decl) = declaration {
            let expected_scheme = decl.scheme();
            let actual_scheme = config.database.scheme();
            if actual_scheme != Some(expected_scheme) {
                let actual = actual_scheme.map_or("none".to_string(), |s| s.to_string());
                write_stderr(&format!(
                    "error: FORGE_DATABASE_URL scheme '{}' does not match declared backend '{}'\n",
                    actual, expected_scheme
                ));
                return ExitCode::from(EXIT_CONFIG);
            }
        } else if config.database.url.is_some() {
            write_stderr(
                "error: FORGE_DATABASE_URL provided but no database backend declared (use App::postgres, App::mysql, or App::sqlite)\n",
            );
            return ExitCode::from(EXIT_CONFIG);
        }

        let mut router = match build_router(registrations) {
            Ok(router) => router,
            Err(error) => {
                error!(app = name, error = %ErrorChain(&error), "failed to register routes");
                return ExitCode::FAILURE;
            }
        };

        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                error!(app = name, error = %ErrorChain(&error), "failed to start async runtime");
                return ExitCode::FAILURE;
            }
        };

        // SQLx lazy pools and adapters composed around them require a Tokio
        // context even before the first request starts.
        let _database = {
            let _runtime_guard = runtime.enter();

            let database = if let Some(ref decl) = declaration {
                Some(match decl.connect_lazy(&config.database) {
                    Ok(db) => db,
                    Err(error) => {
                        error!(app = name, error = %ErrorChain(&error), "failed to create database pool");
                        return ExitCode::FAILURE;
                    }
                })
            } else {
                None
            };

            if !compose_hooks.is_empty() {
                let mut composition = Composition::new(&mut router, &config, database.clone());
                for hook in compose_hooks {
                    if let Err(error) = hook(&mut composition) {
                        error!(app = name, error = %ErrorChain(&error), "composition hook failed");
                        return ExitCode::FAILURE;
                    }
                }
            }

            database
        };

        let result = runtime.block_on(serve_until_signal(name, version, config, router));
        runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);

        match result {
            Ok(()) => {
                info!(app = name, "shutdown complete");
                ExitCode::SUCCESS
            }
            Err(error) => {
                error!(app = name, error = %ErrorChain(&error), "server failed");
                ExitCode::FAILURE
            }
        }
    }
    fn run_database_command(self, command: DatabaseCommand) -> ExitCode {
        let config = match AppConfig::from_env() {
            Ok(config) => config,
            Err(error) => return config_error(&error),
        };
        if let Err(error) = logging::init(&config.log) {
            return config_error(&error);
        }

        let Some(url) = config.database.migration_url else {
            write_stderr("error: FORGE_MIGRATION_DATABASE_URL is required for database commands\n");
            return ExitCode::from(EXIT_CONFIG);
        };

        // Use the declaration to run migrations if available
        if let Some(decl) = self.declaration {
            let database_config = DatabaseConfig::new(url);
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    error!(app = self.name, error = %ErrorChain(&error), "failed to start async runtime");
                    return ExitCode::FAILURE;
                }
            };

            let migration_command = match command {
                DatabaseCommand::Migrate => MigrationCommand::Migrate,
                DatabaseCommand::Rollback => MigrationCommand::Rollback,
            };

            let result = runtime.block_on(decl.run_migration_command(
                self.name,
                &database_config,
                migration_command,
            ));
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
            return result;
        }

        // Fall back to the legacy migrations() handlers
        let handler = match command {
            DatabaseCommand::Migrate => self.migrate,
            DatabaseCommand::Rollback => self.rollback,
        };
        let Some(handler) = handler else {
            write_stderr("error: database migrations are not configured for this application\n");
            return ExitCode::FAILURE;
        };

        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                error!(app = self.name, error = %ErrorChain(&error), "failed to start async runtime");
                return ExitCode::FAILURE;
            }
        };

        let result = runtime.block_on(handler(url));
        runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                error!(
                    app = self.name,
                    command = command.as_str(),
                    error = %ErrorChain(error.as_ref()),
                    "database command failed"
                );
                ExitCode::FAILURE
            }
        }
    }
}

#[derive(Clone, Copy)]
enum DatabaseCommand {
    Migrate,
    Rollback,
}

impl DatabaseCommand {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Migrate => "migrate",
            Self::Rollback => "rollback",
        }
    }
}

/// Builds the route table: framework health routes first, then the
/// application's registrations in order.
fn build_router(registrations: Vec<RouteRegistration>) -> Result<Router, RouteError> {
    let mut router = Router::new();
    router.standard_health_routes()?;
    for register in registrations {
        register(&mut router)?;
    }
    Ok(router)
}

/// Failure while running the `serve` command.
#[derive(Debug, Error)]
enum ServeError {
    #[error("failed to install shutdown signal handlers")]
    Signal(#[source] io::Error),
    #[error("failed to bind HTTP listener on {address}")]
    Bind {
        address: SocketAddr,
        #[source]
        source: io::Error,
    },
    #[error("HTTP server failed")]
    Server(#[from] ServerError),
}

async fn serve_until_signal(
    name: &'static str,
    version: &'static str,
    config: AppConfig,
    router: Router,
) -> Result<(), ServeError> {
    let signal = signal::shutdown_signal().map_err(ServeError::Signal)?;
    let bind = config.server.bind;
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|source| ServeError::Bind {
            address: bind,
            source,
        })?;
    let address = listener.local_addr().unwrap_or(bind);
    info!(
        app = name,
        version,
        environment = environment_name(config.environment),
        %address,
        "listening"
    );

    let shutdown = async move {
        let signal = signal.await;
        info!(app = name, signal, "shutdown started; draining connections");
    };
    Server::new(config.server, router)
        .serve(listener, shutdown)
        .await?;
    Ok(())
}

fn environment_name(environment: Environment) -> &'static str {
    match environment {
        Environment::Development => "development",
        Environment::Test => "test",
        Environment::Production => "production",
    }
}

/// Container health probe: exit 0 only when `GET /health/live` returns 200.
fn healthcheck() -> ExitCode {
    let config = match AppConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            write_stderr(&format!("healthcheck: {error}\n"));
            return ExitCode::FAILURE;
        }
    };
    let target = probe::probe_target(config.server.bind);
    match probe::probe(target, probe::PROBE_TIMEOUT) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            write_stderr(&format!("healthcheck: {error}\n"));
            ExitCode::FAILURE
        }
    }
}

fn config_error(error: &(dyn Error + 'static)) -> ExitCode {
    write_stderr(&format!("error: {}\n", ErrorChain(error)));
    ExitCode::from(EXIT_CONFIG)
}

/// Writes to stdout; a closed pipe yields a failure code instead of a panic.
fn write_stdout(text: &str) -> ExitCode {
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Best-effort stderr write; there is nowhere left to report a failure.
fn write_stderr(text: &str) {
    let mut stderr = io::stderr().lock();
    if stderr.write_all(text.as_bytes()).is_err() {
        return;
    }
    // Ignoring is deliberate: stderr is the last-resort channel.
    drop(stderr.flush());
}

/// Displays an error followed by each of its sources, separated by `: `.
pub struct ErrorChain<'a>(pub &'a (dyn Error + 'static));

impl fmt::Display for ErrorChain<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(error) = source {
            write!(formatter, ": {error}")?;
            source = error.source();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use forge_http::{Method, Response, StatusCode};

    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn version_defaults_to_unknown() {
        let app = App::new("demo");

        assert_eq!(app.version, "unknown");
        assert_eq!(app.version("1.2.3").version, "1.2.3");
    }

    #[test]
    fn informational_commands_succeed() {
        for command in ["--version", "-V", "version", "--help", "-h", "help"] {
            let code = App::new("demo").run_with_args(args(&[command]));
            assert_eq!(code, ExitCode::SUCCESS, "{command}");
        }
    }

    #[test]
    fn migration_handlers_can_be_registered() {
        let app = App::new("demo").migrations(
            |_url| async { Ok::<(), std::io::Error>(()) },
            |_url| async { Ok::<(), std::io::Error>(()) },
        );

        assert!(app.migrate.is_some());
        assert!(app.rollback.is_some());
    }

    #[test]
    fn invalid_command_line_exits_with_usage_code() {
        let code = App::new("demo").run_with_args(args(&["frobnicate"]));

        assert_eq!(code, ExitCode::from(EXIT_USAGE));
    }

    #[test]
    fn router_contains_health_and_application_routes() {
        let app = App::new("demo")
            .routes(|router| {
                router.route(Method::GET, "/a", |_| async {
                    Response::empty(StatusCode::OK)
                })?;
                Ok(())
            })
            .routes(|router| {
                router.route(Method::GET, "/b", |_| async {
                    Response::empty(StatusCode::OK)
                })?;
                Ok(())
            });

        let router = build_router(app.registrations).expect("routes must register");
        let debug = format!("{router:?}");

        for route in [
            "GET /health",
            "GET /health/live",
            "GET /health/ready",
            "GET /a",
            "GET /b",
        ] {
            assert!(debug.contains(route), "{route} missing from {debug}");
        }
    }

    fn register_index(router: &mut Router) -> Result<(), RouteError> {
        router.route(Method::GET, "/", |_| async {
            Response::empty(StatusCode::OK)
        })?;
        Ok(())
    }

    #[test]
    fn function_items_can_register_routes() {
        let app = App::new("demo").routes(register_index);

        let router = build_router(app.registrations).expect("routes must register");

        assert!(format!("{router:?}").contains("\"GET /\""));
    }

    #[test]
    fn application_cannot_replace_health_routes() {
        let app = App::new("demo").routes(|router| {
            router.route(Method::GET, "/health/live", |_| async {
                Response::empty(StatusCode::OK)
            })?;
            Ok(())
        });

        let error = build_router(app.registrations).expect_err("duplicate must be rejected");

        assert!(matches!(error, RouteError::Duplicate { .. }));
    }

    #[test]
    fn error_chain_includes_sources() {
        let error = ServeError::Bind {
            address: "127.0.0.1:1".parse().expect("valid address"),
            source: io::Error::new(io::ErrorKind::AddrInUse, "address in use"),
        };

        assert_eq!(
            ErrorChain(&error).to_string(),
            "failed to bind HTTP listener on 127.0.0.1:1: address in use"
        );
    }
}
