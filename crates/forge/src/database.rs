//! Database wiring for the application lifecycle: the declared backend, the
//! pool handed to composition hooks, the readiness check and the `migrate` /
//! `rollback` commands.
//!
//! [`Declaration`] and [`DatabaseHandle`] have one variant per compiled-in
//! backend (and none without database features), so the rest of the facade
//! handles every backend without feature conditionals.

use std::{process::ExitCode, sync::Arc};

use forge_config::{DatabaseConfig, DatabaseScheme};
use forge_http::ReadinessCheck;

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
use std::time::Duration;

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
use forge_db::{Backend, Database, Migrator, sqlx};

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
use tracing::{error, info, warn};

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
use crate::app::ErrorChain;

/// Failure to create a pool; uninhabited without database features.
#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
pub(crate) type ConnectError = forge_db::DbError;
/// Failure to create a pool; uninhabited without database features.
#[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
pub(crate) type ConnectError = std::convert::Infallible;

/// Upper bound for closing the pool after the server has drained.
#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The database an application declared, with its embedded migrations.
pub(crate) enum Declaration {
    /// Declared with `App::postgres`.
    #[cfg(feature = "postgres")]
    Postgres(Migrator),
    /// Declared with `App::mysql`.
    #[cfg(feature = "mysql")]
    MySql(Migrator),
    /// Declared with `App::sqlite`.
    #[cfg(feature = "sqlite")]
    Sqlite(Migrator),
}

impl Declaration {
    /// The URL schemes `FORGE_DATABASE_URL` must use.
    pub(crate) fn scheme(&self) -> DatabaseScheme {
        match *self {
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => DatabaseScheme::Postgres,
            #[cfg(feature = "mysql")]
            Self::MySql(_) => DatabaseScheme::MySql,
            #[cfg(feature = "sqlite")]
            Self::Sqlite(_) => DatabaseScheme::Sqlite,
        }
    }

    /// The builder method that made this declaration, e.g. "App::postgres".
    #[allow(dead_code)]
    pub(crate) fn method(&self) -> &'static str {
        match *self {
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => "App::postgres",
            #[cfg(feature = "mysql")]
            Self::MySql(_) => "App::mysql",
            #[cfg(feature = "sqlite")]
            Self::Sqlite(_) => "App::sqlite",
        }
    }

    /// Creates the lazily connecting pool for `serve`.
    pub(crate) fn connect_lazy(
        &self,
        config: &DatabaseConfig,
    ) -> Result<DatabaseHandle, ConnectError> {
        let _ = config;
        match *self {
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => Database::connect_lazy(config)
                .map(|db| DatabaseHandle::Postgres(std::sync::Arc::new(db))),
            #[cfg(feature = "mysql")]
            Self::MySql(_) => Database::connect_lazy(config)
                .map(|db| DatabaseHandle::MySql(std::sync::Arc::new(db))),
            #[cfg(feature = "sqlite")]
            Self::Sqlite(_) => Database::connect_lazy(config)
                .map(|db| DatabaseHandle::Sqlite(std::sync::Arc::new(db))),
        }
    }

    /// Connects eagerly and runs `command`; returns the process exit code.
    pub(crate) async fn run_migration_command(
        &self,
        app: &'static str,
        config: &DatabaseConfig,
        command: MigrationCommand,
    ) -> ExitCode {
        let _ = (app, config, command);
        match *self {
            #[cfg(feature = "postgres")]
            Self::Postgres(ref migrator) => {
                run_migration_command::<forge_db::sqlx::Postgres>(app, config, migrator, command)
                    .await
            }
            #[cfg(feature = "mysql")]
            Self::MySql(ref migrator) => {
                run_migration_command::<forge_db::sqlx::MySql>(app, config, migrator, command).await
            }
            #[cfg(feature = "sqlite")]
            Self::Sqlite(ref migrator) => {
                run_migration_command::<forge_db::sqlx::Sqlite>(app, config, migrator, command)
                    .await
            }
        }
    }
}

/// The pool of the declared database, as handed to composition hooks.
#[derive(Debug, Clone)]
pub enum DatabaseHandle {
    /// A PostgreSQL pool.
    #[cfg(feature = "postgres")]
    Postgres(std::sync::Arc<forge_db::PgDatabase>),
    /// A MySQL/MariaDB pool.
    #[cfg(feature = "mysql")]
    MySql(std::sync::Arc<forge_db::MySqlDatabase>),
    /// A SQLite pool.
    #[cfg(feature = "sqlite")]
    Sqlite(std::sync::Arc<forge_db::SqliteDatabase>),
    /// Placeholder when no database features are enabled.
    #[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
    None,
}

