use std::fmt;
use std::io;
use std::path::PathBuf;

use serde::Serialize;

/// Stable machine-readable error returned when `--json-errors` is enabled.
#[derive(Debug, Serialize)]
pub(crate) struct CliDiagnostic {
    pub(crate) schema_version: u32,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) path: Option<String>,
    pub(crate) hint: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CliParseDiagnostic {
    pub(crate) schema_version: u32,
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
    pub(crate) path: Option<String>,
    pub(crate) hint: &'static str,
}

impl CliParseDiagnostic {
    pub(crate) fn invalid_arguments() -> Self {
        Self {
            schema_version: 1,
            code: "cli_argument_error",
            message: "Command-line arguments are invalid.",
            path: None,
            hint: "Check the command usage with `forge --help` and its subcommand help.",
        }
    }
}

#[derive(Debug)]
pub enum CliError {
    CurrentDirectory(io::Error),
    InvalidApplicationName {
        name: String,
        reason: &'static str,
    },
    InvalidForgePath {
        path: String,
        reason: &'static str,
    },
    InvalidMigrationName {
        name: String,
        reason: &'static str,
    },
    InvalidResourceName {
        name: String,
        reason: &'static str,
    },
    InvalidResourceField {
        name: String,
        reason: &'static str,
    },
    InvalidPageName {
        name: String,
    },
    UnsafeSymlinkPath(PathBuf),
    PageScaffoldMissing(PathBuf),
    ResourceRequiresDatabase(PathBuf),
    ResourceModuleAlreadyRegistered(String),
    CapabilityManifestNotFound(PathBuf),
    InvalidCapabilityManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedCapabilitySchema {
        path: PathBuf,
        version: u32,
    },
    SerializeCapabilityManifest(serde_json::Error),
    Clock(std::time::SystemTimeError),
    DestinationExists(PathBuf),
    CreateDirectory {
        path: PathBuf,
        source: io::Error,
    },
    WriteFile {
        path: PathBuf,
        source: io::Error,
    },
    ReadFile {
        path: PathBuf,
        source: io::Error,
    },
    SpawnCargo {
        command: &'static str,
        source: io::Error,
    },
    CargoFailed {
        command: &'static str,
        code: Option<i32>,
    },
    SpawnRustfmt {
        path: PathBuf,
        source: io::Error,
    },
    RustfmtFailed {
        path: PathBuf,
        code: Option<i32>,
    },
    Output(io::Error),
}

