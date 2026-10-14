//! Exercises the `forge` executable as a subprocess.

use std::path::Path;
use std::process::{Command, Output};

fn forge(current_dir: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(arguments)
        .current_dir(current_dir)
        .output()
        .expect("forge executable should start")
}

#[test]
fn new_generates_a_dockerized_application() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");

    let output = forge(
        directory.path(),
        &["new", "shop", "--skip-lockfile", "--forge-path", "../forge"],
    );

    assert!(output.status.success(), "{output:?}");
    let root = directory.path().join("shop");
    for path in [
        "Dockerfile",
        ".dockerignore",
        "compose.yaml",
        ".gitattributes",
        "build.rs",
        "migrations/.gitkeep",
        ".github/workflows/ci.yml",
    ] {
        assert!(root.join(path).is_file(), "{path} should be generated");
    }
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("manifest");
    assert!(manifest.contains("sqlx"));
    let compose = std::fs::read_to_string(root.join("compose.yaml")).expect("compose");
    assert!(compose.contains("postgres:18.6-trixie@sha256:"));
    assert!(compose.contains("command: [\"migrate\"]"));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Created Forge application `shop`"));
}

#[test]
fn new_refuses_existing_destination_and_invalid_names() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    std::fs::create_dir(directory.path().join("shop")).expect("fixture directory");

    let existing = forge(directory.path(), &["new", "shop", "--skip-lockfile"]);
    let traversal = forge(directory.path(), &["new", "../escape", "--skip-lockfile"]);

    assert!(!existing.status.success());
    assert!(String::from_utf8_lossy(&existing.stderr).contains("refusing to overwrite"));
    assert!(!traversal.status.success());
    assert!(!directory.path().join("../escape").exists());
}

#[test]
fn skip_docker_omits_container_assets() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");

    let output = forge(
        directory.path(),
        &["new", "api", "--skip-docker", "--skip-lockfile"],
    );

    assert!(output.status.success(), "{output:?}");
    assert!(!directory.path().join("api/Dockerfile").exists());
    assert!(directory.path().join("api/Cargo.toml").is_file());
}

#[test]
fn skip_database_omits_postgres_and_migration_assets() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");

    let output = forge(
        directory.path(),
        &["new", "api", "--skip-database", "--skip-lockfile"],
    );

    assert!(output.status.success(), "{output:?}");
    let root = directory.path().join("api");
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("manifest");
    let compose = std::fs::read_to_string(root.join("compose.yaml")).expect("compose");
    assert!(!manifest.contains("sqlx"));
    assert!(!compose.contains("postgres:"));
    assert!(!root.join("migrations").exists());
    assert!(!root.join("build.rs").exists());
}

#[test]
fn generate_migration_creates_reversible_pair() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");

    let output = forge(
        directory.path(),
        &["generate", "migration", "Create Customers"],
    );

    assert!(output.status.success(), "{output:?}");
    let migrations = std::fs::read_dir(directory.path().join("migrations"))
        .expect("migrations directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();

    assert_eq!(migrations.len(), 2);
    assert!(
        migrations
            .iter()
            .any(|name| name.ends_with("_create_customers.up.sql"))
    );
    assert!(
        migrations
            .iter()
            .any(|name| name.ends_with("_create_customers.down.sql"))
    );
}

#[test]
fn migration_dry_run_is_json_and_does_not_create_files_or_directories() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let output = forge(
        directory.path(),
        &["generate", "migration", "Create Customers", "--dry-run"],
    );
    assert!(output.status.success(), "{output:?}");
    let preview: serde_json::Value = serde_json::from_slice(&output.stdout).expect("preview JSON");
    assert_eq!(preview["schema_version"], 1);
    assert_eq!(preview["dry_run"], true);
    assert_eq!(preview["paths_are_provisional"], true);
    assert_eq!(preview["files"].as_array().expect("files").len(), 2);
    assert!(!directory.path().join("migrations").exists());
}

