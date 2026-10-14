//! Command-line tooling for creating and maintaining Forge applications.

mod capabilities;
mod cargo;
mod cli;
mod doctor;
mod error;
mod generate;
mod page;
mod preview;
mod resource;

use std::env;
use std::io;

use uuid::Uuid;

use crate::error::CliParseDiagnostic;
pub use error::CliError;

use crate::capabilities::{CAPABILITY_SCHEMA_VERSION, CapabilityManifest};
use crate::cargo::CargoCommand;
use crate::cli::{Cli, Command, GenerateCommand, OutputFormat};
use crate::generate::{ForgeSource, NewApplication};
use crate::page::PageGenerator;
use crate::resource::ResourceGenerator;

/// Parses process arguments and executes the selected Forge command.
pub fn run() -> std::process::ExitCode {
    let cli = match Cli::parse_process() {
        Ok(cli) => cli,
        Err(error) => {
            let display_only = matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            if !display_only && env::args_os().any(|argument| argument == "--json-errors") {
                let stderr = io::stderr();
                let mut output = stderr.lock();
                if serde_json::to_writer(&mut output, &CliParseDiagnostic::invalid_arguments())
                    .is_ok()
                {
                    use std::io::Write as _;
                    let _ = writeln!(output);
                }
            } else {
                let _ = error.print();
            }
            return if display_only {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::from(2)
            };
        }
    };
    let json_errors = cli.json_errors;
    let result = env::current_dir()
        .map_err(CliError::CurrentDirectory)
        .and_then(|current_dir| execute(cli.command, &current_dir, &mut io::stdout()));
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            write_error(&error, json_errors);
            std::process::ExitCode::FAILURE
        }
    }
}

fn write_error(error: &CliError, json: bool) {
    use std::io::Write as _;

    let stderr = io::stderr();
    let mut output = stderr.lock();
    if json {
        if serde_json::to_writer(&mut output, &error.diagnostic()).is_ok() {
            let _ = writeln!(output);
        } else {
            let _ = writeln!(output, "error: {error}");
        }
    } else {
        let _ = writeln!(output, "error: {error}");
    }
}

