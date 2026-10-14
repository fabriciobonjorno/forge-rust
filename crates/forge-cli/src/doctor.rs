//! Read-only, machine-readable diagnostics for a Forge project directory.

use std::{path::Path, time::Duration};

use serde::Serialize;

use crate::capabilities::{CAPABILITY_SCHEMA_VERSION, CapabilityManifest};

/// Versioned report produced by `forge doctor`.
#[derive(Debug, Serialize)]
pub(crate) struct DoctorReport {
    /// Version of this JSON contract.
    pub(crate) schema_version: u32,
    /// Overall static project assessment.
    pub(crate) status: DoctorStatus,
    /// Explicitly describes the checks this command does not perform.
    pub(crate) scope: String,
    /// Individual checks with stable codes.
    pub(crate) checks: Vec<DoctorCheck>,
}

/// Overall doctor status.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DoctorStatus {
    /// No static project/environment issue was found.
    Ready,
    /// One or more checks need attention.
    Attention,
    /// This directory is not a Forge-generated application.
    NotAForgeApp,
}

/// Category separates source problems from local environment prerequisites.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckCategory {
    Project,
    Environment,
    ExternalDependency,
}

/// Result of one deterministic, read-only diagnostic.
#[derive(Debug, Serialize)]
pub(crate) struct DoctorCheck {
    /// Stable machine-readable check code.
    pub(crate) code: &'static str,
    /// Where the issue belongs.
    pub(crate) category: CheckCategory,
    /// `pass`, `warning`, `error`, `not_checked`, or `not_applicable`.
    pub(crate) status: CheckStatus,
    /// Relative path, when the check concerns a project file.
    pub(crate) path: Option<&'static str>,
    /// Safe human-readable summary.
    pub(crate) message: String,
    /// Next action, if one is useful.
    pub(crate) hint: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckStatus {
    Pass,
    Warning,
    Error,
    NotChecked,
    NotApplicable,
}

impl DoctorCheck {
    fn pass(
        code: &'static str,
        category: CheckCategory,
        path: Option<&'static str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            category,
            status: CheckStatus::Pass,
            path,
            message: message.into(),
            hint: None,
        }
    }

    fn finding(
        code: &'static str,
        category: CheckCategory,
        status: CheckStatus,
        path: Option<&'static str>,
        message: impl Into<String>,
        hint: Option<String>,
    ) -> Self {
        Self {
            code,
            category,
            status,
            path,
            message: message.into(),
            hint,
        }
    }
}

