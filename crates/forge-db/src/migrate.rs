//! Backend-independent migration runner built on SQLx's `Migrate` trait.
//!
//! It mirrors `Migrator::run` / `Migrator::undo` (lock, bookkeeping table,
//! dirty and checksum checks) but reports exactly which versions were applied
//! or reverted, and never leaves a session-scoped migration lock on a pooled
//! connection: after any failure the connection is closed instead of being
//! returned to the pool.

use std::collections::HashMap;

use sqlx::{
    Pool,
    migrate::{AppliedMigration, Migrate, MigrateError, Migrator},
    pool::PoolConnection,
};
use tracing::debug;

use crate::{Backend, DbError};

/// Applies pending migrations; returns the versions applied, ascending.
pub(crate) async fn apply_pending<B: Backend>(
    pool: &Pool<B>,
    migrator: &Migrator,
) -> Result<Vec<i64>, DbError> {
    let mut connection = pool.acquire().await?;
    let result = apply_locked(&mut *connection, migrator).await;
    finish(connection, result).await
}

/// Reverts the most recently applied migration.
pub(crate) async fn revert_latest<B: Backend>(
    pool: &Pool<B>,
    migrator: &Migrator,
) -> Result<Option<i64>, DbError> {
    let mut connection = pool.acquire().await?;
    let result = revert_locked(&mut *connection, migrator).await;
    finish(connection, result).await
}

async fn apply_locked<C: Migrate>(
    connection: &mut C,
    migrator: &Migrator,
) -> Result<Vec<i64>, DbError> {
    let applied = prepare(connection, migrator).await?;

    for migration in migrator
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
    {
        if let Some(existing) = applied.get(&migration.version)
            && existing.checksum != migration.checksum
        {
            return Err(MigrateError::VersionMismatch(migration.version).into());
        }
    }

    let mut newly_applied = Vec::new();
    for migration in migrator
        .iter()
        .filter(|m| m.migration_type.is_up_migration())
    {
        if applied.contains_key(&migration.version) {
            continue;
        }
        connection.apply(&migrator.table_name, migration).await?;
        newly_applied.push(migration.version);
    }
    release(connection, migrator).await?;
    Ok(newly_applied)
}

async fn revert_locked<C: Migrate>(
    connection: &mut C,
    migrator: &Migrator,
) -> Result<Option<i64>, DbError> {
    let applied = prepare(connection, migrator).await?;
    let Some(latest) = applied.keys().copied().max() else {
        release(connection, migrator).await?;
        return Ok(None);
    };
    let down = migrator
        .iter()
        .find(|m| m.version == latest && m.migration_type.is_down_migration())
        .ok_or(DbError::IrreversibleMigration { version: latest })?;
    connection.revert(&migrator.table_name, down).await?;
    release(connection, migrator).await?;
    Ok(Some(latest))
}

/// Takes the migration lock, creates the bookkeeping table and returns the
/// applied migrations by version after the same safety checks SQLx performs.
async fn prepare<C: Migrate>(
    connection: &mut C,
    migrator: &Migrator,
) -> Result<HashMap<i64, AppliedMigration>, DbError> {
    if migrator.locking {
        connection.lock().await?;
    }
    for schema in migrator.create_schemas.iter() {
        connection.create_schema_if_not_exists(schema).await?;
    }
    connection
        .ensure_migrations_table(&migrator.table_name)
        .await?;
    if let Some(version) = connection.dirty_version(&migrator.table_name).await? {
        return Err(MigrateError::Dirty(version).into());
    }
    let applied = connection
        .list_applied_migrations(&migrator.table_name)
        .await?;
    if !migrator.ignore_missing
        && let Some(missing) = applied
            .iter()
            .map(|m| m.version)
            .find(|version| !migrator.version_exists(*version))
    {
        return Err(MigrateError::VersionMissing(missing).into());
    }
    Ok(applied.into_iter().map(|m| (m.version, m)).collect())
}

async fn release<C: Migrate>(connection: &mut C, migrator: &Migrator) -> Result<(), DbError> {
    if migrator.locking {
        connection.unlock().await?;
    }
    Ok(())
}

/// Returns the connection to the pool on success. After a failure it may still
/// hold the session-level migration lock, so it is closed instead; ending the
/// session releases every lock it holds.
async fn finish<B: Backend, T>(
    connection: PoolConnection<B>,
    result: Result<T, DbError>,
) -> Result<T, DbError> {
    if result.is_err() {
        use sqlx::Connection as _;
        if let Err(error) = connection.detach().close().await {
            debug!(%error, "failed to close the migration connection cleanly");
        }
    }
    result
}
