//! Composition root: selects concrete implementations and owns the process
//! lifecycle. Forge provides the runtime, configuration, structured logging,
//! health routes and graceful shutdown.
//!
//! `sqlx::migrate!()` embeds `migrations/` into the binary; run them with
//! `<binary> migrate` (or `forge migrate`) and revert the latest with `rollback`.

use std::{process::ExitCode, sync::Arc};

use forge::{ComposeError, Composition, db::PgDatabase};

use crate::{adapters::persistence::PgProductRepository, application::use_cases::ProductCatalog};

/// Runs the command selected on the command line (`serve` by default).
pub fn run() -> ExitCode {
    forge::App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .postgres(sqlx::migrate!())
        .compose(compose)
        .run()
}

/// Builds the dependency graph once at startup and registers the routes.
pub fn compose(composition: &mut Composition<'_>) -> Result<(), ComposeError> {
    let catalog = Arc::new(product_catalog(composition.postgres()?));
    crate::adapters::http::routes(composition.router(), catalog)?;
    Ok(())
}

/// The product catalog use cases backed by PostgreSQL, exactly as served.
///
/// Public so integration tests exercise the production wiring.
#[must_use]
pub fn product_catalog(database: &PgDatabase) -> ProductCatalog<PgProductRepository> {
    ProductCatalog::new(PgProductRepository::new(database))
}