impl DoctorReport {
    /// Checks project structure and the presence of required environment
    /// variables without opening database connections or executing Cargo.
    pub(crate) fn inspect(root: &Path, probe_database: bool, timeout: Duration) -> Self {
        let manifest_path = root.join(".forge/capabilities.json");
        let mut checks = Vec::new();
        let mut did_probe_database = false;
        let manifest = match std::fs::read(&manifest_path) {
            Ok(bytes) => match serde_json::from_slice::<CapabilityManifest>(&bytes) {
                Ok(manifest) if manifest.schema_version == CAPABILITY_SCHEMA_VERSION => {
                    checks.push(DoctorCheck::pass(
                        "capability_manifest_valid",
                        CheckCategory::Project,
                        Some(".forge/capabilities.json"),
                        "Forge capability manifest is present and has a supported structure.",
                    ));
                    Some(manifest)
                }
                Ok(manifest) => {
                    checks.push(DoctorCheck::finding(
                        "capability_schema_unsupported",
                        CheckCategory::Project,
                        CheckStatus::Error,
                        Some(".forge/capabilities.json"),
                        format!("Capability manifest schema version {} is not supported by this CLI.", manifest.schema_version),
                        Some("Use a Forge CLI that supports this schema version, or regenerate the manifest after checking the project.".to_owned()),
                    ));
                    None
                }
                Err(_) => {
                    checks.push(DoctorCheck::finding(
                        "capability_manifest_invalid",
                        CheckCategory::Project,
                        CheckStatus::Error,
                        Some(".forge/capabilities.json"),
                        "Forge capability manifest is malformed or uses an unknown structure.",
                        Some("Regenerate this project with a compatible Forge CLI, or repair the manifest schema.".to_owned()),
                    ));
                    None
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                checks.push(DoctorCheck::finding(
                    "capability_manifest_missing",
                    CheckCategory::Project,
                    CheckStatus::Error,
                    Some(".forge/capabilities.json"),
                    "This directory does not contain a Forge-generated capability manifest.",
                    Some(
                        "Run `forge doctor` from a generated app, or create one with `forge new`."
                            .to_owned(),
                    ),
                ));
                None
            }
            Err(_) => {
                checks.push(DoctorCheck::finding(
                    "capability_manifest_unreadable",
                    CheckCategory::Project,
                    CheckStatus::Error,
                    Some(".forge/capabilities.json"),
                    "Forge could not read the capability manifest.",
                    Some("Check file permissions and retry.".to_owned()),
                ));
                None
            }
        };

        if manifest.is_some() {
            check_file(
                root,
                &mut checks,
                "agent_instructions_present",
                "AGENTS.md",
                "Generated agent instructions are present.",
            );
            check_file(
                root,
                &mut checks,
                "agent_reference_present",
                "docs/for-ai.md",
                "Detailed AI authoring guide is present.",
            );
            check_file(
                root,
                &mut checks,
                "rust_toolchain_present",
                "rust-toolchain.toml",
                "Pinned Rust toolchain file is present.",
            );
            if manifest
                .as_ref()
                .is_some_and(|manifest| manifest.features.docker || manifest.features.ci)
            {
                check_file(
                    root,
                    &mut checks,
                    "cargo_lock_present",
                    "Cargo.lock",
                    "Dependency lockfile is present.",
                );
            } else {
                checks.push(DoctorCheck::finding(
                    "cargo_lock_not_required_by_scaffold",
                    CheckCategory::Project,
                    CheckStatus::NotApplicable,
                    None,
                    "This scaffold has neither Docker nor CI commands that require a committed lockfile.",
                    None,
                ));
            }
        }

        if let Some(manifest) = &manifest {
            if manifest.features.docker {
                for (code, path) in [
                    ("dockerfile_present", "Dockerfile"),
                    ("docker_compose_present", "compose.yaml"),
                    ("dockerignore_present", ".dockerignore"),
                ] {
                    check_file(root, &mut checks, code, path, "Docker asset is present.");
                }
            } else {
                checks.push(DoctorCheck::finding(
                    "docker_assets_not_requested",
                    CheckCategory::Project,
                    CheckStatus::NotApplicable,
                    None,
                    "Docker assets were not selected when this app was generated.",
                    None,
                ));
            }

            if manifest.features.database.is_some() {
                check_environment_variable(
                    &mut checks,
                    "runtime_database_url_configured",
                    "FORGE_DATABASE_URL",
                );
                check_environment_variable(
                    &mut checks,
                    "migration_database_url_configured",
                    "FORGE_MIGRATION_DATABASE_URL",
                );
                if probe_database {
                    did_probe_database = true;
                    probe_database_url(
                        &mut checks,
                        "runtime_database_tcp_reachable",
                        "FORGE_DATABASE_URL",
                        timeout,
                    );
                    probe_database_url(
                        &mut checks,
                        "migration_database_tcp_reachable",
                        "FORGE_MIGRATION_DATABASE_URL",
                        timeout,
                    );
                } else {
                    checks.push(DoctorCheck::finding(
                        "database_connectivity_not_probed",
                        CheckCategory::ExternalDependency,
                        CheckStatus::NotChecked,
                        None,
                        "Database connectivity was not probed.",
                        Some("Use `forge doctor --probe-database` for a bounded TCP reachability check; it does not authenticate or query data.".to_owned()),
                    ));
                }
            }
        }

        let has_error = checks
            .iter()
            .any(|check| matches!(check.status, CheckStatus::Error));
        let has_warning = checks
            .iter()
            .any(|check| matches!(check.status, CheckStatus::Warning));
        let not_a_forge_app = checks
            .iter()
            .any(|check| check.code == "capability_manifest_missing");
        let status = if not_a_forge_app {
            DoctorStatus::NotAForgeApp
        } else if has_error || has_warning {
            DoctorStatus::Attention
        } else {
            DoctorStatus::Ready
        };

        let scope = if did_probe_database {
            "static local checks plus bounded TCP reachability probes; no Cargo command, build, test, database authentication, or SQL query was executed"
        } else {
            "static local checks only; no Cargo command, build, test, network request, or database connection was executed"
        };
        Self {
            schema_version: CAPABILITY_SCHEMA_VERSION,
            status,
            scope: scope.to_owned(),
            checks,
        }
    }
}

fn probe_database_url(
    checks: &mut Vec<DoctorCheck>,
    code: &'static str,
    variable: &'static str,
    timeout: Duration,
) {
    let value = match std::env::var(variable) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            checks.push(DoctorCheck::finding(
                code,
                CheckCategory::ExternalDependency,
                CheckStatus::NotApplicable,
                None,
                format!("{variable} is unset; no endpoint was probed."),
                None,
            ));
            return;
        }
    };
    let endpoint = url::Url::parse(&value).ok().filter(|url| {
        matches!(url.scheme(), "postgres" | "postgresql") && url.host_str().is_some()
    });
    let Some(endpoint) = endpoint else {
        checks.push(DoctorCheck::finding(
            code,
            CheckCategory::Environment,
            CheckStatus::Warning,
            None,
            format!("{variable} is not a valid PostgreSQL URL; its value is omitted."),
            Some("Use a `postgres://` or `postgresql://` URL with a host.".to_owned()),
        ));
        return;
    };
    let host = endpoint.host_str().unwrap_or_default().to_owned();
    let port = endpoint.port().unwrap_or(5432);
    let attempt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| {
            let result = runtime.block_on(async {
                tokio::time::timeout(
                    timeout,
                    tokio::net::TcpStream::connect((host.as_str(), port)),
                )
                .await
            });
            runtime.shutdown_timeout(Duration::from_millis(100));
            result
        });
    match attempt {
        Ok(Ok(Ok(_stream))) => checks.push(DoctorCheck::pass(
            code,
            CheckCategory::ExternalDependency,
            None,
            format!("{variable} endpoint accepted a TCP connection; authentication and database health were not checked."),
        )),
        Ok(Err(_)) => checks.push(DoctorCheck::finding(
            code,
            CheckCategory::ExternalDependency,
            CheckStatus::Warning,
            None,
            format!("{variable} endpoint did not respond within {} ms; its value is omitted.", timeout.as_millis()),
            Some("Check that the database host is reachable and listening, then rerun the probe.".to_owned()),
        )),
        Ok(Ok(Err(_))) => checks.push(DoctorCheck::finding(
            code,
            CheckCategory::ExternalDependency,
            CheckStatus::Warning,
            None,
            format!("{variable} endpoint refused or failed the TCP connection; its value is omitted."),
            Some("Check the database host, port, firewall, and service state.".to_owned()),
        )),
        Err(_) => checks.push(DoctorCheck::finding(
            code,
            CheckCategory::Environment,
            CheckStatus::Warning,
            None,
            "Forge could not initialize the bounded network probe.".to_owned(),
            Some("Retry in an environment with Tokio network and timer support.".to_owned()),
        )),
    }
}

