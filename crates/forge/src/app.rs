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

use forge_config::{AppConfig, Environment, SecretString};
use forge_http::{RouteError, Router, Server, ServerError};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::{
    cli::{self, Command},
    logging, probe, signal,
};

/// `sysexits.h` `EX_USAGE`-style code for an invalid command line.
const EXIT_USAGE: u8 = 2;
/// `sysexits.h` `EX_CONFIG`: the configuration is invalid.
const EXIT_CONFIG: u8 = 78;
/// Upper bound for runtime teardown after the server has already drained.
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const UNKNOWN_VERSION: &str = "unknown";
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

type RouteRegistration = Box<dyn FnOnce(&mut Router) -> Result<(), RouteError> + Send + 'static>;
type SetupFuture = Pin<
    Box<
        dyn Future<Output = Result<RouteRegistration, Box<dyn Error + Send + Sync>>>
            + Send
            + 'static,
    >,
>;
type SetupHandler = Box<dyn FnOnce(AppConfig) -> SetupFuture + Send + 'static>;
type DatabaseCommandFuture =
    Pin<Box<dyn Future<Output = Result<(), Box<dyn Error + Send + Sync>>> + Send + 'static>>;
type DatabaseCommandHandler =
    Box<dyn Fn(SecretString) -> DatabaseCommandFuture + Send + Sync + 'static>;

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
    setups: Vec<SetupHandler>,
    startup_timeout: Duration,
    migrate: Option<DatabaseCommandHandler>,
    rollback: Option<DatabaseCommandHandler>,
}

impl fmt::Debug for App {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("App")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("route_registrations", &self.registrations.len())
            .field("async_setups", &self.setups.len())
            .field("startup_timeout", &self.startup_timeout)
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
            setups: Vec::new(),
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            migrate: None,
            rollback: None,
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

    /// Initializes application resources inside the owned runtime, then registers
    /// routes using those resources. Runs only for `serve`, after synchronous
    /// registrations and before binding the listener. Failure aborts startup.
    ///
    /// Setup handlers run sequentially under one deadline and are cancelled on
    /// shutdown. Resources should be captured by the returned registration so
    /// their lifetime follows the router and its handlers.
    #[must_use]
    pub fn setup<F, Fut, R, E>(mut self, initialize: F) -> Self
    where
        F: FnOnce(AppConfig) -> Fut + Send + 'static,
        Fut: Future<Output = Result<R, E>> + Send + 'static,
        R: FnOnce(&mut Router) -> Result<(), RouteError> + Send + 'static,
        E: Error + Send + Sync + 'static,
    {
        self.setups.push(Box::new(move |config| {
            Box::pin(async move {
                initialize(config)
                    .await
                    .map(|register| Box::new(register) as RouteRegistration)
                    .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
            })
        }));
        self
    }

