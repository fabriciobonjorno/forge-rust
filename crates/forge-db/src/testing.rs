//! Isolated databases for integration tests (feature `testing`).
//!
//! [`TestDatabase::create`] provisions an empty database named
//! `forge_test_<uuidv7>`, connects to it and applies the migrations. The
//! database is removed when the value is dropped, **including when the test
//! body panics**: the cleanup guard is an ordinary `Drop` that runs during
//! unwinding and performs the `DROP DATABASE` on a short-lived helper thread
//! with its own runtime (a `Drop` cannot await, and the test's runtime may be
//! shutting down). Call [`TestDatabase::cleanup`] to do the same
//! asynchronously and observe failures.
//!
//! | Backend | Administrative URL | Isolation |
//! | --- | --- | --- |
//! | PostgreSQL | `FORGE_TEST_DATABASE_URL` (a role that may create databases) | database `forge_test_<uuidv7>` |
//! | MySQL | `FORGE_TEST_MYSQL_URL` (same) | database `forge_test_<uuidv7>` |
//! | MariaDB | `FORGE_TEST_MARIADB_URL` via [`TestDatabase::create_from_env`] | database `forge_test_<uuidv7>` |
//! | SQLite | none | a file in a unique temporary directory |
//!
//! Failures never print the administrative URL.

#[cfg(any(feature = "postgres", feature = "mysql"))]
use std::{future::Future, pin::Pin};

use forge_config::SecretString;
use sqlx::migrate::Migrator;
#[cfg(any(feature = "postgres", feature = "mysql"))]
use sqlx::{AssertSqlSafe, Connection, Executor};
#[cfg(any(feature = "postgres", feature = "mysql"))]
use tracing::warn;
#[cfg(any(feature = "postgres", feature = "mysql"))]
use uuid::Uuid;

use crate::{Backend, Database, DbError};

/// Environment variable with the administrative URL of a MariaDB server, for
/// [`TestDatabase::create_from_env`] with the MySQL backend.
pub const MARIADB_URL_ENV: &str = "FORGE_TEST_MARIADB_URL";

#[cfg(any(feature = "postgres", feature = "mysql"))]
/// A boxed administrative statement.
#[doc(hidden)]
pub type BoxFuture = Pin<Box<dyn Future<Output = Result<(), DbError>> + Send>>;

#[cfg(any(feature = "postgres", feature = "mysql"))]
type AdminFn = fn(SecretString, String) -> BoxFuture;

/// A freshly provisioned database and the guard that removes it.
#[doc(hidden)]
pub struct Provision {
    url: SecretString,
    cleanup: Cleanup,
}

enum Action {
    #[cfg(any(feature = "postgres", feature = "mysql"))]
    Server {
        admin: SecretString,
        name: String,
        drop: AdminFn,
    },
    #[cfg(feature = "sqlite")]
    Directory(tempfile::TempDir),
}

/// Removes the provisioned database when dropped, panic or not.
struct Cleanup(Option<Action>);

impl Cleanup {
    async fn run(&mut self) -> Result<(), DbError> {
        let Some(action) = self.0.take() else {
            return Ok(());
        };
        match action {
            #[cfg(any(feature = "postgres", feature = "mysql"))]
            Action::Server { admin, name, drop } => drop(admin, name).await,
            #[cfg(feature = "sqlite")]
            Action::Directory(directory) => directory
                .close()
                .map_err(|_| DbError::configuration("could not remove the test directory")),
        }
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        // A directory removes itself when its `TempDir` drops.
        #[cfg(any(feature = "postgres", feature = "mysql"))]
        if let Some(Action::Server { admin, name, drop }) = self.0.take() {
            drop_server(admin, name, drop);
        }
    }
}

/// Drops a server database from a helper thread with its own runtime.
#[cfg(any(feature = "postgres", feature = "mysql"))]
fn drop_server(admin: SecretString, name: String, drop: AdminFn) {
    let database = name.clone();
    let outcome = std::thread::Builder::new()
        .name("forge-test-db-cleanup".to_owned())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| DbError::configuration("could not start the cleanup runtime"))?;
            runtime.block_on(drop(admin, name))
        })
        .map_err(|_| DbError::configuration("could not start the cleanup thread"))
        .and_then(|handle| {
            handle
                .join()
                .map_err(|_| DbError::configuration("the cleanup thread panicked"))?
        });
    if let Err(error) = outcome {
        warn!(database, %error, "failed to drop the test database");
    }
}

#[cfg(any(feature = "postgres", feature = "mysql"))]
/// Reads the administrative URL, creates `forge_test_<uuidv7>` and returns the
/// URL of the new database plus its cleanup guard.
pub(crate) async fn provision_server(
    admin_env: &str,
    create: AdminFn,
    drop: AdminFn,
) -> Result<Provision, DbError> {
    let admin = std::env::var(admin_env)
        .map(SecretString::new)
        .map_err(|_| DbError::configuration(format!("{admin_env} is not set")))?;
    let name = format!("forge_test_{}", Uuid::now_v7().simple());
    // Arm the guard before creating so a failure half way still cleans up.
    let cleanup = Cleanup(Some(Action::Server {
        admin: admin.clone(),
        name: name.clone(),
        drop,
    }));
    create(admin.clone(), name.clone()).await?;
    let url = with_database(admin.expose(), &name)
        .ok_or_else(|| DbError::configuration(format!("{admin_env} is not a valid URL")))?;
    Ok(Provision {
        url: SecretString::new(url),
        cleanup,
    })
}

