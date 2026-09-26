use std::path::Path;
use std::process::Command;

use crate::CliError;

pub(crate) enum CargoCommand {
    Check,
    Test,
    Format,
    Lint,
    Build,
    GenerateLockfile,
}

impl CargoCommand {
    pub(crate) fn run(self, current_dir: &Path) -> Result<(), CliError> {
        let (display_name, arguments): (&'static str, &[&str]) = match self {
            Self::Check => ("check", &["check", "--all-targets", "--all-features"]),
            Self::Test => ("test", &["test", "--all-targets", "--all-features"]),
            Self::Format => ("fmt", &["fmt", "--all"]),
            Self::Build => ("build", &["build", "--release", "--locked"]),
            Self::GenerateLockfile => ("generate-lockfile", &["generate-lockfile"]),
            Self::Lint => (
                "clippy",
                &[
                    "clippy",
                    "--all-targets",
                    "--all-features",
                    "--",
                    "-D",
                    "warnings",
                ],
            ),
        };

        let status = Command::new("cargo")
            .args(arguments)
            .current_dir(current_dir)
            .status()
            .map_err(|source| CliError::SpawnCargo {
                command: display_name,
                source,
            })?;

        if status.success() {
            Ok(())
        } else {
            Err(CliError::CargoFailed {
                command: display_name,
                code: status.code(),
            })
        }
    }
}
