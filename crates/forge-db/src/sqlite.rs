//! SQLite backend implementation.

use crate::{Backend, DbError};
use forge_config::DatabaseScheme;
use sqlx::{
    Connection, Pool,
    migrate::Migrator,
    pool::PoolOptions,
    sqlite::{Sqlite, SqliteConnectOptions},
};
use std::str::FromStr;

use super::sealed::Hooks;

impl Hooks for Sqlite {
    const SCHEME: DatabaseScheme = DatabaseScheme::Sqlite;

    type Connection = <Sqlite as sqlx::Database>::Connection;

    fn connect_options(
        url: &str,
    ) -> Result<<<Self as Hooks>::Connection as Connection>::Options, sqlx::Error> {
        let options = SqliteConnectOptions::from_str(url)?;
        Ok(options)
    }

    fn pool_options(url: &str, options: PoolOptions<Self>) -> PoolOptions<Self> {
        let _ = url;
        options
    }

    fn apply_pending<'a>(
        _pool: &'a Pool<Self>,
        _migrator: &'a Migrator,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<i64>, DbError>> + Send + 'a>>
    {
        Box::pin(std::future::ready(Ok(vec![])))
    }

    fn revert_latest<'a>(
        _pool: &'a Pool<Self>,
        _migrator: &'a Migrator,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<i64>, DbError>> + Send + 'a>,
    > {
        Box::pin(std::future::ready(Ok(None)))
    }

    fn prepare_migration_session(
        connection: &mut <Self as Hooks>::Connection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DbError>> + Send + '_>> {
        let _ = connection;
        Box::pin(std::future::ready(Ok(())))
    }

    fn restore_session(
        connection: &mut <Self as Hooks>::Connection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DbError>> + Send + '_>> {
        let _ = connection;
        Box::pin(std::future::ready(Ok(())))
    }

    #[cfg(feature = "testing")]
    fn provision_test_database(
        _name: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::testing::Provisioned, DbError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(std::future::ready(Err(DbError::Configuration(
            sqlx::Error::Configuration("provision_test_database not implemented for SQLite".into()),
        ))))
    }
}

impl Backend for Sqlite {
    const SCHEME: DatabaseScheme = DatabaseScheme::Sqlite;
}