#[cfg(any(feature = "postgres", feature = "mysql"))]
/// Replaces the database path of a `scheme://authority/database?query` URL,
/// keeping credentials, host and query parameters.
fn with_database(url: &str, name: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let (location, query) = match rest.split_once('?') {
        Some((location, query)) => (location, Some(query)),
        None => (rest, None),
    };
    let authority = location.split('/').next().filter(|a| !a.is_empty())?;
    let query = query.map(|query| format!("?{query}")).unwrap_or_default();
    Some(format!("{scheme}://{authority}/{name}{query}"))
}

/// A SQLite database in a unique temporary directory.
#[cfg(feature = "sqlite")]
pub(crate) fn provision_file() -> Result<Provision, DbError> {
    let directory = tempfile::Builder::new()
        .prefix("forge-test-")
        .tempdir()
        .map_err(|_| DbError::configuration("could not create a temporary directory"))?;
    let url = SecretString::new(format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("test.db").display()
    ));
    Ok(Provision {
        url,
        cleanup: Cleanup(Some(Action::Directory(directory))),
    })
}

#[cfg(any(feature = "postgres", feature = "mysql"))]
/// Runs one administrative statement on a dedicated connection.
///
/// `sql` embeds only an identifier generated locally from UUID hex, never
/// caller input.
pub(crate) async fn execute_admin<B>(admin: &SecretString, sql: String) -> Result<(), DbError>
where
    B: Backend,
    for<'c> &'c mut <B as sqlx::Database>::Connection: Executor<'c, Database = B>,
    <B as sqlx::Database>::Arguments: sqlx::IntoArguments<B>,
{
    let mut connection = <<B as sqlx::Database>::Connection as Connection>::connect(admin.expose())
        .await
        .map_err(DbError::from)?;
    let result = sqlx::query(AssertSqlSafe(sql))
        .execute(&mut connection)
        .await
        .map(|_| ())
        .map_err(DbError::from);
    // The statement result matters; a failed close of a one-shot connection does not.
    drop(connection.close().await);
    result
}

/// An isolated, migrated database for one test.
///
/// See the [module documentation](self) for provisioning and cleanup.
pub struct TestDatabase<B: Backend> {
    database: Database<B>,
    url: SecretString,
    cleanup: Cleanup,
}

impl<B: Backend> std::fmt::Debug for TestDatabase<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TestDatabase")
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> TestDatabase<B> {
    /// Provisions a database for this backend and applies `migrator`, using
    /// the backend's default administrative variable (`FORGE_TEST_DATABASE_URL`
    /// for PostgreSQL, `FORGE_TEST_MYSQL_URL` for MySQL; none for SQLite).
    pub async fn create(migrator: &Migrator) -> Result<Self, DbError> {
        Self::create_from_env(<B as crate::backend::sealed::Hooks>::TEST_URL_ENV, migrator).await
    }

    /// Like [`TestDatabase::create`] with an explicit administrative
    /// variable, e.g. [`MARIADB_URL_ENV`] to test the MySQL backend against
    /// MariaDB.
    pub async fn create_from_env(admin_env: &str, migrator: &Migrator) -> Result<Self, DbError> {
        let test = Self::create_empty_from_env(admin_env).await?;
        test.database.migrate(migrator).await?;
        Ok(test)
    }

    /// Provisions an empty database (no migrations) with the default
    /// administrative variable.
    pub async fn create_empty() -> Result<Self, DbError> {
        Self::create_empty_from_env(<B as crate::backend::sealed::Hooks>::TEST_URL_ENV).await
    }

    /// Provisions an empty database with an explicit administrative variable.
    pub async fn create_empty_from_env(admin_env: &str) -> Result<Self, DbError> {
        let Provision { url, cleanup } =
            <B as crate::backend::sealed::Hooks>::provision(admin_env).await?;
        // From here on `cleanup` removes the database on every exit path.
        let database = Database::connect(&url).await?;
        Ok(Self {
            database,
            url,
            cleanup,
        })
    }

    /// The migrated database.
    #[must_use]
    pub fn database(&self) -> &Database<B> {
        &self.database
    }

    /// The URL of the isolated database (contains credentials; handle as a
    /// secret).
    #[must_use]
    pub fn url(&self) -> &SecretString {
        &self.url
    }

    /// Closes the pool and removes the database, reporting failures.
    pub async fn cleanup(mut self) -> Result<(), DbError> {
        self.database.close().await;
        self.cleanup.run().await
    }
}

#[cfg(all(test, any(feature = "postgres", feature = "mysql")))]
mod tests {
    use super::*;

    #[test]
    fn database_path_is_replaced_and_the_rest_kept() {
        assert_eq!(
            with_database(
                "postgres://u:p%40@h:5432/postgres?sslmode=disable",
                "forge_test_1"
            )
            .as_deref(),
            Some("postgres://u:p%40@h:5432/forge_test_1?sslmode=disable")
        );
        assert_eq!(
            with_database("mariadb://root:forge@127.0.0.1:3306", "x").as_deref(),
            Some("mariadb://root:forge@127.0.0.1:3306/x")
        );
        assert_eq!(with_database("sqlite:file.db", "x"), None);
        assert_eq!(with_database("postgres:///db", "x"), None);
    }
}
