use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::CliError;

/// Toolchain pinned by generated applications and their builder image.
/// Kept equal to the framework's `rust-toolchain.toml` by a unit test.
pub(crate) const RUST_VERSION: &str = "1.98.1";

const FORGE_REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const FORGE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where a generated application obtains the `forge` crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForgeSource {
    /// The release tag matching this CLI's version.
    GitTag,
    /// A local checkout, for framework development (like `rails new --dev`).
    Path(String),
}

/// Options accepted by `forge new`.
#[derive(Debug, Clone)]
pub(crate) struct NewApplication {
    pub(crate) name: String,
    pub(crate) docker: bool,
    pub(crate) ci: bool,
    pub(crate) database: bool,
    pub(crate) forge: ForgeSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Always,
    Docker,
    Ci,
    Database,
}

struct Template {
    path: &'static str,
    content: &'static str,
    group: Group,
}

macro_rules! template {
    ($group:expr, $path:literal => $source:literal) => {
        Template {
            path: $path,
            content: include_str!(concat!("../templates/app/", $source)),
            group: $group,
        }
    };
}

const TEMPLATES: &[Template] = &[
    template!(Group::Always, "Cargo.toml" => "Cargo.toml.tmpl"),
    template!(Group::Always, "rust-toolchain.toml" => "rust-toolchain.toml.tmpl"),
    template!(Group::Always, ".gitignore" => "gitignore.tmpl"),
    template!(Group::Always, "README.md" => "README.md.tmpl"),
    template!(Group::Always, "src/main.rs" => "src/main.rs.tmpl"),
    template!(Group::Always, "src/lib.rs" => "src/lib.rs.tmpl"),
    template!(Group::Always, "src/domain/mod.rs" => "src/domain/mod.rs.tmpl"),
    template!(Group::Always, "src/application/mod.rs" => "src/application/mod.rs.tmpl"),
    template!(Group::Always, "src/adapters/mod.rs" => "src/adapters/mod.rs.tmpl"),
    template!(Group::Always, "src/adapters/http/mod.rs" => "src/adapters/http/mod.rs.tmpl"),
    template!(Group::Always, "src/infrastructure/mod.rs" => "src/infrastructure/mod.rs.tmpl"),
    template!(Group::Always, "src/bootstrap/mod.rs" => "src/bootstrap/mod.rs.tmpl"),
    template!(Group::Always, "tests/architecture.rs" => "tests/architecture.rs.tmpl"),
    template!(Group::Database, "build.rs" => "build.rs.tmpl"),
    template!(Group::Database, ".gitattributes" => "gitattributes.tmpl"),
    template!(Group::Database, "migrations/.gitkeep" => "migrations/gitkeep.tmpl"),
    template!(Group::Docker, "Dockerfile" => "Dockerfile.tmpl"),
    template!(Group::Docker, ".dockerignore" => "dockerignore.tmpl"),
    template!(Group::Docker, "compose.yaml" => "compose.yaml.tmpl"),
    template!(Group::Docker, "tests/container.rs" => "tests/container.rs.tmpl"),
    template!(Group::Ci, ".github/workflows/ci.yml" => "github/workflows/ci.yml.tmpl"),
    template!(Group::Ci, ".github/dependabot.yml" => "github/dependabot.yml.tmpl"),
];

