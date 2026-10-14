//! Resource generator for typed domain/application contracts and PostgreSQL
//! persistence, with explicit tenant isolation when tenancy is enabled.

use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::CliError;
use crate::preview::{GenerationPreview, PreviewFile};

const RESOURCE_TYPES: &str =
    "supported field types: string, text, uuid, bool, enum, enum[value|value]";
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];

#[derive(Clone, Debug, Eq, PartialEq)]
enum FieldType {
    String,
    Text,
    Uuid,
    Bool,
    /// Text-backed enum. When values are present they are constrained in SQL;
    /// Rust-facing representation stays a string until generated closed enums
    /// can be supported across serialization and persistence consistently.
    Enum(Option<Vec<String>>),
}

impl FieldType {
    fn rust_type(&self) -> &'static str {
        match self {
            Self::String | Self::Text | Self::Enum(_) => "String",
            Self::Uuid => "uuid::Uuid",
            Self::Bool => "bool",
        }
    }

    fn sql_type(&self) -> &'static str {
        match self {
            Self::String | Self::Text | Self::Enum(_) => "TEXT",
            Self::Uuid => "UUID",
            Self::Bool => "BOOLEAN",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceField {
    name: String,
    ty: FieldType,
}

pub(crate) struct ResourceGenerator {
    pub(crate) name: String,
    pub(crate) fields: String,
    pub(crate) tenant_scoped: bool,
    pub(crate) version: Option<u64>,
}

impl ResourceGenerator {
    pub(crate) fn preview(&self, current_dir: &Path) -> Result<GenerationPreview, CliError> {
        let (root, files) = self.plan(current_dir)?;
        Ok(GenerationPreview::new(self.version.is_none(), files).relative_to(&root))
    }

    pub(crate) fn generate(&self, current_dir: &Path) -> Result<PathBuf, CliError> {
        let (root, files) = self.plan(current_dir)?;
        let mut created = Vec::new();
        for file in files.iter().filter(|file| file.operation == "create") {
            let directory_error = file.path.parent().and_then(|parent| {
                fs::create_dir_all(parent)
                    .err()
                    .map(|source| (parent.to_path_buf(), source))
            });
            if let Some((path, source)) = directory_error {
                for created_path in created {
                    let _ = fs::remove_file(created_path);
                }
                return Err(CliError::CreateDirectory { path, source });
            }
            if let Err(error) = write_new_file(&file.path, &file.content) {
                for created_path in created {
                    let _ = fs::remove_file(created_path);
                }
                return Err(error);
            }
            created.push(file.path.clone());
        }
        for path in created
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        {
            if let Err(error) = format_generated_rust(path) {
                for created_path in created {
                    let _ = fs::remove_file(created_path);
                }
                return Err(error);
            }
        }
        let mut appended: Vec<(PathBuf, String)> = Vec::new();
        for module_file in files.iter().filter(|file| file.operation == "append") {
            if let Err(error) = append_module(&module_file.path, module_file.content.trim()) {
                for (path, declaration) in appended {
                    remove_appended_module(&path, &declaration);
                }
                for created_path in created {
                    let _ = fs::remove_file(created_path);
                }
                return Err(error);
            }
            appended.push((
                module_file.path.clone(),
                module_file.content.trim().to_owned(),
            ));
        }
        Ok(root)
    }

    fn plan(&self, current_dir: &Path) -> Result<(PathBuf, Vec<PreviewFile>), CliError> {
        validate_resource_name(&self.name)?;
        let fields = parse_fields(&self.fields)?;
        let root = find_application_dir(current_dir)?;
        let entity_dir = root.join("src/domain/entities");
        let entity_module = entity_dir.join("mod.rs");
        let entity_file = entity_dir.join(format!("{}.rs", to_snake_case(&self.name)));
        let migrations_dir = root.join("migrations");
        if !migrations_dir.is_dir() {
            return Err(CliError::ResourceRequiresDatabase(root));
        }
        let table = pluralize(&to_snake_case(&self.name));
        let version = match self.version {
            Some(version) if version > 0 && version <= i64::MAX as u64 => version.to_string(),
            Some(_) => {
                return Err(CliError::InvalidMigrationName {
                    name: self.name.clone(),
                    reason: "migration version must be between 1 and 9223372036854775807",
                });
            }
            None => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(CliError::Clock)?
                .as_nanos()
                .to_string(),
        };
        let stem = format!("{version}_{table}");
        let up_path = migrations_dir.join(format!("{stem}.up.sql"));
        let down_path = migrations_dir.join(format!("{stem}.down.sql"));
        let module_declaration = format!("pub mod {};", to_snake_case(&self.name));
        let ports_module = root.join("src/application/ports/mod.rs");
        let use_cases_module = root.join("src/application/use_cases/mod.rs");
        let persistence_module = root.join("src/adapters/persistence/mod.rs");
        let application_port_module = format!("pub mod {};", to_snake_case(&self.name));
        let application_use_case_module = format!("pub mod {};", to_snake_case(&self.name));
        let persistence_adapter_module = format!("pub mod {}_postgres;", to_snake_case(&self.name));
        let port_file = root.join(format!(
            "src/application/ports/{}.rs",
            to_snake_case(&self.name)
        ));
        let use_case_file = root.join(format!(
            "src/application/use_cases/{}.rs",
            to_snake_case(&self.name)
        ));
        let persistence_adapter_file = root.join(format!(
            "src/adapters/persistence/{}_postgres.rs",
            to_snake_case(&self.name)
        ));
        let integration_test_file =
            root.join(format!("tests/resource_{}.rs", to_snake_case(&self.name)));
        for path in [
            &entity_module,
            &entity_file,
            &migrations_dir,
            &ports_module,
            &use_cases_module,
            &persistence_module,
            &port_file,
            &use_case_file,
            &persistence_adapter_file,
            &integration_test_file,
        ] {
            ensure_no_symlink_components(&root, path)?;
        }
        for path in [
            &entity_file,
            &up_path,
            &down_path,
            &port_file,
            &use_case_file,
            &persistence_adapter_file,
            &integration_test_file,
        ] {
            if path.exists() {
                return Err(CliError::DestinationExists(path.clone()));
            }
        }
        let current_modules =
            fs::read_to_string(&entity_module).map_err(|source| CliError::ReadFile {
                path: entity_module.clone(),
                source,
            })?;
        if current_modules
            .lines()
            .any(|line| line.trim() == module_declaration)
        {
            return Err(CliError::ResourceModuleAlreadyRegistered(
                module_declaration,
            ));
        }
        for (path, declaration) in [
            (&ports_module, &application_port_module),
            (&use_cases_module, &application_use_case_module),
            (&persistence_module, &persistence_adapter_module),
        ] {
            let modules = fs::read_to_string(path).map_err(|source| CliError::ReadFile {
                path: path.clone(),
                source,
            })?;
            if modules.lines().any(|line| line.trim() == declaration) {
                return Err(CliError::ResourceModuleAlreadyRegistered(
                    declaration.clone(),
                ));
            }
        }
        Ok((
            root,
            vec![
                PreviewFile {
                    path: entity_file,
                    operation: "create",
                    content: render_entity(&self.name, &fields, self.tenant_scoped),
                },
                PreviewFile {
                    path: up_path,
                    operation: "create",
                    content: render_up_migration(&table, &fields, self.tenant_scoped),
                },
                PreviewFile {
                    path: down_path,
                    operation: "create",
                    content: format!("DROP TABLE IF EXISTS \"{table}\";\n"),
                },
                PreviewFile {
                    path: entity_module,
                    operation: "append",
                    content: format!("\n{module_declaration}\n"),
                },
                PreviewFile {
                    path: ports_module,
                    operation: "append",
                    content: format!("\n{application_port_module}\n"),
                },
                PreviewFile {
                    path: use_cases_module,
                    operation: "append",
                    content: format!("\n{application_use_case_module}\n"),
                },
                PreviewFile {
                    path: persistence_module,
                    operation: "append",
                    content: format!("\n{persistence_adapter_module}\n"),
                },
                PreviewFile {
                    path: port_file,
                    operation: "create",
                    content: render_repository_port(&self.name, self.tenant_scoped),
                },
                PreviewFile {
                    path: use_case_file,
                    operation: "create",
                    content: render_resource_use_cases(&self.name, &fields, self.tenant_scoped),
                },
                PreviewFile {
                    path: persistence_adapter_file,
                    operation: "create",
                    content: render_postgres_adapter(
                        &self.name,
                        &table,
                        &fields,
                        self.tenant_scoped,
                    ),
                },
                PreviewFile {
                    path: integration_test_file,
                    operation: "create",
                    content: render_resource_integration_test(
                        &self.name,
                        &fields,
                        self.tenant_scoped,
                    ),
                },
            ],
        ))
    }
}

pub(crate) fn format_generated_rust(path: &Path) -> Result<(), CliError> {
    let output = Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .arg(path)
        .output()
        .map_err(|source| CliError::SpawnRustfmt {
            path: path.to_path_buf(),
            source,
        })?;
    if !output.status.success() {
        return Err(CliError::RustfmtFailed {
            path: path.to_path_buf(),
            code: output.status.code(),
        });
    }
    Ok(())
}

fn parse_fields(input: &str) -> Result<Vec<ResourceField>, CliError> {
    if input.trim().is_empty() {
        return Err(CliError::InvalidResourceField {
            name: input.to_owned(),
            reason: "at least one field is required",
        });
    }
    let mut fields = Vec::new();
    let mut names = HashSet::new();
    for part in input.split(',') {
        let Some((name, kind)) = part.trim().split_once(':') else {
            return Err(CliError::InvalidResourceField {
                name: part.trim().to_owned(),
                reason: "expected name:type",
            });
        };
        let name = name.trim();
        if !valid_field_name(name) {
            return Err(CliError::InvalidResourceField {
                name: name.to_owned(),
                reason: "use a lowercase Rust identifier; id, tenant_id, version, created_at and updated_at are reserved",
            });
        }
        if !names.insert(name.to_owned()) {
            return Err(CliError::InvalidResourceField {
                name: name.to_owned(),
                reason: "duplicate field",
            });
        }
        let kind = kind.trim();
        let ty = match kind {
            "string" => FieldType::String,
            "text" => FieldType::Text,
            "uuid" => FieldType::Uuid,
            "bool" => FieldType::Bool,
            "enum" => FieldType::Enum(None),
            value if value.starts_with("enum[") && value.ends_with(']') => {
                let values = &value[5..value.len() - 1];
                let variants = values.split('|').map(str::trim).collect::<Vec<_>>();
                if variants.len() < 2
                    || variants.iter().any(|variant| !valid_enum_value(variant))
                    || variants.iter().collect::<HashSet<_>>().len() != variants.len()
                {
                    return Err(CliError::InvalidResourceField {
                        name: kind.to_owned(),
                        reason: "enum values must contain at least two unique lowercase snake_case values, e.g. enum[draft|published]",
                    });
                }
                FieldType::Enum(Some(variants.into_iter().map(str::to_owned).collect()))
            }
            other => {
                return Err(CliError::InvalidResourceField {
                    name: other.to_owned(),
                    reason: RESOURCE_TYPES,
                });
            }
        };
        fields.push(ResourceField {
            name: name.to_owned(),
            ty,
        });
    }
    if fields.is_empty() {
        return Err(CliError::InvalidResourceField {
            name: input.to_owned(),
            reason: "at least one field is required",
        });
    }
    Ok(fields)
}

fn valid_enum_value(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
        && value.len() <= 64
        && !value.ends_with('_')
        && !value.contains("__")
}

pub(crate) fn validate_resource_name(name: &str) -> Result<(), CliError> {
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|character| character.is_ascii_alphanumeric())
        && name.len() <= 64;
    if !valid || RUST_KEYWORDS.contains(&name) {
        return Err(CliError::InvalidResourceName {
            name: name.to_owned(),
            reason: "use a PascalCase ASCII identifier of at most 64 characters",
        });
    }
    Ok(())
}