fn execute(
    command: Command,
    current_dir: &std::path::Path,
    output: &mut dyn io::Write,
) -> Result<(), CliError> {
    match command {
        Command::Id => writeln!(output, "{}", Uuid::now_v7()).map_err(CliError::Output),
        Command::New {
            name,
            skip_docker,
            skip_ci,
            skip_database,
            skip_lockfile,
            forge_path,
        } => {
            let options = NewApplication {
                name,
                docker: !skip_docker,
                ci: !skip_ci,
                database: !skip_database,
                forge: forge_path.map_or(ForgeSource::GitTag, ForgeSource::Path),
            };
            let destination = generate::create_application(current_dir, &options)?;
            writeln!(
                output,
                "Created Forge application `{}` at {}",
                options.name,
                destination.display()
            )
            .map_err(CliError::Output)?;

            if !skip_lockfile && let Err(error) = CargoCommand::GenerateLockfile.run(&destination) {
                writeln!(
                    output,
                    "warning: {error}\n\
                         warning: the application was created, but `docker build` and \
                         `--locked` builds need Cargo.lock; run `cargo generate-lockfile` \
                         in {} once the `forge` dependency can be resolved",
                    destination.display()
                )
                .map_err(CliError::Output)?;
            }
            Ok(())
        }
        Command::Generate { generator } => match generator {
            GenerateCommand::Migration {
                name,
                version,
                dry_run,
            } => {
                if dry_run {
                    let preview = generate::preview_migration(current_dir, &name, version)?
                        .relative_to(current_dir);
                    serde_json::to_writer(&mut *output, &preview)
                        .map_err(|error| CliError::Output(io::Error::other(error)))?;
                    writeln!(output).map_err(CliError::Output)
                } else {
                    let (up, down) = generate::create_migration(current_dir, &name, version)?;
                    writeln!(
                        output,
                        "Created migration files:\n  {}\n  {}",
                        up.display(),
                        down.display()
                    )
                    .map_err(CliError::Output)
                }
            }
            GenerateCommand::Resource {
                name,
                fields,
                global,
                tenant_scoped: _,
                version,
                dry_run,
            } => {
                let generator = ResourceGenerator {
                    name: name.clone(),
                    fields,
                    tenant_scoped: !global,
                    version,
                };
                if dry_run {
                    let preview = generator.preview(current_dir)?.relative_to(current_dir);
                    serde_json::to_writer(&mut *output, &preview)
                        .map_err(|error| CliError::Output(io::Error::other(error)))?;
                    writeln!(output).map_err(CliError::Output)
                } else {
                    let destination = generator.generate(current_dir)?;
                    writeln!(
                        output,
                        "Generated resource `{name}` domain, application and PostgreSQL adapter code plus migration at {}",
                        destination.display()
                    )
                    .map_err(CliError::Output)
                }
            }
            GenerateCommand::Page { name, dry_run } => {
                let generator = PageGenerator { name: name.clone() };
                if dry_run {
                    let preview = generator.preview(current_dir)?.relative_to(current_dir);
                    serde_json::to_writer(&mut *output, &preview)
                        .map_err(|error| CliError::Output(io::Error::other(error)))?;
                    writeln!(output).map_err(CliError::Output)
                } else {
                    let (route, destination) = generator.generate(current_dir)?;
                    writeln!(
                        output,
                        "Generated server-rendered page `{name}` at `{route}` with automatic escaping in {}",
                        destination.display()
                    )
                    .map_err(CliError::Output)
                }
            }
        },
        Command::Check => CargoCommand::Check.run(current_dir),
        Command::Test => CargoCommand::Test.run(current_dir),
        Command::Format => CargoCommand::Format.run(current_dir),
        Command::Lint => CargoCommand::Lint.run(current_dir),
        Command::Build => CargoCommand::Build.run(current_dir),
        Command::Capabilities { format } => match format {
            OutputFormat::Json => {
                let path = current_dir.join(".forge/capabilities.json");
                let manifest = std::fs::read(&path).map_err(|source| {
                    if source.kind() == io::ErrorKind::NotFound {
                        CliError::CapabilityManifestNotFound(path.clone())
                    } else {
                        CliError::ReadFile {
                            path: path.clone(),
                            source,
                        }
                    }
                })?;
                let manifest: CapabilityManifest =
                    serde_json::from_slice(&manifest).map_err(|source| {
                        CliError::InvalidCapabilityManifest {
                            path: path.clone(),
                            source,
                        }
                    })?;
                if manifest.schema_version != CAPABILITY_SCHEMA_VERSION {
                    return Err(CliError::UnsupportedCapabilitySchema {
                        path,
                        version: manifest.schema_version,
                    });
                }
                serde_json::to_writer_pretty(&mut *output, &manifest)
                    .map_err(|error| CliError::Output(io::Error::other(error)))?;
                writeln!(output).map_err(CliError::Output)
            }
        },
        Command::Doctor {
            format,
            probe_database,
            timeout_ms,
        } => match format {
            OutputFormat::Json => {
                let report = doctor::DoctorReport::inspect(
                    current_dir,
                    probe_database,
                    std::time::Duration::from_millis(timeout_ms),
                );
                serde_json::to_writer_pretty(&mut *output, &report)
                    .map_err(|error| CliError::Output(io::Error::other(error)))?;
                writeln!(output).map_err(CliError::Output)
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_command_emits_a_version_seven_uuid() {
        let directory = tempfile::tempdir().expect("temporary directory should be available");
        let mut output = Vec::new();

        execute(Command::Id, directory.path(), &mut output).expect("id generation should succeed");

        let rendered = String::from_utf8(output).expect("UUID output should be UTF-8");
        let id = Uuid::parse_str(rendered.trim()).expect("output should be a UUID");
        assert_eq!(id.get_version_num(), 7);
    }
}
