//! Process-wide `tracing` subscriber installation.

use std::io::IsTerminal;

use forge_config::{LogConfig, LogFormat};
use thiserror::Error;
use tracing_subscriber::{EnvFilter, filter::ParseError};

/// `FORGE_LOG` could not be parsed as filter directives.
///
/// `ParseError`'s message already embeds its own cause, so it is displayed
/// inline rather than exposed as a source to avoid repeating it.
#[derive(Debug, Error)]
#[error("invalid value for FORGE_LOG: {filter:?}: {reason}")]
pub(crate) struct InvalidLogFilter {
    filter: String,
    reason: ParseError,
}

/// Installs the global subscriber writing to standard output.
///
/// If a global subscriber is already installed (for example by a test
/// harness embedding the application) it is kept and this call succeeds.
pub(crate) fn init(config: &LogConfig) -> Result<(), InvalidLogFilter> {
    let filter = EnvFilter::try_new(&config.filter).map_err(|reason| InvalidLogFilter {
        filter: config.filter.clone(),
        reason,
    })?;
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    let installed = match config.format {
        LogFormat::Json => builder.json().with_ansi(false).try_init(),
        LogFormat::Text => builder
            .compact()
            .with_ansi(std::io::stdout().is_terminal())
            .try_init(),
    };
    // `try_init` fails only when a global subscriber already exists; keep it.
    drop(installed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_filter_is_rejected() {
        let config = LogConfig {
            filter: "my_app=loudest".to_owned(),
            format: LogFormat::Text,
        };

        let error = init(&config).expect_err("invalid level must be rejected");

        assert!(error.to_string().contains("FORGE_LOG"));
    }

    #[test]
    fn repeated_initialization_does_not_panic() {
        for format in [LogFormat::Json, LogFormat::Text] {
            let config = LogConfig {
                filter: "info,forge=debug".to_owned(),
                format,
            };
            init(&config).expect("valid filter must initialize");
        }
    }
}