impl CliError {
    pub(crate) fn diagnostic(&self) -> CliDiagnostic {
        let (code, path, hint) = match self {
            Self::CurrentDirectory(_) => (
                "current_directory_error",
                None,
                Some("Run Forge from a directory you can access.".to_owned()),
            ),
            Self::InvalidApplicationName { reason, .. } => {
                ("invalid_application_name", None, Some((*reason).to_owned()))
            }
            Self::InvalidForgePath { reason, .. } => {
                ("invalid_forge_path", None, Some((*reason).to_owned()))
            }
            Self::InvalidMigrationName { reason, .. } => {
                ("invalid_migration_name", None, Some((*reason).to_owned()))
            }
            Self::InvalidResourceName { reason, .. } => {
                ("invalid_resource_name", None, Some((*reason).to_owned()))
            }
            Self::InvalidResourceField { reason, .. } => {
                ("invalid_resource_field", None, Some((*reason).to_owned()))
            }
            Self::InvalidPageName { .. } => (
                "invalid_page_name",
                None,
                Some(
                    "Use a PascalCase ASCII page name such as `Home` or `UserProfile`.".to_owned(),
                ),
            ),
            Self::UnsafeSymlinkPath(path) => (
                "unsafe_symlink_path",
                Some(path),
                Some("Replace the symlink with a regular file or directory inside the application root.".to_owned()),
            ),
            Self::PageScaffoldMissing(path) => (
                "page_scaffold_missing",
                Some(path),
                Some(
                    "Generate the application with a Forge CLI that includes the views scaffold."
                        .to_owned(),
                ),
            ),
            Self::ResourceRequiresDatabase(path) => (
                "resource_requires_database",
                Some(path),
                Some(
                    "Generate this resource inside an app created with database support."
                        .to_owned(),
                ),
            ),
            Self::ResourceModuleAlreadyRegistered(_) => (
                "resource_module_already_registered",
                None,
                Some("Choose a different resource name or inspect the existing module.".to_owned()),
            ),
            Self::CapabilityManifestNotFound(path) => (
                "capability_manifest_missing",
                Some(path),
                Some("Run this command from an app created with `forge new`.".to_owned()),
            ),
            Self::InvalidCapabilityManifest { path, .. } => (
                "capability_manifest_invalid",
                Some(path),
                Some(
                    "Repair or regenerate the capability manifest with a compatible Forge CLI."
                        .to_owned(),
                ),
            ),
            Self::UnsupportedCapabilitySchema { path, .. } => (
                "capability_schema_unsupported",
                Some(path),
                Some("Use a Forge CLI that supports this manifest version.".to_owned()),
            ),
            Self::SerializeCapabilityManifest(_) => (
                "capability_manifest_serialize_error",
                None,
                Some("Inspect the capability manifest structure and retry.".to_owned()),
            ),
            Self::Clock(_) => (
                "system_clock_error",
                None,
                Some("Check the system clock and retry.".to_owned()),
            ),
            Self::DestinationExists(path) => (
                "destination_exists",
                Some(path),
                Some(
                    "Choose another destination; Forge does not overwrite existing files."
                        .to_owned(),
                ),
            ),
            Self::CreateDirectory { path, .. } => (
                "directory_create_error",
                Some(path),
                Some("Check parent directory permissions and available storage.".to_owned()),
            ),
            Self::WriteFile { path, .. } => (
                "file_write_error",
                Some(path),
                Some("Check file permissions and available storage.".to_owned()),
            ),
            Self::ReadFile { path, .. } => (
                "file_read_error",
                Some(path),
                Some("Check that the file exists and is readable.".to_owned()),
            ),
            Self::SpawnCargo { .. } => (
                "cargo_spawn_error",
                None,
                Some("Install Cargo or make it available in PATH.".to_owned()),
            ),
            Self::CargoFailed { .. } => (
                "cargo_command_failed",
                None,
                Some("Inspect Cargo's output, fix the reported issue, and retry.".to_owned()),
            ),
            Self::SpawnRustfmt { path, .. } | Self::RustfmtFailed { path, .. } => (
                "rustfmt_failed",
                Some(path),
                Some("Install rustfmt for the active Rust toolchain and retry.".to_owned()),
            ),
            Self::Output(_) => (
                "output_write_error",
                None,
                Some("Check that the output stream is writable.".to_owned()),
            ),
        };

        CliDiagnostic {
            schema_version: 1,
            code,
            message: self.to_string(),
            path: path.map(|path| path.to_string_lossy().into_owned()),
            hint,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDirectory(error) => {
                write!(
                    formatter,
                    "could not determine the current directory: {error}"
                )
            }
            Self::InvalidApplicationName { name, reason } => {
                write!(formatter, "invalid application name `{name}`: {reason}")
            }
            Self::InvalidForgePath { path, reason } => {
                write!(formatter, "invalid --forge-path {path:?}: {reason}")
            }
            Self::InvalidMigrationName { name, reason } => {
                write!(formatter, "invalid migration name {name:?}: {reason}")
            }
            Self::InvalidResourceName { name, reason } => {
                write!(formatter, "invalid resource name {name:?}: {reason}")
            }
            Self::InvalidResourceField { name, reason } => {
                write!(formatter, "invalid resource field {name:?}: {reason}")
            }
            Self::InvalidPageName { name } => write!(
                formatter,
                "invalid page name {name:?}: use a PascalCase ASCII identifier of at most 64 characters"
            ),
            Self::UnsafeSymlinkPath(path) => write!(
                formatter,
                "refusing to follow a symlink while generating at `{}`",
                path.display()
            ),
            Self::PageScaffoldMissing(path) => write!(
                formatter,
                "page generation requires the Forge views scaffold at `{}`",
                path.display()
            ),
            Self::ResourceRequiresDatabase(path) => write!(
                formatter,
                "resource generation requires a database-enabled Forge application at `{}`",
                path.display()
            ),
            Self::ResourceModuleAlreadyRegistered(module) => {
                write!(
                    formatter,
                    "resource module `{module}` is already registered"
                )
            }
            Self::CapabilityManifestNotFound(path) => write!(
                formatter,
                "no Forge capability manifest found at `{}`; run this command inside an app created by `forge new`",
                path.display()
            ),
            Self::InvalidCapabilityManifest { path, source } => write!(
                formatter,
                "invalid Forge capability manifest at `{}`: {source}",
                path.display()
            ),
            Self::UnsupportedCapabilitySchema { path, version } => write!(
                formatter,
                "unsupported Forge capability schema version {version} at `{}`",
                path.display()
            ),
            Self::SerializeCapabilityManifest(source) => {
                write!(
                    formatter,
                    "could not serialize capability manifest: {source}"
                )
            }
            Self::Clock(error) => {
                write!(formatter, "system clock is before the Unix epoch: {error}")
            }
            Self::DestinationExists(path) => write!(
                formatter,
                "destination `{}` already exists; refusing to overwrite it",
                path.display()
            ),
            Self::CreateDirectory { path, source } => {
                write!(formatter, "could not create `{}`: {source}", path.display())
            }
            Self::WriteFile { path, source } => {
                write!(formatter, "could not write `{}`: {source}", path.display())
            }
            Self::ReadFile { path, source } => {
                write!(formatter, "could not read `{}`: {source}", path.display())
            }
            Self::SpawnCargo { command, source } => {
                write!(formatter, "could not run `cargo {command}`: {source}")
            }
            Self::CargoFailed { command, code } => match code {
                Some(code) => write!(formatter, "`cargo {command}` failed with exit code {code}"),
                None => write!(formatter, "`cargo {command}` was terminated by a signal"),
            },
            Self::SpawnRustfmt { path, source } => write!(
                formatter,
                "could not format generated Rust file `{}`: {source}",
                path.display()
            ),
            Self::RustfmtFailed { path, code } => match code {
                Some(code) => write!(
                    formatter,
                    "rustfmt failed with exit code {code} for `{}`",
                    path.display()
                ),
                None => write!(
                    formatter,
                    "rustfmt was terminated by a signal for `{}`",
                    path.display()
                ),
            },
            Self::Output(error) => write!(formatter, "could not write command output: {error}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CurrentDirectory(error)
            | Self::Output(error)
            | Self::CreateDirectory { source: error, .. }
            | Self::WriteFile { source: error, .. }
            | Self::ReadFile { source: error, .. }
            | Self::SpawnCargo { source: error, .. }
            | Self::SpawnRustfmt { source: error, .. } => Some(error),
            Self::Clock(error) => Some(error),
            Self::InvalidApplicationName { .. }
            | Self::InvalidForgePath { .. }
            | Self::InvalidMigrationName { .. }
            | Self::InvalidResourceName { .. }
            | Self::InvalidResourceField { .. }
            | Self::InvalidPageName { .. }
            | Self::UnsafeSymlinkPath(_)
            | Self::PageScaffoldMissing(_)
            | Self::ResourceRequiresDatabase(_)
            | Self::ResourceModuleAlreadyRegistered(_)
            | Self::CapabilityManifestNotFound(_)
            | Self::UnsupportedCapabilitySchema { .. }
            | Self::DestinationExists(_)
            | Self::CargoFailed { .. }
            | Self::RustfmtFailed { .. } => None,
            Self::InvalidCapabilityManifest { source, .. } => Some(source),
            Self::SerializeCapabilityManifest(source) => Some(source),
        }
    }
}