#[test]
fn explicit_migration_version_produces_a_stable_non_provisional_preview() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let args = [
        "generate",
        "migration",
        "Create Customers",
        "--version",
        "20261004010101",
        "--dry-run",
    ];
    let first = forge(directory.path(), &args);
    let second = forge(directory.path(), &args);
    assert!(first.status.success(), "{first:?}");
    assert!(second.status.success(), "{second:?}");
    assert_eq!(first.stdout, second.stdout);
    let preview: serde_json::Value = serde_json::from_slice(&first.stdout).expect("preview JSON");
    assert_eq!(preview["paths_are_provisional"], false);
    assert_eq!(
        preview["files"][0]["path"],
        "migrations/20261004010101_create_customers.up.sql"
    );
}

#[test]
fn generate_resource_creates_tenant_entity_and_rls_migrations_without_overwriting() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let forge_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace crates directory")
        .join("forge")
        .to_string_lossy()
        .into_owned();
    let created = forge(
        directory.path(),
        &[
            "new",
            "catalog",
            "--skip-lockfile",
            "--forge-path",
            &forge_path,
        ],
    );
    assert!(created.status.success(), "{created:?}");

    let app = directory.path().join("catalog");
    let generated = forge(
        &app,
        &[
            "generate",
            "resource",
            "Project",
            "--fields",
            "name:string,owner_id:uuid,active:bool",
        ],
    );
    assert!(generated.status.success(), "{generated:?}");

    let entity = std::fs::read_to_string(app.join("src/domain/entities/project.rs"))
        .expect("generated entity");
    assert!(entity.contains("forge::tenancy::TenantId"));
    assert!(entity.contains("pub struct Project"));
    assert!(entity.contains("owner_id: uuid::Uuid"));
    let modules = std::fs::read_to_string(app.join("src/domain/entities/mod.rs"))
        .expect("entity module list");
    assert!(modules.contains("pub mod project;"));

    let repository = std::fs::read_to_string(app.join("src/application/ports/project.rs"))
        .expect("generated repository port");
    assert!(repository.contains("context: &TenantContext"));
    assert!(repository.contains("fn update("));
    assert!(repository.contains("expected_version: Version"));
    assert!(!repository.contains("tenant_id: forge::tenancy::TenantId"));
    let use_cases = std::fs::read_to_string(app.join("src/application/use_cases/project.rs"))
        .expect("generated use cases");
    assert!(use_cases.contains("pub struct CreateProject"));
    assert!(use_cases.contains("pub struct UpdateProject"));
    assert!(use_cases.contains("context: &TenantContext"));
    assert!(use_cases.contains("resource.version() != command.expected_version"));
    for permission in [
        "project:create",
        "project:read",
        "project:update",
        "project:delete",
        "project:list",
    ] {
        assert!(
            use_cases.contains(permission),
            "missing permission check: {permission}"
        );
    }
    assert!(use_cases.contains("ErrorCategory::Forbidden"));
    let adapter = std::fs::read_to_string(app.join("src/adapters/persistence/project_postgres.rs"))
        .expect("generated PostgreSQL adapter");
    assert!(adapter.contains("begin_tenant_transaction(&self.pool, context)"));
    assert!(adapter.contains("WHERE \\\"id\\\" = $1 AND \\\"tenant_id\\\" = $2"));
    assert!(adapter.contains("\\\"version\\\" = \\\"version\\\" + 1"));
    assert!(adapter.contains("ORDER BY \\\"id\\\" DESC LIMIT $3"));
    assert!(
        std::fs::read_to_string(app.join("src/adapters/persistence/mod.rs"))
            .expect("persistence module list")
            .contains("pub mod project_postgres;")
    );
    assert!(
        std::fs::read_to_string(app.join("src/application/ports/mod.rs"))
            .expect("repository module list")
            .contains("pub mod project;")
    );
    assert!(
        std::fs::read_to_string(app.join("src/application/use_cases/mod.rs"))
            .expect("use-case module list")
            .contains("pub mod project;")
    );
    let integration_test =
        std::fs::read_to_string(app.join("tests/resource_project.rs")).expect("generated test");
    assert!(integration_test.contains("requires migrated PostgreSQL"));
    assert!(integration_test.contains("another_tenant"));
    assert!(integration_test.contains("permission_denied"));

    let migrations = std::fs::read_dir(app.join("migrations"))
        .expect("migrations directory")
        .map(|entry| entry.expect("migration entry").path())
        .collect::<Vec<_>>();
    let up = migrations
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("_projects.up.sql"))
        })
        .expect("up migration");
    let up_sql = std::fs::read_to_string(up).expect("up migration contents");
    assert!(up_sql.contains("ENABLE ROW LEVEL SECURITY"));
    assert!(up_sql.contains("WITH CHECK"));

    let enum_generated = forge(
        &app,
        &[
            "generate",
            "resource",
            "StatusReport",
            "--fields",
            "state:enum[draft|published]",
        ],
    );
    assert!(enum_generated.status.success(), "{enum_generated:?}");
    let enum_migration = std::fs::read_dir(app.join("migrations"))
        .expect("migrations directory")
        .map(|entry| entry.expect("migration entry").path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("_status_reports.up.sql"))
        })
        .expect("enum migration");
    let enum_sql = std::fs::read_to_string(enum_migration).expect("enum migration contents");
    assert!(enum_sql.contains("CHECK (\"state\" IN ('draft', 'published'))"));

    let injection = forge(
        &app,
        &[
            "generate",
            "resource",
            "UnsafeReport",
            "--fields",
            "state:enum[draft');DROP|published]",
        ],
    );
    assert!(!injection.status.success());
    assert!(!app.join("src/domain/entities/unsafe_report.rs").exists());

    let repeated = forge(
        &app,
        &["generate", "resource", "Project", "--fields", "name:string"],
    );
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("already"));
}