/// Modules that exist to make each ring's responsibilities discoverable.
/// Each is a documented, empty module until a feature needs it.
const RESERVED_MODULES: &[(&str, &str)] = &[
    (
        "src/domain/entities",
        "Entities: domain objects with identity and invariants.",
    ),
    (
        "src/domain/value_objects",
        "Value objects: immutable, self-validating domain values.",
    ),
    (
        "src/domain/services",
        "Domain services: business rules spanning several entities.",
    ),
    (
        "src/domain/events",
        "Domain events: facts produced by domain behavior.",
    ),
    (
        "src/domain/errors",
        "Domain errors: typed business rule violations.",
    ),
    (
        "src/application/commands",
        "Commands: validated intents that change state.",
    ),
    (
        "src/application/queries",
        "Queries: read models that never change state.",
    ),
    (
        "src/application/use_cases",
        "Use cases: orchestration and transaction boundaries.",
    ),
    (
        "src/application/ports",
        "Ports: traits implemented by adapters and infrastructure.",
    ),
    (
        "src/adapters/persistence",
        "Persistence adapters implementing repository ports.",
    ),
    (
        "src/adapters/messaging",
        "Messaging adapters for integration events.",
    ),
    ("src/adapters/storage", "Object storage adapters."),
    ("src/adapters/email", "Email delivery adapters."),
    (
        "src/adapters/ai",
        "AI provider adapters implementing the application's AI ports.",
    ),
    (
        "src/infrastructure/database",
        "Database pools, migrations and transactions.",
    ),
    ("src/infrastructure/cache", "Cache clients."),
    ("src/infrastructure/queue", "Job queue infrastructure."),
    (
        "src/infrastructure/telemetry",
        "Telemetry exporters and instrumentation.",
    ),
    (
        "src/infrastructure/security",
        "Secrets, keys and security mechanisms.",
    ),
];

pub(crate) fn create_migration(
    root: &Path,
    name: &str,
) -> Result<(PathBuf, PathBuf), CliError> {
    let normalized = normalize_migration_name(name)?;
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(CliError::Clock)?;
    let version = elapsed.as_nanos();
    let migrations = root.join("migrations");
    fs::create_dir_all(&migrations).map_err(|source| CliError::CreateDirectory {
        path: migrations.clone(),
        source,
    })?;

    let stem = format!("{version}_{normalized}");
    let up = migrations.join(format!("{stem}.up.sql"));
    let down = migrations.join(format!("{stem}.down.sql"));

    write_new_file(
        &up,
        &format!("-- {normalized}: apply migration\n\n"),
    )?;
    if let Err(error) = write_new_file(
        &down,
        &format!("-- {normalized}: revert migration\n\n"),
    ) {
        let _ = fs::remove_file(&up);
        return Err(error);
    }

    Ok((up, down))
}

fn normalize_migration_name(name: &str) -> Result<String, CliError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CliError::InvalidMigrationName {
            name: name.to_owned(),
            reason: "the name cannot be empty",
        });
    }
    if name.len() > 80 || !name.is_ascii() {
        return Err(CliError::InvalidMigrationName {
            name: name.to_owned(),
            reason: "use at most 80 ASCII characters",
        });
    }

    let mut normalized = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            'a'..='z' | '0'..='9' | '_' => normalized.push(character),
            'A'..='Z' => normalized.push(character.to_ascii_lowercase()),
            '-' | ' ' => normalized.push('_'),
            _ => {
                return Err(CliError::InvalidMigrationName {
                    name: name.to_owned(),
                    reason: "use letters, numbers, spaces, '-' or '_' only",
                });
            }
        }
    }

    while normalized.contains("__") {
        normalized = normalized.replace("__", "_");
    }
    let normalized = normalized.trim_matches('_').to_owned();
    if normalized.is_empty()
        || !normalized
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
    {
        return Err(CliError::InvalidMigrationName {
            name: name.to_owned(),
            reason: "the normalized name must start with a letter",
        });
    }
    Ok(normalized)
}

pub(crate) fn create_application(
    parent: &Path,
    options: &NewApplication,
) -> Result<PathBuf, CliError> {
    validate_application_name(&options.name)?;
    let forge_dependency = forge_dependency(&options.forge)?;

    let destination = parent.join(&options.name);
    match fs::create_dir(&destination) {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(CliError::DestinationExists(destination));
        }
        Err(source) => {
            return Err(CliError::CreateDirectory {
                path: destination,
                source,
            });
        }
    }

    let result = write_application(&destination, options, &forge_dependency);

    if result.is_err() {
        let _cleanup_result = fs::remove_dir_all(&destination);
    }

    result.map(|()| destination)
}