impl DatabaseHandle {
    /// The `/health/ready` check named `database` for this pool.
    #[allow(dead_code)]
    pub(crate) fn readiness(&self) -> Arc<dyn ReadinessCheck> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Postgres(database) => Arc::new(DatabaseReadiness::new(database.clone())),
            #[cfg(feature = "mysql")]
            Self::MySql(database) => Arc::new(DatabaseReadiness::new(database.clone())),
            #[cfg(feature = "sqlite")]
            Self::Sqlite(database) => Arc::new(DatabaseReadiness::new(database.clone())),
            #[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
            Self::None => {
                // No database backend enabled, return a dummy readiness check that always fails
                Arc::new(NoDatabaseReadiness)
            }
        }
    }

    /// Closes the pool, giving checked-out connections a bounded time to
    /// return.
    #[allow(dead_code)]
    pub(crate) async fn close(&self) {
        match self {
            #[cfg(feature = "postgres")]
            Self::Postgres(database) => close(database.clone()).await,
            #[cfg(feature = "mysql")]
            Self::MySql(database) => close(database.clone()).await,
            #[cfg(feature = "sqlite")]
            Self::Sqlite(database) => close(database.clone()).await,
            #[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
            Self::None => {}
        }
    }
}

/// Schema command selected on the command line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationCommand {
    /// Apply every pending migration.
    Migrate,
    /// Revert the most recently applied migration.
    Rollback,
}

/// `/health/ready` check that pings the database.
#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
#[allow(dead_code)]
pub(crate) struct DatabaseReadiness<B: Backend> {
    database: std::sync::Arc<Database<B>>,
}

/// Dummy readiness check when no database backend is enabled.
#[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
#[allow(dead_code)]
struct NoDatabaseReadiness;

#[cfg(not(any(feature = "postgres", feature = "mysql", feature = "sqlite")))]
#[async_trait::async_trait]
impl ReadinessCheck for NoDatabaseReadiness {
    fn name(&self) -> &'static str {
        "database"
    }

    async fn check(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Err("no database backend configured".into())
    }
}

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
impl<B: Backend> DatabaseReadiness<B> {
    #[allow(dead_code)]
    pub(crate) fn new(database: std::sync::Arc<Database<B>>) -> Self {
        Self { database }
    }
}

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
#[async_trait::async_trait]
impl<B: Backend> ReadinessCheck for DatabaseReadiness<B> {
    fn name(&self) -> &'static str {
        "database"
    }

    #[allow(dead_code)]
    async fn check(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let database = self.database.clone();
        database.ping().await.map_err(|error| {
            warn!(error = %crate::app::ErrorChain(&error), "database readiness check failed");
            Box::new(error) as Box<dyn std::error::Error + Send + Sync>
        })
    }
}

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
async fn close<B: Backend>(database: std::sync::Arc<Database<B>>) {
    if tokio::time::timeout(CLOSE_TIMEOUT, database.close())
        .await
        .is_err()
    {
        warn!(
            timeout = ?CLOSE_TIMEOUT,
            "timed out closing the database pool; abandoning remaining connections"
        );
    }
}

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
async fn run_migration_command<B: Backend>(
    app: &'static str,
    config: &DatabaseConfig,
    migrator: &Migrator,
    command: MigrationCommand,
) -> ExitCode
where
    <B as sqlx::Database>::Connection: sqlx::migrate::Migrate,
{
    let database = match Database::<B>::connect(config).await {
        Ok(database) => std::sync::Arc::new(database),
        Err(error) => {
            error!(app, error = %ErrorChain(&error), "failed to connect to the database");
            return ExitCode::FAILURE;
        }
    };
    let code = match command {
        MigrationCommand::Migrate => migrate(app, &database, migrator).await,
        MigrationCommand::Rollback => rollback(app, &database, migrator).await,
    };
    close(std::sync::Arc::clone(&database)).await;
    code
}

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
async fn migrate<B: Backend>(
    app: &'static str,
    database: &Database<B>,
    migrator: &Migrator,
) -> ExitCode
where
    <B as sqlx::Database>::Connection: sqlx::migrate::Migrate,
{
    match database.migrate(migrator).await {
        Ok(applied) if applied.is_empty() => {
            info!(app, "no pending migrations");
            ExitCode::SUCCESS
        }
        Ok(applied) => {
            for version in &applied {
                info!(
                    app,
                    version,
                    description = description(migrator, *version),
                    "applied migration"
                );
            }
            info!(app, count = applied.len(), "migrations applied");
            ExitCode::SUCCESS
        }
        Err(error) => {
            error!(app, error = %ErrorChain(&error), "migration failed");
            ExitCode::FAILURE
        }
    }
}

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
async fn rollback<B: Backend>(
    app: &'static str,
    database: &Database<B>,
    migrator: &Migrator,
) -> ExitCode
where
    <B as sqlx::Database>::Connection: sqlx::migrate::Migrate,
{
    match database.rollback(migrator).await {
        Ok(Some(version)) => {
            info!(
                app,
                version,
                description = description(migrator, version),
                "reverted migration"
            );
            ExitCode::SUCCESS
        }
        Ok(None) => {
            info!(app, "no applied migrations to revert");
            ExitCode::SUCCESS
        }
        Err(error) => {
            error!(app, error = %ErrorChain(&error), "rollback failed");
            ExitCode::FAILURE
        }
    }
}

#[cfg(any(feature = "postgres", feature = "mysql", feature = "sqlite"))]
fn description(migrator: &Migrator, version: i64) -> &str {
    migrator
        .iter()
        .find(|migration| {
            migration.version == version && migration.migration_type.is_up_migration()
        })
        .map_or("", |migration| migration.description.as_ref())
}