#[test]
fn resource_dry_run_previews_all_artifacts_without_mutating_the_app() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let forge_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace crates directory")
        .join("forge")
        .to_string_lossy()
        .into_owned();
    let created = forge(
        directory.path(),
        &[
            "new",
            "catalog",
            "--skip-lockfile",
            "--forge-path",
            &forge_path,
        ],
    );
    assert!(created.status.success(), "{created:?}");
    let app = directory.path().join("catalog");
    let before_modules =
        std::fs::read(app.join("src/domain/entities/mod.rs")).expect("module list");
    let before_migrations = std::fs::read_dir(app.join("migrations"))
        .expect("migrations")
        .map(|entry| entry.expect("entry").file_name())
        .collect::<Vec<_>>();
    let output = forge(
        &app,
        &[
            "generate",
            "resource",
            "Project",
            "--fields",
            "name:string",
            "--version",
            "20261004010202",
            "--dry-run",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    let preview: serde_json::Value = serde_json::from_slice(&output.stdout).expect("preview JSON");
    let repeated = forge(
        &app,
        &[
            "generate",
            "resource",
            "Project",
            "--fields",
            "name:string",
            "--version",
            "20261004010202",
            "--dry-run",
        ],
    );
    assert!(repeated.status.success(), "{repeated:?}");
    assert_eq!(output.stdout, repeated.stdout);
    assert_eq!(preview["paths_are_provisional"], false);
    assert_eq!(preview["schema_version"], 1);
    assert_eq!(preview["files"].as_array().expect("files").len(), 11);
    assert_eq!(
        preview["files"][0]["path"],
        "src/domain/entities/project.rs"
    );
    assert_eq!(preview["files"][3]["operation"], "append");
    assert_eq!(
        preview["files"][6]["path"],
        "src/adapters/persistence/mod.rs"
    );
    assert_eq!(
        preview["files"][7]["path"],
        "src/application/ports/project.rs"
    );
    assert_eq!(
        preview["files"][8]["path"],
        "src/application/use_cases/project.rs"
    );
    assert_eq!(
        preview["files"][9]["path"],
        "src/adapters/persistence/project_postgres.rs"
    );
    assert_eq!(preview["files"][10]["path"], "tests/resource_project.rs");
    assert!(!app.join("src/domain/entities/project.rs").exists());
    assert!(!app.join("src/application/ports/project.rs").exists());
    assert!(!app.join("src/application/use_cases/project.rs").exists());
    assert!(
        !app.join("src/adapters/persistence/project_postgres.rs")
            .exists()
    );
    assert!(!app.join("tests/resource_project.rs").exists());
    assert_eq!(
        std::fs::read(app.join("src/domain/entities/mod.rs")).expect("module list"),
        before_modules
    );
    let after_migrations = std::fs::read_dir(app.join("migrations"))
        .expect("migrations")
        .map(|entry| entry.expect("entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(after_migrations, before_migrations);
}

#[test]
fn generate_resource_rejects_path_traversal_before_writing() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let output = forge(
        directory.path(),
        &[
            "generate",
            "resource",
            "../Escape",
            "--fields",
            "name:string",
        ],
    );
    assert!(!output.status.success());
    assert!(!directory.path().join("Escape.rs").exists());
}

#[test]
fn id_prints_a_uuid_version_seven() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");

    let output = forge(directory.path(), &["id"]);

    assert!(output.status.success());
    let id = String::from_utf8_lossy(&output.stdout);
    let id = id.trim();
    assert_eq!(id.len(), 36);
    assert_eq!(id.as_bytes()[14], b'7', "version nibble of {id}");
}

#[test]
fn generated_capability_manifest_matches_the_selected_scaffold_and_is_queryable() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let created = forge(
        directory.path(),
        &[
            "new",
            "agent-ready",
            "--skip-lockfile",
            "--skip-ci",
            "--forge-path",
            "../forge",
        ],
    );
    assert!(created.status.success(), "{created:?}");
    let app = directory.path().join("agent-ready");
    let manifest_path = app.join(".forge/capabilities.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("capability manifest"))
            .expect("manifest should be JSON");
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["application"], "agent-ready");
    assert_eq!(manifest["features"]["database"], "postgresql");
    assert_eq!(manifest["features"]["authentication"], true);
    assert_eq!(manifest["features"]["ci"], false);
    assert_eq!(manifest["features"]["ai_providers_and_execution"], false);
    assert_eq!(manifest["generators"][1]["status"], "partial");
    assert_eq!(manifest["generators"][2]["status"], "available");
    let agent_entrypoint = std::fs::read_to_string(app.join("AGENTS.md")).expect("agent guide");
    let detailed_guide =
        std::fs::read_to_string(app.join("docs/for-ai.md")).expect("detailed AI guide");
    assert!(agent_entrypoint.contains("Start with `docs/for-ai.md`"));
    assert!(detailed_guide.contains("Database adapter: `postgresql`"));
    assert!(detailed_guide.contains("Jobs/events: false"));
    assert!(detailed_guide.contains("AI providers or tool execution: false"));
    assert_ne!(agent_entrypoint, detailed_guide);

    let output = forge(&app, &["capabilities", "--format", "json"]);
    assert!(output.status.success(), "{output:?}");
    let queried: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("CLI output should be JSON");
    assert_eq!(queried, manifest);
}

