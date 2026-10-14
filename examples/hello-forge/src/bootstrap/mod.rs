//! Composition root: selects concrete implementations and owns the process
//! lifecycle. Forge provides the runtime, configuration, structured logging,
//! health routes and graceful shutdown.

use std::process::ExitCode;

/// No database-backed adapters are needed by this application.
pub fn compose(_composition: &mut forge::Composition<'_>) -> Result<(), forge::ComposeError> {
    Ok(())
}

/// Dispatches framework commands and the generated app's one-shot account setup.
pub fn run() -> ExitCode {
    run_server()
}

fn run_server() -> ExitCode {
    forge::App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .routes(crate::adapters::http::routes)
        .compose(crate::bootstrap::compose)
        .run()
}
