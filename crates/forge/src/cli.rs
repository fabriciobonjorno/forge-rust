//! Command-line parsing for generated application binaries.

use std::ffi::OsString;

use thiserror::Error;

/// Process command selected by the first argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    /// Run the HTTP server (the default).
    Serve,
    /// Probe the local liveness endpoint for container health checks.
    Healthcheck,
    /// Print name and version.
    Version,
    /// Print usage.
    Help,
}

/// Invalid command line.
#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum UsageError {
    /// The first argument is not a known command.
    #[error("unknown command {0:?}")]
    UnknownCommand(String),
    /// A command was followed by arguments it does not accept.
    #[error("unexpected argument {0:?}")]
    UnexpectedArgument(String),
}

/// Parses process arguments, excluding the program name.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, UsageError> {
    let mut args = args.into_iter();
    let command = match args.next() {
        None => Command::Serve,
        Some(arg) => match arg.to_str() {
            Some("serve") => Command::Serve,
            Some("healthcheck") => Command::Healthcheck,
            Some("version" | "--version" | "-V") => Command::Version,
            Some("help" | "--help" | "-h") => Command::Help,
            _ => {
                return Err(UsageError::UnknownCommand(
                    arg.to_string_lossy().into_owned(),
                ));
            }
        },
    };
    if let Some(extra) = args.next() {
        return Err(UsageError::UnexpectedArgument(
            extra.to_string_lossy().into_owned(),
        ));
    }
    Ok(command)
}

/// Human-readable usage, including every supported configuration variable.
pub(crate) fn usage(name: &str) -> String {
    format!(
        "\
Usage: {name} [COMMAND]

Commands:
  serve        Run the HTTP server (default)
  healthcheck  Probe GET /health/live on the configured address; exit 0 when healthy
  version      Print the application name and version (also --version, -V)
  help         Print this help (also --help, -h)

Environment:
  FORGE_ENV                   development | test | production (default: development)
  FORGE_BIND                  listen socket address (default: 127.0.0.1:3000)
  FORGE_REQUEST_TIMEOUT_SECS  per-request deadline in seconds (default: 30)
  FORGE_SHUTDOWN_GRACE_SECS   connection drain period on shutdown in seconds (default: 15)
  FORGE_MAX_BODY_BYTES        maximum request body size in bytes (default: 1048576)
  FORGE_MAX_CONNECTIONS       maximum concurrent connections (default: 10000)
  FORGE_LOG                   log filter directives (default: info)
  FORGE_LOG_FORMAT            json | text (default: json in production, text otherwise)

Unknown FORGE_* variables are rejected at startup.
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_argument_serves() {
        assert_eq!(parse(args(&[])), Ok(Command::Serve));
        assert_eq!(parse(args(&["serve"])), Ok(Command::Serve));
    }

    #[test]
    fn known_commands_and_aliases_are_recognized() {
        assert_eq!(parse(args(&["healthcheck"])), Ok(Command::Healthcheck));
        for alias in ["version", "--version", "-V"] {
            assert_eq!(parse(args(&[alias])), Ok(Command::Version), "{alias}");
        }
        for alias in ["help", "--help", "-h"] {
            assert_eq!(parse(args(&[alias])), Ok(Command::Help), "{alias}");
        }
    }

    #[test]
    fn unknown_commands_are_rejected() {
        assert_eq!(
            parse(args(&["migrate"])),
            Err(UsageError::UnknownCommand("migrate".to_owned()))
        );
        assert_eq!(
            parse(args(&["-v"])),
            Err(UsageError::UnknownCommand("-v".to_owned()))
        );
    }

    #[test]
    fn trailing_arguments_are_rejected() {
        assert_eq!(
            parse(args(&["serve", "--port"])),
            Err(UsageError::UnexpectedArgument("--port".to_owned()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_arguments_are_rejected() {
        use std::os::unix::ffi::OsStringExt;

        let result = parse(vec![OsString::from_vec(vec![0xff])]);

        assert!(matches!(result, Err(UsageError::UnknownCommand(_))));
    }

    #[test]
    fn usage_documents_every_configuration_key() {
        let text = usage("demo");

        assert!(text.starts_with("Usage: demo [COMMAND]"));
        for command in ["serve", "healthcheck", "version", "help"] {
            assert!(text.contains(command), "{command}");
        }
        for key in forge_config::KNOWN_KEYS {
            assert!(text.contains(key), "{key}");
        }
    }
}