    /// Sets the total asynchronous setup deadline (default: 30 seconds).
    #[must_use]
    pub fn startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
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
            setups,
            startup_timeout,
            migrate: _,
            rollback: _,
        } = self;

        let config = match AppConfig::from_env() {
            Ok(config) => config,
            Err(error) => return config_error(&error),
        };
        if let Err(error) = logging::init(&config.log) {
            return config_error(&error);
        }

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

        let result = runtime.block_on(serve_until_signal(
            name,
            version,
            config,
            registrations,
            setups,
            startup_timeout,
        ));
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
    #[error("failed to register routes")]
    Routes(#[from] RouteError),
    #[error("application setup failed")]
    Setup(#[source] Box<dyn Error + Send + Sync>),
    #[error("application setup exceeded its deadline")]
    SetupTimeout,
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

async fn initialize_router(
    config: AppConfig,
    registrations: Vec<RouteRegistration>,
    setups: Vec<SetupHandler>,
    timeout: Duration,
) -> Result<Router, ServeError> {
    tokio::time::timeout(timeout, async move {
        let mut router = build_router(registrations)?;
        for initialize in setups {
            let register = initialize(config.clone())
                .await
                .map_err(ServeError::Setup)?;
            register(&mut router)?;
        }
        Ok(router)
    })
    .await
    .map_err(|_| ServeError::SetupTimeout)?
}

async fn serve_until_signal(
    name: &'static str,
    version: &'static str,
    config: AppConfig,
    registrations: Vec<RouteRegistration>,
    setups: Vec<SetupHandler>,
    startup_timeout: Duration,
) -> Result<(), ServeError> {
    let mut signal = Box::pin(signal::shutdown_signal().map_err(ServeError::Signal)?);
    let Some(router) = initialize_until_shutdown(
        initialize_router(config.clone(), registrations, setups, startup_timeout),
        signal.as_mut(),
    )
    .await?
    else {
        info!(app = name, "shutdown during application setup");
        return Ok(());
    };
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

async fn initialize_until_shutdown(
    initialization: impl Future<Output = Result<Router, ServeError>>,
    shutdown: impl Future<Output = &'static str>,
) -> Result<Option<Router>, ServeError> {
    tokio::select! {
        biased;
        _ = shutdown => Ok(None),
        result = initialization => result.map(Some),
    }
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
struct ErrorChain<'a>(&'a (dyn Error + 'static));

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

    #[tokio::test]
    async fn setup_runs_in_runtime_and_registers_routes_after_initialization() {
        let app = App::new("demo").setup(|config| async move {
            assert_eq!(config.environment, Environment::Development);
            tokio::task::yield_now().await;
            Ok::<_, io::Error>(register_index)
        });
        let router = initialize_router(
            AppConfig::default(),
            app.registrations,
            app.setups,
            app.startup_timeout,
        )
        .await
        .expect("setup should succeed");
        let routes = format!("{router:?}");
        assert!(routes.contains("GET /health/live"));
        assert!(routes.contains("\"GET /\""));
    }

    #[tokio::test]
    async fn setup_errors_and_duplicate_routes_abort_initialization() {
        let app = App::new("demo").setup(|_| async {
            Err::<fn(&mut Router) -> Result<(), RouteError>, _>(io::Error::other("setup failed"))
        });
        assert!(matches!(
            initialize_router(
                AppConfig::default(),
                app.registrations,
                app.setups,
                app.startup_timeout
            )
            .await,
            Err(ServeError::Setup(_))
        ));
        let app = App::new("demo")
            .routes(register_index)
            .setup(|_| async { Ok::<_, io::Error>(register_index) });
        assert!(matches!(
            initialize_router(
                AppConfig::default(),
                app.registrations,
                app.setups,
                app.startup_timeout
            )
            .await,
            Err(ServeError::Routes(RouteError::Duplicate { .. }))
        ));
    }

    #[tokio::test]
    async fn setup_deadline_drops_owned_resources() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct OwnedResource(Arc<AtomicBool>);
        impl Drop for OwnedResource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let resource = OwnedResource(Arc::clone(&dropped));
        let app = App::new("demo")
            .startup_timeout(Duration::from_millis(1))
            .setup(|_| async move {
                let _resource = resource;
                std::future::pending::<
                        Result<fn(&mut Router) -> Result<(), RouteError>, io::Error>,
                    >()
                    .await
            });
        assert!(matches!(
            initialize_router(
                AppConfig::default(),
                app.registrations,
                app.setups,
                app.startup_timeout
            )
            .await,
            Err(ServeError::SetupTimeout)
        ));
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn shutdown_cancels_setup_before_a_router_can_be_served() {
        let result = initialize_until_shutdown(
            std::future::pending::<Result<Router, ServeError>>(),
            async { "shutdown" },
        )
        .await
        .expect("shutdown should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn informational_commands_do_not_run_setup() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let called = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&called);
        let code = App::new("demo")
            .setup(move |_| async move {
                observed.store(true, Ordering::SeqCst);
                Ok::<_, io::Error>(register_index)
            })
            .run_with_args(args(&["version"]));
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(!called.load(Ordering::SeqCst));
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