#[test]
fn database_free_capability_manifest_does_not_claim_database_features() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let created = forge(
        directory.path(),
        &[
            "new",
            "static-site",
            "--skip-database",
            "--skip-docker",
            "--skip-ci",
            "--skip-lockfile",
            "--forge-path",
            "../forge",
        ],
    );
    assert!(created.status.success(), "{created:?}");
    let app = directory.path().join("static-site");
    let output = forge(&app, &["capabilities"]);
    assert!(output.status.success(), "{output:?}");
    let manifest: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("CLI output should be JSON");
    assert!(manifest["features"]["database"].is_null());
    assert_eq!(manifest["features"]["authentication"], false);
    assert_eq!(manifest["features"]["tenancy"], false);
    assert_eq!(manifest["features"]["docker"], false);
    assert_eq!(manifest["generators"][0]["status"], "available");
    assert_eq!(manifest["generators"][1]["status"], "not_available");
    assert_eq!(manifest["generators"][2]["status"], "available");
    let detailed_guide =
        std::fs::read_to_string(app.join("docs/for-ai.md")).expect("detailed AI guide");
    assert!(detailed_guide.contains("Database adapter: `disabled`"));
    assert!(detailed_guide.contains("This scaffold has no database adapter or migrations."));
    assert!(detailed_guide.contains("Docker assets were not requested"));
}