fn validate_application_name(name: &str) -> Result<(), CliError> {
    if name.is_empty() {
        return invalid_name(name, "the name cannot be empty");
    }
    if name.len() > 64 {
        return invalid_name(name, "use at most 64 ASCII characters");
    }
    if !name.is_ascii() {
        return invalid_name(name, "use ASCII lowercase letters, numbers, `_`, or `-`");
    }
    if !name.as_bytes()[0].is_ascii_lowercase() {
        return invalid_name(
            name,
            "the first character must be an ASCII lowercase letter",
        );
    }
    if !name.bytes().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, b'_' | b'-')
    }) {
        return invalid_name(
            name,
            "use only ASCII lowercase letters, numbers, `_`, or `-`",
        );
    }
    if name.ends_with('-') || name.ends_with('_') {
        return invalid_name(name, "the last character must be a letter or number");
    }
    if is_reserved_name(name) {
        return invalid_name(name, "this package name is reserved by Rust or Cargo");
    }
    Ok(())
}

fn invalid_name(name: &str, reason: &'static str) -> Result<(), CliError> {
    Err(CliError::InvalidApplicationName {
        name: name.to_owned(),
        reason,
    })
}

fn is_reserved_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do",
        "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
        "std", "core", "alloc", "test", "app", "forge", "con", "prn", "aux", "nul", "com1", "com2",
        "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4",
        "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    RESERVED.contains(&name)
}

/// Renders the `forge` dependency as a TOML inline table.
fn forge_dependency(source: &ForgeSource) -> Result<String, CliError> {
    match source {
        ForgeSource::GitTag => Ok(format!(
            "{{ git = \"{FORGE_REPOSITORY}\", tag = \"v{FORGE_VERSION}\" }}"
        )),
        ForgeSource::Path(path) => {
            let rejected = path.is_empty()
                || path.chars().any(|character| {
                    character == '"' || character == '\\' || character.is_control()
                });
            if rejected {
                return Err(CliError::InvalidForgePath {
                    path: path.clone(),
                    reason: "use a non-empty path with forward slashes and no quotes or control characters",
                });
            }
            Ok(format!("{{ path = \"{path}\" }}"))
        }
    }
}

fn write_application(
    root: &Path,
    options: &NewApplication,
    forge_dependency: &str,
) -> Result<(), CliError> {
    let database_name = options.name.replace('-', "_");
    let database_url_local = format!(
        "postgres://app:app@127.0.0.1:5432/{database_name}"
    );
    let database_url_container =
        format!("postgres://app:app@db:5432/{database_name}");
    let variables = [
        ("%%APP_NAME%%", options.name.as_str()),
        ("%%RUST_VERSION%%", RUST_VERSION),
        ("%%FORGE_DEPENDENCY%%", forge_dependency),
        ("%%DATABASE_NAME%%", database_name.as_str()),
        ("%%DATABASE_DRIVER%%", "postgres"),
        ("%%DATABASE_URL_LOCAL%%", database_url_local.as_str()),
        ("%%DATABASE_URL_CONTAINER%%", database_url_container.as_str()),
    ];

    for template in TEMPLATES {
        let enabled = match template.group {
            Group::Always => true,
            Group::Docker => options.docker,
            Group::Ci => options.ci,
            Group::Database => options.database,
        };
        if enabled {
            let content = render(template.content, &variables, options.docker, options.database);
            write_file(&root.join(template.path), &content)?;
        }
    }

    for (directory, description) in RESERVED_MODULES {
        write_file(
            &root.join(directory).join("mod.rs"),
            &format!("//! {description}\n"),
        )?;
    }

    Ok(())
}