fn check_file(
    root: &Path,
    checks: &mut Vec<DoctorCheck>,
    code: &'static str,
    path: &'static str,
    success_message: &'static str,
) {
    if root.join(path).is_file() {
        checks.push(DoctorCheck::pass(
            code,
            CheckCategory::Project,
            Some(path),
            success_message,
        ));
    } else {
        checks.push(DoctorCheck::finding(
            code,
            CheckCategory::Project,
            CheckStatus::Error,
            Some(path),
            format!("Required project file `{path}` is missing."),
            Some(
                "Restore the generated file or regenerate the app after reviewing local changes."
                    .to_owned(),
            ),
        ));
    }
}

fn check_environment_variable(
    checks: &mut Vec<DoctorCheck>,
    code: &'static str,
    name: &'static str,
) {
    if std::env::var(name).is_ok_and(|value| !value.trim().is_empty()) {
        checks.push(DoctorCheck::pass(
            code,
            CheckCategory::Environment,
            None,
            format!("{name} is set; its value is not included in this report."),
        ));
    } else {
        checks.push(DoctorCheck::finding(
            code,
            CheckCategory::Environment,
            CheckStatus::Warning,
            None,
            format!("{name} is not set in the current process environment."),
            Some(format!("Set {name} in the environment that launches the app when the database-backed commands are needed.")),
        ));
    }
}