#[test]
fn generate_page_preview_is_non_mutating_and_generation_registers_an_escaped_route() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let created = forge(
        directory.path(),
        &[
            "new",
            "page-app",
            "--skip-database",
            "--skip-docker",
            "--skip-ci",
            "--skip-lockfile",
            "--forge-path",
            "../forge",
        ],
    );
    assert!(created.status.success(), "{created:?}");
    let app = directory.path().join("page-app");

    let preview = forge(&app, &["generate", "page", "UserProfile", "--dry-run"]);
    assert!(preview.status.success(), "{preview:?}");
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("preview is JSON");
    assert_eq!(preview["schema_version"], 1);
    assert_eq!(preview["paths_are_provisional"], false);
    assert_eq!(preview["summary"]["creates"], 1);
    assert_eq!(preview["summary"]["appends"], 2);
    assert_eq!(
        preview["files"][0]["path"],
        "src/adapters/views/user_profile.rs"
    );
    assert!(!app.join("src/adapters/views/user_profile.rs").exists());

    let generated = forge(&app, &["generate", "page", "UserProfile"]);
    assert!(generated.status.success(), "{generated:?}");
    assert!(String::from_utf8_lossy(&generated.stdout).contains("/user-profile"));
    let page = std::fs::read_to_string(app.join("src/adapters/views/user_profile.rs"))
        .expect("generated page");
    assert!(page.contains("forge::web::escape_html(title)"));
    assert!(page.contains("forge::web::escape_html(content)"));
    assert!(
        std::fs::read_to_string(app.join("src/adapters/views/routes.rs"))
            .expect("route registrations")
            .contains("crate::adapters::views::user_profile::register(router)?;")
    );

    let repeated = forge(&app, &["generate", "page", "UserProfile"]);
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("already exists"));
}

#[test]
fn capabilities_command_fails_clearly_outside_a_generated_app() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let output = forge(directory.path(), &["capabilities", "--format", "json"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no Forge capability manifest"));
}

#[test]
fn doctor_returns_versioned_static_json_without_running_checks_or_exposing_secrets() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let created = forge(
        directory.path(),
        &[
            "new",
            "doctor-sample",
            "--skip-database",
            "--skip-docker",
            "--skip-ci",
            "--skip-lockfile",
            "--forge-path",
            "../forge",
        ],
    );
    assert!(created.status.success(), "{created:?}");

    let app = directory.path().join("doctor-sample");
    let output = forge(&app, &["doctor", "--format", "json"]);
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be JSON");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["status"], "ready");
    assert_eq!(
        report["scope"],
        "static local checks only; no Cargo command, build, test, network request, or database connection was executed"
    );
    assert!(
        report["checks"]
            .as_array()
            .is_some_and(|checks| !checks.is_empty())
    );
    assert!(report["checks"].as_array().is_some_and(|checks| {
        checks
            .iter()
            .any(|check| check["code"] == "cargo_lock_not_required_by_scaffold")
    }));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("postgres://"));
}

#[test]
fn doctor_distinguishes_a_non_forge_directory_from_project_findings() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let output = forge(directory.path(), &["doctor"]);

    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be JSON");
    assert_eq!(report["status"], "not_a_forge_app");
    assert_eq!(report["checks"][0]["code"], "capability_manifest_missing");
    assert_eq!(report["checks"][0]["category"], "project");
    assert_eq!(report["checks"][0]["status"], "error");

    let requested_probe = forge(directory.path(), &["doctor", "--probe-database"]);
    let report: serde_json::Value =
        serde_json::from_slice(&requested_probe.stdout).expect("doctor report");
    assert!(
        report["scope"]
            .as_str()
            .unwrap_or_default()
            .contains("static local checks only")
    );
}

#[test]
fn doctor_reports_database_connectivity_as_unprobed_not_as_a_code_failure() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let created = forge(
        directory.path(),
        &[
            "new",
            "db-app",
            "--skip-docker",
            "--skip-ci",
            "--skip-lockfile",
            "--forge-path",
            "../forge",
        ],
    );
    assert!(created.status.success(), "{created:?}");

    let output = forge(&directory.path().join("db-app"), &["doctor"]);
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be JSON");
    let connectivity = report["checks"]
        .as_array()
        .and_then(|checks| {
            checks
                .iter()
                .find(|check| check["code"] == "database_connectivity_not_probed")
        })
        .expect("database connectivity should be explicitly reported");
    assert_eq!(connectivity["category"], "external_dependency");
    assert_eq!(connectivity["status"], "not_checked");
    assert_eq!(report["status"], "attention");
}