/// Substitutes `%%NAME%%` variables and evaluates template conditionals.
///
/// Phase 1 only enables the `docker` capability. The database condition names
/// are intentionally understood already because Phase 2 templates landed ahead
/// of their public CLI switch; until that vertical slice is complete they render
/// as disabled rather than leaking `%%if ...%%` markers into generated apps.
fn render(
    template: &str,
    variables: &[(&str, &str)],
    docker: bool,
    database: bool,
) -> String {
    let condition = |name: &str| match name {
        "docker" => docker,
        "database" | "postgresql" => database,
        "mysql" | "sqlite" => false,
        _ => false,
    };

    let mut output = String::with_capacity(template.len());
    let mut stack = vec![true];

    for line in template.split_inclusive('\n') {
        let marker = line.trim();
        let directive = match marker {
            "%%IF_DOCKER%%" => Some(("docker", false)),
            "%%END_IF_DOCKER%%" => {
                if stack.len() > 1 {
                    stack.pop();
                }
                continue;
            }
            _ if marker.starts_with("%%if ") && marker.ends_with("%%") => {
                let name = &marker[5..marker.len() - 2];
                Some((name.strip_prefix('!').unwrap_or(name), name.starts_with('!')))
            }
            "%%end%%" => {
                if stack.len() > 1 {
                    stack.pop();
                }
                continue;
            }
            _ => None,
        };

        if let Some((name, negated)) = directive {
            let enabled = condition(name);
            let enabled = if negated { !enabled } else { enabled };
            let parent = *stack.last().unwrap_or(&true);
            stack.push(parent && enabled);
            continue;
        }

        if *stack.last().unwrap_or(&true) {
            output.push_str(line);
        }
    }

    for (name, value) in variables {
        output = output.replace(name, value);
    }
    output
}

fn write_new_file(path: &Path, content: &str) -> Result<(), CliError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                CliError::DestinationExists(path.to_path_buf())
            } else {
                CliError::WriteFile {
                    path: path.to_path_buf(),
                    source,
                }
            }
        })?;
    file.write_all(content.as_bytes())
        .map_err(|source| CliError::WriteFile {
            path: path.to_path_buf(),
            source,
        })
}

