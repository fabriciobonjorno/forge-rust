use std::collections::HashSet;

use sqlx::{
    Connection,
    migrate::{Migrate, MigrateError, Migrator},
    pool::{Pool, PoolConnection},
};
use tracing::debug;

use crate::{Backend, DbError};

/// Applies pending migrations; returns the versions applied, ascending.
pub(crate) async fn apply_pending<B>(
    pool: &Pool<B>,
    migrator: &Migrator,
) -> Result<Vec<i64>, DbError>
where
    B: Backend,
    <B as sqlx::Database>::Connection: Migrate,
{
    let mut connection = pool.acquire().await?;
    let result = apply_pending_locked::<B>(&mut *connection, migrator).await;
    if result.is_err() {
        discard(connection).await;
    }
    result
}

/// Reverts the most recently applied migration.
pub(crate) async fn revert_latest<B>(
    pool: &Pool<B>,
    migrator: &Migrator,
) -> Result<Option<i64>, DbError>
where
    B: Backend,
    <B as sqlx::Database>::Connection: Migrate,
{
    let mut connection = pool.acquire().await?;
    let result = revert_latest_locked::<B>(&mut *connection, migrator).await;
    if result.is_err() {
        discard(connection).await;
    }
    result
}

async fn apply_pending_locked<B>(
    connection: &mut <B as sqlx::Database>::Connection,
    migrator: &Migrator,
) -> Result<Vec<i64>, DbError>
where
    B: Backend,
    <B as sqlx::Database>::Connection: Migrate,
{
    <B as crate::backend::sealed::Hooks>::prepare_migration_session(connection).await?;
    connection.lock().await?;
    let before = applied_versions(connection, migrator).await?;
    // `run_direct` is `Migrator::run` on a borrowed connection (SQLx exposes
    // it because `Acquire` bounds do not compose generically). It takes the
    // (reentrant) lock again, rejects dirty databases and missing migrations,
    // and verifies checksums of already-applied migrations.
    migrator.run_direct(None, connection, false).await?;
    let after = applied_versions(connection, migrator).await?;
    connection.unlock().await?;
    <B as crate::backend::sealed::Hooks>::restore_session(connection).await?;

    let before: HashSet<i64> = before.into_iter().collect();
    Ok(after
        .into_iter()
        .filter(|version| !before.contains(version))
        .collect())
}

async fn revert_latest_locked<B>(
    connection: &mut <B as sqlx::Database>::Connection,
    migrator: &Migrator,
) -> Result<Option<i64>, DbError>
where
    B: Backend,
    <B as sqlx::Database>::Connection: Migrate,
{
    <B as crate::backend::sealed::Hooks>::prepare_migration_session(connection).await?;
    connection.lock().await?;
    let applied = applied_versions(connection, migrator).await?;
    let Some(latest) = applied.iter().copied().max() else {
        connection.unlock().await?;
        <B as crate::backend::sealed::Hooks>::restore_session(connection).await?;
        return Ok(None);
    };
    // The same checks `Migrator::undo` performs before reverting.
    if let Some(version) = connection.dirty_version(&migrator.table_name).await? {
        return Err(MigrateError::Dirty(version).into());
    }
    if !migrator.ignore_missing
        && let Some(missing) = applied
            .iter()
            .copied()
            .find(|version| !migrator.version_exists(*version))
    {
        return Err(MigrateError::VersionMissing(missing).into());
    }
    let Some(down) = migrator.iter().find(|migration| {
        migration.version == latest && migration.migration_type.is_down_migration()
    }) else {
        return Err(DbError::Configuration(sqlx::Error::Configuration(
            "migration has no down migration".into(),
        )));
    };
    connection.revert(&migrator.table_name, down).await?;
    connection.unlock().await?;
    <B as crate::backend::sealed::Hooks>::restore_session(connection).await?;
    Ok(Some(latest))
}

/// Lists applied versions, creating the bookkeeping table (and any schemas
/// the migrator is configured to create) when missing.
async fn applied_versions<C: Migrate>(
    connection: &mut C,
    migrator: &Migrator,
) -> Result<Vec<i64>, DbError> {
    for schema in migrator.create_schemas.iter() {
        connection.create_schema_if_not_exists(schema).await?;
    }
    connection
        .ensure_migrations_table(&migrator.table_name)
        .await?;
    let mut versions: Vec<i64> = connection
        .list_applied_migrations(&migrator.table_name)
        .await?
        .into_iter()
        .map(|migration| migration.version)
        .collect();
    versions.sort_unstable();
    Ok(versions)
}

/// Closes a connection that may still hold the session-level migration
/// lock instead of returning it to the pool; ending the session releases
/// every lock it holds.
async fn discard<B: Backend>(connection: PoolConnection<B>) {
    if let Err(error) = connection.detach().close().await {
        debug!(%error, "failed to close migration connection cleanly");
    }
}