fn valid_field_name(name: &str) -> bool {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
        && name.len() <= 64
        && !name.starts_with('_')
        && !name.ends_with('_')
        && !name.contains("__");
    valid
        && !RUST_KEYWORDS.contains(&name)
        && !["id", "tenant_id", "version", "created_at", "updated_at"].contains(&name)
}

fn render_entity(name: &str, fields: &[ResourceField], tenant_scoped: bool) -> String {
    let field_declarations = fields
        .iter()
        .map(|field| format!("    {}: {},", field.name, field.ty.rust_type()))
        .collect::<Vec<_>>()
        .join("\n");
    let field_arguments = fields
        .iter()
        .map(|field| format!("{}: {}", field.name, field.ty.rust_type()))
        .collect::<Vec<_>>()
        .join(", ");
    let field_initializers = fields
        .iter()
        .map(|field| format!("            {},", field.name))
        .collect::<Vec<_>>()
        .join("\n");
    let field_updates = fields
        .iter()
        .map(|field| format!("        self.{0} = {0};", field.name))
        .collect::<Vec<_>>()
        .join("\n");
    let accessors = fields
        .iter()
        .map(|field| {
            format!(
                "    #[must_use]\n    pub const fn {}(&self) -> &{} {{\n        &self.{}\n    }}\n",
                field.name,
                field.ty.rust_type(),
                field.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let tenant_field = if tenant_scoped {
        "    tenant_id: forge::tenancy::TenantId,\n"
    } else {
        ""
    };
    let tenant_argument = if tenant_scoped {
        "tenant_id: forge::tenancy::TenantId, "
    } else {
        ""
    };
    let tenant_initializer = if tenant_scoped {
        "            tenant_id,\n"
    } else {
        ""
    };
    let tenant_getter = if tenant_scoped {
        "    #[must_use]\n    pub const fn tenant_id(&self) -> forge::tenancy::TenantId {\n        self.tenant_id\n    }\n\n"
    } else {
        ""
    };
    let tenant_persistence_argument = if tenant_scoped {
        "tenant_id: forge::tenancy::TenantId, "
    } else {
        ""
    };
    let tenant_persistence_initializer = if tenant_scoped {
        "            tenant_id,\n"
    } else {
        ""
    };

    format!(
        "//! Generated {name} domain entity.\n\nuse forge::core::{{Id, Version}};\nuse serde::{{Deserialize, Serialize}};\n\n/// {name} domain entity.\n#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]\npub struct {name} {{\n    id: Id<Self>,\n    version: Version,\n{tenant_field}{field_declarations}\n}}\n\nimpl {name} {{\n    /// Creates a new {name} with a UUIDv7 identifier and initial record version.\n    #[must_use]\n    pub fn new({tenant_argument}{field_arguments}) -> Self {{\n        Self {{\n            id: Id::new(),\n            version: Version::INITIAL,\n{tenant_initializer}{field_initializers}\n        }}\n    }}\n\n    /// Reconstitutes persisted state after the adapter validates the UUID version.\n    #[must_use]\n    pub fn from_persistence(id: Id<Self>, version: Version, {tenant_persistence_argument}{field_arguments}) -> Self {{\n        Self {{\n            id,\n            version,\n{tenant_persistence_initializer}{field_initializers}\n        }}\n    }}\n\n    /// Replaces mutable resource fields; the repository persists with this version as its concurrency token.\n    pub fn revise(&mut self, {field_arguments}) {{\n{field_updates}\n    }}\n\n    /// Stable typed identifier.\n    #[must_use]\n    pub const fn id(&self) -> Id<Self> {{\n        self.id\n    }}\n\n    /// Optimistic-locking version stored in the database.\n    #[must_use]\n    pub const fn version(&self) -> Version {{\n        self.version\n    }}\n\n{tenant_getter}{accessors}}}\n"
    )
}

fn render_repository_port(name: &str, tenant_scoped: bool) -> String {
    let module = to_snake_case(name);
    let tenant_import = if tenant_scoped {
        "use forge::tenancy::TenantContext;\n"
    } else {
        ""
    };
    let context_argument = if tenant_scoped {
        "context: &TenantContext, "
    } else {
        ""
    };
    let scope_documentation = if tenant_scoped {
        "Tenant-scoped: every method takes an authorized TenantContext. Implementations must apply it to every query and transaction."
    } else {
        "Global resource: methods do not carry tenant state."
    };
    format!(
        "//! Application persistence contract for {name}.\n\nuse std::future::Future;\nuse forge::core::{{ErrorCategory, ForgeError, Id, Page, PageRequest, Version}};\n{tenant_import}use crate::domain::entities::{module}::{name};\n\n/// Persistence failures expressed without driver types.\n#[derive(Debug, thiserror::Error)]\npub enum RepositoryError {{\n    /// The requested row does not exist.\n    #[error(\"resource not found\")]\n    NotFound,\n    /// The row changed since the caller read it.\n    #[error(\"resource version conflict\")]\n    VersionConflict,\n    /// A backend failure with a safe message and retained diagnostic source.\n    #[error(transparent)]\n    Backend(#[from] ForgeError),\n}}\n\nimpl From<RepositoryError> for ForgeError {{\n    fn from(error: RepositoryError) -> Self {{\n        match error {{\n            RepositoryError::NotFound => Self::new(ErrorCategory::NotFound, \"resource_not_found\", \"the requested resource was not found\"),\n            RepositoryError::VersionConflict => Self::new(ErrorCategory::Conflict, \"concurrent_modification\", \"the resource changed concurrently; reload it and retry\"),\n            RepositoryError::Backend(error) => error,\n        }}\n    }}\n}}\n\n/// Repository port for {name}.\n///\n/// {scope_documentation}\n/// Implementations must perform version-checked writes and deletes, use deterministic\n/// keyset pagination, and keep SQLx types outside this contract.\npub trait {name}Repository: Send + Sync {{\n    /// Inserts a new resource and returns persisted state.\n    fn insert(&self, {context_argument}resource: &{name}) -> impl Future<Output = Result<{name}, RepositoryError>> + Send;\n\n    /// Finds a resource by its typed identifier.\n    fn find(&self, {context_argument}id: Id<{name}>) -> impl Future<Output = Result<Option<{name}>, RepositoryError>> + Send;\n\n    /// Updates the resource only when its current version still matches.\n    fn update(&self, {context_argument}resource: &{name}) -> impl Future<Output = Result<{name}, RepositoryError>> + Send;\n\n    /// Deletes only when the persisted version matches `expected_version`.\n    fn delete(&self, {context_argument}id: Id<{name}>, expected_version: Version) -> impl Future<Output = Result<(), RepositoryError>> + Send;\n\n    /// Lists one bounded page in descending UUIDv7 order.\n    fn page(&self, {context_argument}request: PageRequest<{name}>) -> impl Future<Output = Result<Page<{name}, {name}>, RepositoryError>> + Send;\n}}\n"
    )
}

fn render_resource_use_cases(name: &str, fields: &[ResourceField], tenant_scoped: bool) -> String {
    let module = to_snake_case(name);
    let field_declarations = fields
        .iter()
        .map(|field| format!("    pub {}: {},", field.name, field.ty.rust_type()))
        .collect::<Vec<_>>()
        .join("\n");
    let field_accesses = fields
        .iter()
        .map(|field| format!("command.{},", field.name))
        .collect::<Vec<_>>()
        .join("\n            ");
    let field_update_accesses = fields
        .iter()
        .map(|field| format!("command.{},", field.name))
        .collect::<Vec<_>>()
        .join("\n            ");
    let context_import = if tenant_scoped {
        "use forge::tenancy::TenantContext;\n"
    } else {
        ""
    };
    let context_argument = if tenant_scoped {
        "context: &TenantContext, "
    } else {
        ""
    };
    let context_call = if tenant_scoped { "context, " } else { "" };
    let tenant_constructor_argument = if tenant_scoped {
        "context.tenant_id(), "
    } else {
        ""
    };
    let error_category_import = if tenant_scoped { "ErrorCategory, " } else { "" };
    let authorization_checks = if tenant_scoped {
        ["create", "read", "update", "delete", "list"]
            .map(|action| format!("require_permission(context, \"{module}:{action}\")?;\n        "))
    } else {
        std::array::from_fn(|_| String::new())
    };
    let permission_helper = if tenant_scoped {
        "\nfn require_permission(context: &TenantContext, expected: &str) -> Result<(), ForgeError> {\n    if context.authorization().permission().as_str() == expected {\n        Ok(())\n    } else {\n        Err(ForgeError::new(ErrorCategory::Forbidden, \"permission_denied\", \"caller is not authorized for this resource operation\"))\n    }\n}\n"
    } else {
        ""
    };
    let mut source = format!(
        "//! Application use cases for {name}.\n\nuse forge::core::{{{error_category_import}ForgeError, Id, Page, PageRequest, Version}};\n{context_import}use crate::{{\n    application::ports::{module}::{{{name}Repository, RepositoryError}},\n    domain::entities::{module}::{name},\n}};\n\n/// Input for creating a {name}.\n#[derive(Debug)]\npub struct Create{name} {{\n{field_declarations}\n}}\n\n/// Input for updating a {name} with an optimistic concurrency token.\n#[derive(Debug)]\npub struct Update{name} {{\n    /// Resource to update.\n    pub id: Id<{name}>,\n    /// Version previously observed by the caller.\n    pub expected_version: Version,\n{field_declarations}\n}}\n\n/// CRUD use cases over a repository selected by the composition root.\n#[derive(Debug)]\npub struct {name}UseCases<R> {{\n    repository: R,\n}}\n\nimpl<R: {name}Repository> {name}UseCases<R> {{\n    /// Builds use cases around a concrete repository.\n    #[must_use]\n    pub const fn new(repository: R) -> Self {{ Self {{ repository }} }}\n\n    /// Creates a resource.\n    pub async fn create(&self, {context_argument}command: Create{name}) -> Result<{name}, ForgeError> {{\n        __AUTH_CREATE__let resource = {name}::new({tenant_constructor_argument}{field_accesses});\n        Ok(self.repository.insert({context_call}&resource).await?)\n    }}\n\n    /// Loads a resource or returns a stable not-found error.\n    pub async fn get(&self, {context_argument}id: Id<{name}>) -> Result<{name}, ForgeError> {{\n        __AUTH_GET__self.repository.find({context_call}id).await?.ok_or_else(|| RepositoryError::NotFound.into())\n    }}\n\n    /// Updates a resource only if the caller's expected version is current.\n    pub async fn update(&self, {context_argument}command: Update{name}) -> Result<{name}, ForgeError> {{\n        __AUTH_UPDATE__let mut resource = self.repository.find({context_call}command.id).await?.ok_or(RepositoryError::NotFound)?;\n        if resource.version() != command.expected_version {{ return Err(RepositoryError::VersionConflict.into()); }}\n        resource.revise({field_update_accesses});\n        Ok(self.repository.update({context_call}&resource).await?)\n    }}\n\n    /// Deletes a resource using its optimistic concurrency token.\n    pub async fn delete(&self, {context_argument}id: Id<{name}>, expected_version: Version) -> Result<(), ForgeError> {{\n        __AUTH_DELETE__Ok(self.repository.delete({context_call}id, expected_version).await?)\n    }}\n\n    /// Returns one bounded keyset-paginated result page.\n    pub async fn page(&self, {context_argument}request: PageRequest<{name}>) -> Result<Page<{name}, {name}>, ForgeError> {{\n        __AUTH_PAGE__Ok(self.repository.page({context_call}request).await?)\n    }}\n}}\n__PERMISSION_HELPER__"
    );
    for (placeholder, value) in [
        ("__AUTH_CREATE__", authorization_checks[0].as_str()),
        ("__AUTH_GET__", authorization_checks[1].as_str()),
        ("__AUTH_UPDATE__", authorization_checks[2].as_str()),
        ("__AUTH_DELETE__", authorization_checks[3].as_str()),
        ("__AUTH_PAGE__", authorization_checks[4].as_str()),
        ("__PERMISSION_HELPER__", permission_helper),
    ] {
        source = source.replace(placeholder, value);
    }
    source
}

fn render_postgres_adapter(
    name: &str,
    table: &str,
    fields: &[ResourceField],
    tenant_scoped: bool,
) -> String {
    let module = to_snake_case(name);
    let context_argument = if tenant_scoped {
        "context: &forge::tenancy::TenantContext, "
    } else {
        ""
    };
    let context_bind = if tenant_scoped {
        "let query = query.bind(*context.tenant_id().as_uuid());\n        "
    } else {
        ""
    };
    let tenant_column = if tenant_scoped { ", \"tenant_id\"" } else { "" };
    let tenant_select = if tenant_scoped {
        "    let tenant_raw: uuid::Uuid = row.try_get(\"tenant_id\").map_err(corrupt_record)?;\n    let tenant_id = forge::tenancy::TenantId::from_uuid(tenant_raw).map_err(corrupt_record)?;\n"
    } else {
        ""
    };
    let tenant_constructor_argument = if tenant_scoped { "tenant_id, " } else { "" };
    let find_tenant_predicate = if tenant_scoped {
        " AND \"tenant_id\" = $2"
    } else {
        ""
    };
    let update_tenant_predicate = if tenant_scoped {
        format!(" AND \"tenant_id\" = ${}", fields.len() + 3)
    } else {
        String::new()
    };
    let delete_tenant_predicate = if tenant_scoped {
        " AND \"tenant_id\" = $3"
    } else {
        ""
    };
    let page_tenant_predicate = if tenant_scoped {
        " AND \"tenant_id\" = $2"
    } else {
        ""
    };
    let begin_transaction = if tenant_scoped {
        "crate::infrastructure::database::begin_tenant_transaction(&self.pool, context).await"
    } else {
        "self.pool.begin().await"
    };
    let selected_columns = {
        let mut columns = vec!["\"id\"".to_owned()];
        if tenant_scoped {
            columns.push("\"tenant_id\"".to_owned());
        }
        columns.extend(fields.iter().map(|field| format!("\"{}\"", field.name)));
        columns.push("\"version\"".to_owned());
        columns.join(", ")
    };
    let field_columns = fields
        .iter()
        .map(|field| format!("\"{}\"", field.name))
        .collect::<Vec<_>>()
        .join(", ");
    let insert_columns = format!("\"id\"{tenant_column}, {field_columns}, \"version\"");
    let insert_values = (1..=(fields.len() + usize::from(tenant_scoped) + 2))
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let update_assignments = fields
        .iter()
        .enumerate()
        .map(|(index, field)| format!("\"{}\" = ${}", field.name, index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let id_parameter = fields.len() + 1;
    let version_parameter = fields.len() + 2;
    let mut insert_binds = format!(
        "let mut query = sqlx::query({:?}).bind(*resource.id().as_uuid());\n        ",
        format!(
            "INSERT INTO \"{table}\" ({insert_columns}) VALUES ({insert_values}) RETURNING {selected_columns}"
        )
    );
    if tenant_scoped {
        insert_binds.push_str("query = query.bind(*context.tenant_id().as_uuid());\n        ");
    }
    for field in fields {
        let value = match &field.ty {
            FieldType::String | FieldType::Text | FieldType::Enum(_) => {
                format!("resource.{}().clone()", field.name)
            }
            FieldType::Uuid | FieldType::Bool => format!("*resource.{}()", field.name),
        };
        insert_binds.push_str(&format!("query = query.bind({value});\n        ",));
    }
    insert_binds.push_str("query = query.bind(resource.version().get());\n        ");
    let mut update_binds = format!(
        "let mut query = sqlx::query({:?});\n        ",
        format!(
            "UPDATE \"{table}\" SET {update_assignments}, \"version\" = \"version\" + 1, \"updated_at\" = now() WHERE \"id\" = ${id_parameter} AND \"version\" = ${version_parameter}{update_tenant_predicate} RETURNING {selected_columns}"
        )
    );
    for field in fields {
        let value = match &field.ty {
            FieldType::String | FieldType::Text | FieldType::Enum(_) => {
                format!("resource.{}().clone()", field.name)
            }
            FieldType::Uuid | FieldType::Bool => format!("*resource.{}()", field.name),
        };
        update_binds.push_str(&format!("query = query.bind({value});\n        ",));
    }
    update_binds.push_str(&format!(
        "query = query.bind(*resource.id().as_uuid()).bind(resource.version().get());\n        {context_bind}",
    ));
    let select = format!(
        "SELECT {selected_columns} FROM \"{table}\" WHERE \"id\" = $1{find_tenant_predicate}"
    );
    let delete = format!(
        "DELETE FROM \"{table}\" WHERE \"id\" = $1 AND \"version\" = $2{delete_tenant_predicate}"
    );
    let page = format!(
        "SELECT {selected_columns} FROM \"{table}\" WHERE ($1::uuid IS NULL OR \"id\" < $1){page_tenant_predicate} ORDER BY \"id\" DESC LIMIT ${} ",
        if tenant_scoped { 3 } else { 2 }
    );
    let mut fields_from_row = String::new();
    for field in fields {
        fields_from_row.push_str(&format!(
            "    let {}: {} = row.try_get({:?}).map_err(corrupt_record)?;\n",
            field.name,
            field.ty.rust_type(),
            field.name
        ));
    }
    let entity_args = fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut source = r#"//! PostgreSQL persistence adapter generated for __NAME__.

use forge::core::{ErrorCategory, ForgeError, Id, Page, PageRequest, Version};
use sqlx::{PgPool, Row};
use crate::{
    application::ports::__MODULE__::{__NAME__Repository, RepositoryError},
    domain::entities::__MODULE__::__NAME__,
};

/// PostgreSQL implementation for the generated __NAME__ repository port.
#[derive(Clone, Debug)]
pub struct Pg__NAME__Repository {
    pool: PgPool,
}

impl Pg__NAME__Repository {
    /// Uses the runtime pool; callers must never pass the migration-role pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

impl __NAME__Repository for Pg__NAME__Repository {
    async fn insert(&self, __CONTEXT_ARG__resource: &__NAME__) -> Result<__NAME__, RepositoryError> {
        let mut transaction = __BEGIN__.map_err(map_sqlx_error)?;
        __INSERT_BINDS__
        let row = query.fetch_one(&mut *transaction).await.map_err(map_sqlx_error)?;
        let resource = map_row(row)?;
        transaction.commit().await.map_err(map_sqlx_error)?;
        Ok(resource)
    }

    async fn find(&self, __CONTEXT_ARG__id: Id<__NAME__>) -> Result<Option<__NAME__>, RepositoryError> {
        let mut transaction = __BEGIN__.map_err(map_sqlx_error)?;
        let query = sqlx::query(__SELECT__).bind(*id.as_uuid());
        __FIND_CONTEXT_BIND__
        let row = query.fetch_optional(&mut *transaction).await.map_err(map_sqlx_error)?;
        let resource = row.map(map_row).transpose()?;
        transaction.commit().await.map_err(map_sqlx_error)?;
        Ok(resource)
    }

    async fn update(&self, __CONTEXT_ARG__resource: &__NAME__) -> Result<__NAME__, RepositoryError> {
        let mut transaction = __BEGIN__.map_err(map_sqlx_error)?;
        __UPDATE_BINDS__
        let row = query.fetch_optional(&mut *transaction).await.map_err(map_sqlx_error)?;
        let resource = row.map(map_row).transpose()?.ok_or(RepositoryError::VersionConflict)?;
        transaction.commit().await.map_err(map_sqlx_error)?;
        Ok(resource)
    }

    async fn delete(&self, __CONTEXT_ARG__id: Id<__NAME__>, expected_version: Version) -> Result<(), RepositoryError> {
        let mut transaction = __BEGIN__.map_err(map_sqlx_error)?;
        let query = sqlx::query(__DELETE__).bind(*id.as_uuid()).bind(expected_version.get());
        __DELETE_CONTEXT_BIND__
        let result = query.execute(&mut *transaction).await.map_err(map_sqlx_error)?;
        if result.rows_affected() == 0 { return Err(RepositoryError::VersionConflict); }
        transaction.commit().await.map_err(map_sqlx_error)
    }

    async fn page(&self, __CONTEXT_ARG__request: PageRequest<__NAME__>) -> Result<Page<__NAME__, __NAME__>, RepositoryError> {
        let mut transaction = __BEGIN__.map_err(map_sqlx_error)?;
        let query = sqlx::query(__PAGE__).bind(request.after.map(|cursor| *cursor.as_uuid()));
        __PAGE_CONTEXT_BIND__
        let query = query.bind(i64::from(request.limit.get()) + 1);
        let rows = query.fetch_all(&mut *transaction).await.map_err(map_sqlx_error)?;
        let resources: Result<Vec<__NAME__>, RepositoryError> = rows.into_iter().map(map_row).collect();
        let page = Page::from_overfetched(resources?, request.limit, |resource| resource.id());
        transaction.commit().await.map_err(map_sqlx_error)?;
        Ok(page)
    }
}

fn map_row(row: sqlx::postgres::PgRow) -> Result<__NAME__, RepositoryError> {
    let id_raw: uuid::Uuid = row.try_get("id").map_err(corrupt_record)?;
    let id = Id::<__NAME__>::from_uuid(id_raw).map_err(corrupt_record)?;
    let version_raw: i64 = row.try_get("version").map_err(corrupt_record)?;
    let version = Version::new(version_raw).map_err(corrupt_record)?;
__TENANT_ROW__    
__FIELDS_FROM_ROW__    Ok(__NAME__::from_persistence(id, version, __TENANT_CONSTRUCTOR____ENTITY_ARGS__))
}

fn corrupt_record(error: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::Backend(
        ForgeError::new(ErrorCategory::Internal, "invalid_persisted_resource", "stored resource data is invalid")
            .with_source(error),
    )
}

fn map_sqlx_error(error: sqlx::Error) -> RepositoryError {
    let database_code = error.as_database_error().and_then(|database| database.code());
    let conflict = database_code.as_deref().is_some_and(|code| matches!(code, "23505" | "23503" | "23514"));
    let retryable = database_code.as_deref().is_some_and(|code| matches!(code, "40001" | "40P01" | "55P03"));
    let (category, code, message) = if conflict {
        (ErrorCategory::Conflict, "persistence_constraint_conflict", "resource conflicts with a stored constraint")
    } else {
        (ErrorCategory::Infrastructure, "persistence_failure", "resource storage is temporarily unavailable")
    };
    RepositoryError::Backend(ForgeError::new(category, code, message).with_retryable(retryable).with_source(error))
}
"#
    .to_owned();
    let replacements = [
        ("__NAME__", name.to_owned()),
        ("__MODULE__", module),
        ("__CONTEXT_ARG__", context_argument.to_owned()),
        ("__BEGIN__", begin_transaction.to_owned()),
        ("__INSERT_BINDS__", insert_binds),
        ("__SELECT__", format!("{select:?}")),
        ("__FIND_CONTEXT_BIND__", context_bind.to_owned()),
        ("__UPDATE_BINDS__", update_binds),
        ("__DELETE__", format!("{delete:?}")),
        ("__DELETE_CONTEXT_BIND__", context_bind.to_owned()),
        ("__PAGE__", format!("{page:?}")),
        ("__PAGE_CONTEXT_BIND__", context_bind.to_owned()),
        ("__TENANT_ROW__", tenant_select.to_owned()),
        ("__FIELDS_FROM_ROW__", fields_from_row),
        (
            "__TENANT_CONSTRUCTOR__",
            tenant_constructor_argument.to_owned(),
        ),
        ("__ENTITY_ARGS__", entity_args),
    ];
    for (placeholder, value) in replacements {
        source = source.replace(placeholder, &value);
    }
    source
}

fn render_resource_integration_test(
    name: &str,
    fields: &[ResourceField],
    tenant_scoped: bool,
) -> String {
    let module = to_snake_case(name);
    let (context_imports, test_body, permission_helper) = if tenant_scoped {
        (
            "use forge::{\n    auth::{Session, UnixTimestamp},\n    security::{Permission, PrincipalId, RbacPolicy, Role},\n    tenancy::{Membership, TenantContext, TenantId},\n};\n",
            format!(
                r#"    let tenant_id = TenantId::new();
    let another_tenant = TenantId::new();
    let principal_id = PrincipalId::new();
    let create_context = authorized_context(tenant_id, principal_id, "{module}:create")?;
    let read_context = authorized_context(tenant_id, principal_id, "{module}:read")?;
    let list_context = authorized_context(tenant_id, principal_id, "{module}:list")?;
    let update_context = authorized_context(tenant_id, principal_id, "{module}:update")?;
    let delete_context = authorized_context(tenant_id, principal_id, "{module}:delete")?;
    let other_tenant_context = authorized_context(another_tenant, principal_id, "{module}:read")?;
    let use_cases = {name}UseCases::new(Pg{name}Repository::new(pool.clone()));

    let denied = use_cases.create(&read_context, create_input()).await;
    assert!(denied.is_err_and(|error| error.code() == "permission_denied"));

    let created = use_cases.create(&create_context, create_input()).await?;
    assert_eq!(created.version(), Version::INITIAL);
    let loaded = use_cases.get(&read_context, created.id()).await?;
    assert_eq!(loaded.id(), created.id());

    let page = use_cases.page(&list_context, PageRequest::default()).await?;
    assert!(page.items.iter().any(|item| item.id() == created.id()));
    let cross_tenant = use_cases.get(&other_tenant_context, created.id()).await;
    assert!(cross_tenant.is_err_and(|error| error.code() == "resource_not_found"));

    let updated = use_cases.update(
        &update_context,
        update_input(created.id(), created.version()),
    ).await?;
    assert_eq!(updated.version().get(), created.version().get() + 1);
    let stale = use_cases.update(
        &update_context,
        update_input(created.id(), created.version()),
    ).await;
    assert!(stale.is_err_and(|error| error.code() == "concurrent_modification"));

    use_cases.delete(&delete_context, updated.id(), updated.version()).await?;
    let deleted = use_cases.get(&read_context, created.id()).await;
    assert!(deleted.is_err_and(|error| error.code() == "resource_not_found"));
"#
            ),
            r#"
fn authorized_context(
    tenant_id: TenantId,
    principal_id: PrincipalId,
    permission_name: &str,
) -> Result<TenantContext, Box<dyn Error>> {
    let issued = UnixTimestamp::from_secs(10);
    let authenticated = Session::new(
        principal_id,
        issued,
        UnixTimestamp::from_secs(100),
    )?.authenticate(UnixTimestamp::from_secs(11))?;
    let role = Role::new("resource_test")?;
    let membership = Membership::active(tenant_id, principal_id, vec![role.clone()]);
    let permission = Permission::new(permission_name)?;
    let mut policy = RbacPolicy::new();
    policy.allow(role, permission.clone());
    Ok(TenantContext::authorize(authenticated, &membership, &policy, &permission)?)
}
"#,
        )
    } else {
        (
            "",
            format!(
                r#"    let use_cases = {name}UseCases::new(Pg{name}Repository::new(pool.clone()));
    let created = use_cases.create(create_input()).await?;
    assert_eq!(created.version(), Version::INITIAL);
    let loaded = use_cases.get(created.id()).await?;
    assert_eq!(loaded.id(), created.id());
    let page = use_cases.page(PageRequest::default()).await?;
    assert!(page.items.iter().any(|item| item.id() == created.id()));

    let updated = use_cases.update(update_input(created.id(), created.version())).await?;
    assert_eq!(updated.version().get(), created.version().get() + 1);
    let stale = use_cases.update(update_input(created.id(), created.version())).await;
    assert!(stale.is_err_and(|error| error.code() == "concurrent_modification"));
    use_cases.delete(updated.id(), updated.version()).await?;
    let deleted = use_cases.get(created.id()).await;
    assert!(deleted.is_err_and(|error| error.code() == "resource_not_found"));
"#
            ),
            "",
        )
    };
    let create_fields = fields
        .iter()
        .map(|field| {
            format!(
                "        {}: {},",
                field.name,
                integration_value(field, false)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let update_fields = fields
        .iter()
        .map(|field| {
            format!(
                "        {}: {},",
                field.name,
                integration_value(field, true)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut source = format!(
        r#"//! PostgreSQL integration contract for generated {name} persistence.
//!
//! Run explicitly with `FORGE_DATABASE_URL=... cargo test --test resource_{module} -- --ignored`.

use std::error::Error;

use app::{{
    adapters::persistence::{module}_postgres::Pg{name}Repository,
    application::use_cases::{module}::{{Create{name}, Update{name}, {name}UseCases}},
    domain::entities::{module}::{name},
}};
use forge::core::{{Id, PageRequest, Version}};
{context_imports}use sqlx::postgres::PgPoolOptions;

fn create_input() -> Create{name} {{
    Create{name} {{
{create_fields}
    }}
}}

fn update_input(id: Id<{name}>, expected_version: Version) -> Update{name} {{
    Update{name} {{
        id,
        expected_version,
{update_fields}
    }}
}}

#[test]
#[ignore = "requires migrated PostgreSQL via FORGE_DATABASE_URL"]
fn generated_resource_repository_obeys_crud_concurrency_and_tenant_boundaries() -> Result<(), Box<dyn Error>> {{
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {{
        let database_url = std::env::var("FORGE_DATABASE_URL")?;
        // One pooled connection deliberately exercises transaction-local RLS reset.
        let pool = PgPoolOptions::new().max_connections(1).connect(&database_url).await?;
{test_body}        pool.close().await;
        Ok(())
    }})
}}
{permission_helper}
"#,
    );
    source = source.replace("{context_imports}", context_imports);
    source = source.replace("{create_fields}", &create_fields);
    source = source.replace("{update_fields}", &update_fields);
    source = source.replace("{test_body}", &test_body);
    source = source.replace("{permission_helper}", permission_helper);
    source
}

fn integration_value(field: &ResourceField, updated: bool) -> String {
    match &field.ty {
        FieldType::String | FieldType::Text | FieldType::Enum(None) => {
            format!(
                "{:?}.to_owned()",
                if updated { "updated" } else { "created" }
            )
        }
        FieldType::Enum(Some(values)) => format!(
            "{:?}.to_owned()",
            if updated {
                values.last().map(String::as_str).unwrap_or_default()
            } else {
                values.first().map(String::as_str).unwrap_or_default()
            }
        ),
        FieldType::Uuid => "uuid::Uuid::now_v7()".to_owned(),
        FieldType::Bool => updated.to_string(),
    }
}

fn render_up_migration(table: &str, fields: &[ResourceField], tenant_scoped: bool) -> String {
    let mut columns = vec!["    \"id\" UUID PRIMARY KEY".to_owned()];
    if tenant_scoped {
        columns.push("    \"tenant_id\" UUID NOT NULL".to_owned());
    }
    columns.extend(fields.iter().map(|field| {
        let constraint = match &field.ty {
            FieldType::Enum(Some(values)) => format!(
                " CHECK (\"{}\" IN ({}))",
                field.name,
                values
                    .iter()
                    .map(|value| format!("'{value}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => String::new(),
        };
        format!(
            "    \"{}\" {} NOT NULL{}",
            field.name,
            field.ty.sql_type(),
            constraint
        )
    }));
    columns.extend([
        "    \"version\" BIGINT NOT NULL DEFAULT 1 CHECK (\"version\" > 0)".to_owned(),
        "    \"created_at\" TIMESTAMPTZ NOT NULL DEFAULT now()".to_owned(),
        "    \"updated_at\" TIMESTAMPTZ NOT NULL DEFAULT now()".to_owned(),
    ]);
    let mut sql = format!("CREATE TABLE \"{table}\" (\n{}\n);\n", columns.join(",\n"));
    if tenant_scoped {
        sql.push_str(&format!(
            "\nALTER TABLE \"{table}\" ENABLE ROW LEVEL SECURITY;\n\
             ALTER TABLE \"{table}\" FORCE ROW LEVEL SECURITY;\n\
             CREATE POLICY \"{table}_tenant_isolation\" ON \"{table}\"\n\
                 USING (\"tenant_id\" = NULLIF(current_setting('forge.tenant_id', true), '')::uuid)\n\
                 WITH CHECK (\"tenant_id\" = NULLIF(current_setting('forge.tenant_id', true), '')::uuid);\n"
        ));
    }
    sql
}

pub(crate) fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::with_capacity(name.len() + 4);
    for (index, character) in chars.iter().copied().enumerate() {
        let previous_is_lower_or_digit = index > 0
            && (chars[index - 1].is_ascii_lowercase() || chars[index - 1].is_ascii_digit());
        let acronym_boundary = index > 0
            && character.is_ascii_uppercase()
            && chars
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_lowercase())
            && chars[index - 1].is_ascii_uppercase();
        if character.is_ascii_uppercase() && (previous_is_lower_or_digit || acronym_boundary) {
            result.push('_');
        }
        result.push(character.to_ascii_lowercase());
    }
    result
}

fn pluralize(singular: &str) -> String {
    if singular.ends_with('s') || singular.ends_with('x') || singular.ends_with('z') {
        format!("{singular}es")
    } else if singular.ends_with('y') && !singular.ends_with("ey") {
        format!("{}ies", &singular[..singular.len() - 1])
    } else {
        format!("{singular}s")
    }
}

pub(crate) fn find_application_dir(current_dir: &Path) -> Result<PathBuf, CliError> {
    let mut directory = current_dir.to_path_buf();
    loop {
        if directory.join("Cargo.toml").is_file()
            && directory.join("src/domain/entities/mod.rs").is_file()
        {
            return Ok(directory);
        }
        if !directory.pop() {
            break;
        }
    }
    Err(CliError::InvalidApplicationName {
        name: "current directory".to_owned(),
        reason: "not inside a Forge application with src/domain/entities/mod.rs",
    })
}

/// Refuses generator targets that traverse a symlink below the application root.
/// The canonical root itself is trusted; generated paths must remain beneath it.
pub(crate) fn ensure_no_symlink_components(root: &Path, target: &Path) -> Result<(), CliError> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| CliError::UnsafeSymlinkPath(target.to_path_buf()))?;
    let mut current = fs::canonicalize(root).map_err(|source| CliError::ReadFile {
        path: root.to_path_buf(),
        source,
    })?;
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(CliError::UnsafeSymlinkPath(current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(CliError::ReadFile {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn write_new_file(path: &Path, content: &str) -> Result<(), CliError> {
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
    if let Err(source) = file.write_all(content.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(CliError::WriteFile {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

pub(crate) fn append_module(path: &Path, declaration: &str) -> Result<(), CliError> {
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|source| CliError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?;
    let original_len = file
        .metadata()
        .map_err(|source| CliError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if let Err(source) = writeln!(file, "\n{declaration}") {
        let _ = file.set_len(original_len);
        return Err(CliError::WriteFile {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

pub(crate) fn remove_appended_module(path: &Path, declaration: &str) {
    let Ok(file) = OpenOptions::new().read(true).write(true).open(path) else {
        return;
    };
    let Ok(contents) = fs::read(path) else {
        return;
    };
    let suffix = format!("\n{declaration}\n");
    if contents.ends_with(suffix.as_bytes()) {
        let new_len = contents.len().saturating_sub(suffix.len()) as u64;
        let _ = file.set_len(new_len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn generator_target_check_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary root");
        let outside = tempfile::tempdir().expect("temporary target");
        symlink(outside.path(), root.path().join("linked"))
            .expect("directory symlink should be created");

        let failure =
            ensure_no_symlink_components(root.path(), &root.path().join("linked/file.rs"))
                .expect_err("generator must reject symlink traversal");
        assert!(matches!(failure, CliError::UnsafeSymlinkPath(_)));
        assert!(!outside.path().join("file.rs").exists());
    }

    #[test]
    fn parses_supported_types_and_rejects_unsafe_or_duplicate_fields() {
        let parsed = parse_fields("name:string, owner_id:uuid, active:bool");
        assert!(matches!(parsed, Ok(ref fields) if fields.len() == 3));
        for invalid in [
            "../name:string",
            "type:text",
            "id:uuid",
            "name:string,name:bool",
            "state:enum[active]",
            "state:enum[active|active]",
            "state:enum[active|bad'value]",
            "state:enum[active||disabled]",
        ] {
            assert!(parse_fields(invalid).is_err(), "{invalid}");
        }
        assert!(matches!(
            parse_fields("state:enum[draft|published]").as_deref(),
            Ok([ResourceField { ty: FieldType::Enum(Some(values)), .. }]) if values == &["draft", "published"]
        ));
    }

    #[test]
    fn validates_resource_identifiers_before_using_them_in_paths_or_sql() {
        assert!(validate_resource_name("Project").is_ok());
        let too_long = "A".repeat(65);
        for invalid in [
            "../Project",
            "project",
            "Project-Admin",
            "Project/../Admin",
            too_long.as_str(),
        ] {
            assert!(validate_resource_name(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn tenant_migration_enforces_explicit_database_context() {
        let sql = render_up_migration(
            "projects",
            &parse_fields("name:string").expect("valid fields"),
            true,
        );
        assert!(sql.contains("ENABLE ROW LEVEL SECURITY"));
        assert!(sql.contains("FORCE ROW LEVEL SECURITY"));
        assert!(sql.contains("current_setting('forge.tenant_id', true)"));
        assert!(sql.contains("WITH CHECK"));
    }

    #[test]
    fn constrained_enum_values_are_enforced_by_postgres() {
        let fields = parse_fields("state:enum[draft|published]").expect("valid enum");
        let sql = render_up_migration("articles", &fields, false);
        assert!(
            sql.contains("\"state\" TEXT NOT NULL CHECK (\"state\" IN ('draft', 'published'))")
        );
        assert!(integration_value(&fields[0], false).contains("draft"));
        assert!(integration_value(&fields[0], true).contains("published"));
        assert!(!sql.contains("DROP"));
    }

    #[test]
    fn generated_global_entity_has_no_tenant_state() {
        let source = render_entity(
            "Project",
            &parse_fields("name:string").expect("valid fields"),
            false,
        );
        assert!(source.contains("pub struct Project"));
        assert!(source.contains("Id<Self>"));
        assert!(source.contains("version: Version"));
        assert!(source.contains("Version::INITIAL"));
        assert!(source.contains("from_persistence"));
        assert!(source.contains("pub fn revise(&mut self, name: String)"));
        assert!(source.contains("self.name = name;"));
        assert!(!source.contains("TenantId"));
    }

    #[test]
    fn generated_entity_module_registration_preserves_existing_content() -> Result<(), CliError> {
        let directory = tempfile::tempdir().map_err(|source| CliError::CreateDirectory {
            path: std::env::temp_dir(),
            source,
        })?;
        let module = directory.path().join("entities.rs");
        fs::write(&module, "//! existing\npub mod existing;\n").map_err(|source| {
            CliError::WriteFile {
                path: module.clone(),
                source,
            }
        })?;
        append_module(&module, "pub mod project;")?;
        let content = fs::read_to_string(&module).map_err(|source| CliError::ReadFile {
            path: module.clone(),
            source,
        })?;
        assert!(content.starts_with("//! existing\npub mod existing;"));
        assert!(content.contains("pub mod project;"));
        Ok(())
    }

    #[test]
    fn module_rollback_removes_only_the_exact_generated_trailing_declaration()
    -> Result<(), CliError> {
        let directory = tempfile::tempdir().map_err(|source| CliError::CreateDirectory {
            path: std::env::temp_dir(),
            source,
        })?;
        let module = directory.path().join("ports.rs");
        fs::write(&module, "//! existing\n").map_err(|source| CliError::WriteFile {
            path: module.clone(),
            source,
        })?;

        append_module(&module, "pub mod project;")?;
        remove_appended_module(&module, "pub mod project;");
        assert_eq!(
            fs::read_to_string(&module).ok().as_deref(),
            Some("//! existing\n")
        );

        append_module(&module, "pub mod project;")?;
        fs::write(
            &module,
            "//! existing\n\npub mod project;\n// concurrent edit\n",
        )
        .map_err(|source| CliError::WriteFile {
            path: module.clone(),
            source,
        })?;
        remove_appended_module(&module, "pub mod project;");
        assert!(
            fs::read_to_string(&module)
                .ok()
                .is_some_and(|contents| contents.contains("// concurrent edit"))
        );
        Ok(())
    }
}
