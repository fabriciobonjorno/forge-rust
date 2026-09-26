//! Process shutdown signals.

use std::{future::Future, io};

/// Installs shutdown signal handlers and returns a future that resolves with
/// the name of the first received signal.
///
/// Handlers are registered eagerly so that a registration failure is reported
/// before the server starts. On Unix both `SIGTERM` (container runtimes) and
/// `SIGINT` (interactive Ctrl-C) trigger shutdown. Must be called within a
/// Tokio runtime.
#[cfg(unix)]
pub(crate) fn shutdown_signal() -> io::Result<impl Future<Output = &'static str> + Send + 'static> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => "SIGTERM",
            _ = interrupt.recv() => "SIGINT",
        }
    })
}

/// Installs a Ctrl-C handler and returns a future that resolves when it fires.
///
/// If listening fails the future resolves immediately so the failure causes a
/// visible shutdown instead of an unstoppable process.
#[cfg(not(unix))]
pub(crate) fn shutdown_signal() -> io::Result<impl Future<Output = &'static str> + Send + 'static> {
    Ok(async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => "ctrl-c",
            Err(error) => {
                tracing::error!(%error, "failed to listen for ctrl-c");
                "ctrl-c listener failure"
            }
        }
    })
}