fn write_file(path: &Path, content: &str) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| CliError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, content).map_err(|source| CliError::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn options(name: &str) -> NewApplication {
        NewApplication {
            name: name.to_owned(),
            docker: true,
            ci: true,
            database: true,
            forge: ForgeSource::GitTag,
        }
    }

    fn read(root: &Path, relative: &str) -> String {
        fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("{relative} should be readable: {error}"))
    }

    /// Reads every file below `root` except build outputs and the lockfile.
    fn snapshot(root: &Path) -> BTreeMap<String, String> {
        fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, String>) {
            let entries = fs::read_dir(directory).expect("directory should be readable");
            for entry in entries {
                let path = entry.expect("entry should be readable").path();
                let relative = path
                    .strip_prefix(root)
                    .expect("path is below root")
                    .to_string_lossy()
                    .replace('\\', "/");
                if relative == "target" || relative == "Cargo.lock" {
                    continue;
                }
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    let content = fs::read_to_string(&path).expect("file should be UTF-8");
                    files.insert(relative, content);
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    #[test]
    fn creates_reversible_migration_files() {
        let root = tempfile::tempdir().expect("temporary directory should be available");
        let (up, down) =
            create_migration(root.path(), "Create Users").expect("migration should be created");

        assert!(up.is_file());
        assert!(down.is_file());
        assert!(up.file_name().unwrap().to_string_lossy().ends_with("_create_users.up.sql"));
        assert!(
            down.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("_create_users.down.sql")
        );
        assert!(read(root.path(), up.strip_prefix(root.path()).unwrap().to_str().unwrap())
            .contains("apply migration"));
        assert!(read(root.path(), down.strip_prefix(root.path()).unwrap().to_str().unwrap())
            .contains("revert migration"));
    }

    #[test]
    fn rejects_unsafe_migration_names() {
        for name in ["", "../users", "1users", "users/roles", "café"] {
            assert!(normalize_migration_name(name).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn accepts_valid_cargo_package_names() {
        for name in ["shop", "shop-api", "shop_api", "shop2"] {
            assert!(validate_application_name(name).is_ok(), "rejected {name}");
        }
    }

    #[test]
    fn rejects_unsafe_or_invalid_names() {
        for name in [
            "", "Shop", "2shop", "../shop", "shop/api", "shop api", "shop-", "fn", "con", "café",
            "app", "forge",
        ] {
            assert!(validate_application_name(name).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn rust_version_matches_the_framework_toolchain() {
        let toolchain = read(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            "../../rust-toolchain.toml",
        );
        assert!(
            toolchain.contains(&format!("channel = \"{RUST_VERSION}\"")),
            "RUST_VERSION must equal the workspace rust-toolchain.toml channel"
        );
    }

    #[test]
    fn creates_a_dockerized_application_by_default() {
        let parent = tempfile::tempdir().expect("temporary directory should be available");

        let root = create_application(parent.path(), &options("sample-app"))
            .expect("application generation should succeed");

        assert_eq!(root, parent.path().join("sample-app"));
        for directory in RESERVED_MODULES.iter().map(|(directory, _)| directory) {
            assert!(root.join(directory).join("mod.rs").is_file(), "{directory}");
        }

        let manifest = read(&root, "Cargo.toml");
        assert!(manifest.contains("name = \"sample-app\""));
        assert!(manifest.contains("sqlx"));
        assert!(root.join("build.rs").is_file());
        assert!(root.join("migrations/.gitkeep").is_file());
        assert!(manifest.contains(&format!(
            "forge = {{ git = \"https://github.com/fabriciobonjorno/forge-rust\", tag = \"v{FORGE_VERSION}\" }}"
        )));

        let dockerfile = read(&root, "Dockerfile");
        assert!(dockerfile.contains(&format!("ARG RUST_VERSION={RUST_VERSION}")));
        assert!(dockerfile.contains("cargo build --release --locked --bin sample-app"));
        assert!(dockerfile.contains("FROM gcr.io/distroless/cc-debian13:nonroot AS runtime"));
        assert!(dockerfile.contains("USER 65532:65532"));
        assert!(dockerfile.contains("CMD [\"/usr/local/bin/sample-app\", \"healthcheck\"]"));
        assert!(dockerfile.contains("ENTRYPOINT [\"/usr/local/bin/sample-app\"]"));
        assert!(!dockerfile.contains("generate-lockfile"));

        let dockerignore = read(&root, ".dockerignore");
        assert!(dockerignore.contains("**/.env"));
        assert!(dockerignore.contains("**/target"));

        assert!(read(&root, "compose.yaml").contains("no-new-privileges:true"));
        assert!(read(&root, ".github/workflows/ci.yml").contains("docker/build-push-action"));
        assert!(read(&root, ".github/dependabot.yml").contains("package-ecosystem: docker"));
        assert!(root.join("tests/container.rs").is_file());
        assert!(root.join("tests/architecture.rs").is_file());
    }

    #[test]
    fn skip_flags_omit_docker_and_ci_assets() {
        let parent = tempfile::tempdir().expect("temporary directory should be available");
        let options = NewApplication {
            docker: false,
            ci: false,
            database: false,
            ..options("plain")
        };

        let root = create_application(parent.path(), &options).expect("generation should succeed");

        for path in [
            "Dockerfile",
            ".dockerignore",
            "compose.yaml",
            "tests/container.rs",
            ".github",
        ] {
            assert!(!root.join(path).exists(), "{path} should not be generated");
        }
        assert!(!read(&root, "README.md").contains("docker"));
        assert!(!read(&root, "Cargo.toml").contains("sqlx"));
        assert!(!root.join("migrations").exists());
    }

    #[test]
    fn skipping_docker_keeps_ci_without_the_image_job() {
        let parent = tempfile::tempdir().expect("temporary directory should be available");
        let options = NewApplication {
            docker: false,
            database: false,
            ..options("plain")
        };

        let root = create_application(parent.path(), &options).expect("generation should succeed");

        let workflow = read(&root, ".github/workflows/ci.yml");
        assert!(workflow.contains("cargo test"));
        assert!(!workflow.contains("docker"));
        assert!(!read(&root, ".github/dependabot.yml").contains("docker"));
    }

    #[test]
    fn every_placeholder_is_rendered() {
        let parent = tempfile::tempdir().expect("temporary directory should be available");
        let root = create_application(parent.path(), &options("sample-app"))
            .expect("generation should succeed");

        for (path, content) in snapshot(&root) {
            assert!(
                !content.contains("%%"),
                "{path} contains an unrendered placeholder"
            );
        }
    }

    #[test]
    fn forge_path_is_written_as_a_path_dependency() {
        let parent = tempfile::tempdir().expect("temporary directory should be available");
        let options = NewApplication {
            forge: ForgeSource::Path("../forge/crates/forge".to_owned()),
            ..options("sample-app")
        };

        let root = create_application(parent.path(), &options).expect("generation should succeed");

        assert!(read(&root, "Cargo.toml").contains("forge = { path = \"../forge/crates/forge\" }"));
    }

    #[test]
    fn forge_path_cannot_inject_toml() {
        let parent = tempfile::tempdir().expect("temporary directory should be available");
        for path in ["", "x\" }\nevil = { path = \"/", "C:\\forge", "a\u{0}b"] {
            let options = NewApplication {
                forge: ForgeSource::Path(path.to_owned()),
                ..options("sample-app")
            };

            let error = create_application(parent.path(), &options)
                .expect_err("unsafe path must be rejected");

            assert!(matches!(error, CliError::InvalidForgePath { .. }));
            assert!(!parent.path().join("sample-app").exists());
        }
    }

    #[test]
    fn refuses_to_overwrite_an_existing_destination() {
        let parent = tempfile::tempdir().expect("temporary directory should be available");
        let existing = parent.path().join("sample-app");
        fs::create_dir(&existing).expect("test directory should be created");
        fs::write(existing.join("keep.txt"), "user data").expect("fixture should be written");

        let error = create_application(parent.path(), &options("sample-app"))
            .expect_err("an existing destination must be rejected");

        assert!(matches!(error, CliError::DestinationExists(path) if path == existing));
        assert_eq!(read(&existing, "keep.txt"), "user data");
    }

    /// `examples/hello-forge` is the committed output of
    /// `forge new hello-forge --skip-ci --forge-path ../../crates/forge`.
    /// Regenerate it with `FORGE_BLESS=1 cargo test -p forge-cli`.
    #[test]
    fn hello_forge_example_matches_generator_output() {
        let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello-forge");
        let parent = tempfile::tempdir().expect("temporary directory should be available");
        let options = NewApplication {
            ci: false,
            database: false,
            forge: ForgeSource::Path("../../crates/forge".to_owned()),
            ..options("hello-forge")
        };
        let generated = create_application(parent.path(), &options).expect("generation succeeds");
        let expected = snapshot(&generated);

        if std::env::var_os("FORGE_BLESS").is_some() {
            if example.exists() {
                for relative in snapshot(&example).keys() {
                    fs::remove_file(example.join(relative)).expect("stale file is removable");
                }
            }
            for (relative, content) in &expected {
                write_file(&example.join(relative), content).expect("example is writable");
            }
            return;
        }

        let actual = snapshot(&example);
        let drift: Vec<&str> = expected
            .keys()
            .chain(actual.keys())
            .filter(|path| expected.get(*path) != actual.get(*path))
            .map(String::as_str)
            .collect();
        assert!(
            drift.is_empty(),
            "examples/hello-forge differs from generator output in {drift:?}; \
             run `FORGE_BLESS=1 cargo test -p forge-cli` and review the diff"
        );
    }
}