#[test]
fn doctor_database_probe_is_opt_in_bounded_tcp_only_and_redacts_urls() {
    use std::net::TcpListener;

    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let created = forge(
        directory.path(),
        &[
            "new",
            "db-probe",
            "--skip-docker",
            "--skip-ci",
            "--skip-lockfile",
            "--forge-path",
            "../forge",
        ],
    );
    assert!(created.status.success(), "{created:?}");
    let listener = TcpListener::bind("127.0.0.1:0").expect("local test listener");
    let port = listener.local_addr().expect("listener address").port();
    let app = directory.path().join("db-probe");
    let output = Command::new(env!("CARGO_BIN_EXE_forge"))
        .args(["doctor", "--probe-database", "--timeout-ms", "500"])
        .current_dir(app)
        .env(
            "FORGE_DATABASE_URL",
            format!("postgres://user:secret-password@127.0.0.1:{port}/app"),
        )
        .env_remove("FORGE_MIGRATION_DATABASE_URL")
        .output()
        .expect("doctor process should start");
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("doctor report");
    let checks = report["checks"].as_array().expect("checks");
    assert!(checks.iter().any(|check| {
        check["code"] == "runtime_database_tcp_reachable" && check["status"] == "pass"
    }));
    assert!(checks.iter().any(|check| {
        check["code"] == "migration_database_tcp_reachable" && check["status"] == "not_applicable"
    }));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-password"));
    assert!(
        report["scope"]
            .as_str()
            .unwrap_or_default()
            .contains("no Cargo command")
    );

    let invalid_timeout = forge(
        &directory.path().join("db-probe"),
        &["doctor", "--probe-database", "--timeout-ms", "20000"],
    );
    assert_eq!(invalid_timeout.status.code(), Some(2));
}

#[test]
fn doctor_rejects_a_well_formed_but_unsupported_capability_schema() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let metadata = directory.path().join(".forge");
    std::fs::create_dir(&metadata).expect("metadata directory");
    std::fs::write(
        metadata.join("capabilities.json"),
        r#"{"schema_version":99,"forge_version":"0","application":"x","features":{"database":null,"authentication":false,"tenancy":false,"audit":false,"docker":false,"ci":false,"jobs_and_events":false,"ai_providers_and_execution":false},"generators":[]}"#,
    )
    .expect("future schema fixture");

    let output = forge(directory.path(), &["doctor", "--format", "json"]);
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor output should be JSON");
    assert_eq!(report["status"], "attention");
    assert_eq!(report["checks"][0]["code"], "capability_schema_unsupported");

    let capabilities = forge(directory.path(), &["capabilities", "--format", "json"]);
    assert!(!capabilities.status.success());
    assert!(
        String::from_utf8_lossy(&capabilities.stderr)
            .contains("unsupported Forge capability schema version 99")
    );
}

#[test]
fn json_errors_have_stable_codes_hints_paths_and_exit_status() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let output = forge(
        directory.path(),
        &["generate", "migration", "../unsafe", "--json-errors"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let diagnostic: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("error output should be JSON");
    assert_eq!(diagnostic["schema_version"], 1);
    assert_eq!(diagnostic["code"], "invalid_migration_name");
    assert!(diagnostic["message"].as_str().is_some());
    assert!(diagnostic["hint"].as_str().is_some());
    assert!(diagnostic["path"].is_null());
}

#[test]
fn json_errors_cover_argument_parser_failures_without_echoing_invalid_values() {
    let directory = tempfile::tempdir().expect("temporary directory should be available");
    let output = forge(
        directory.path(),
        &[
            "generate",
            "resource",
            "Project",
            "--json-errors",
            "--fields",
            "name:secret-value",
            "--unexpected",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let diagnostic: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("parse JSON diagnostic");
    assert_eq!(diagnostic["schema_version"], 1);
    assert_eq!(diagnostic["code"], "cli_argument_error");
    assert_eq!(diagnostic["path"], serde_json::Value::Null);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-value"));

    let help = forge(directory.path(), &["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage:"));
}
