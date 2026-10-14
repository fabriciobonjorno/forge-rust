//! Generator for small, server-rendered HTML pages with safe text escaping.

use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use crate::{
    CliError,
    preview::{GenerationPreview, PreviewFile},
    resource::{
        append_module, ensure_no_symlink_components, find_application_dir, format_generated_rust,
        remove_appended_module, to_snake_case, validate_resource_name, write_new_file,
    },
};

pub(crate) struct PageGenerator {
    pub(crate) name: String,
}

impl PageGenerator {
    pub(crate) fn preview(&self, current_dir: &Path) -> Result<GenerationPreview, CliError> {
        let (root, files) = self.plan(current_dir)?;
        Ok(GenerationPreview::new(false, files).relative_to(&root))
    }

    pub(crate) fn generate(&self, current_dir: &Path) -> Result<(String, PathBuf), CliError> {
        let (root, files) = self.plan(current_dir)?;
        let mut created = Vec::new();
        for file in files.iter().filter(|file| file.operation == "create") {
            if let Some(parent) = file.path.parent() {
                fs::create_dir_all(parent).map_err(|source| CliError::CreateDirectory {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            if let Err(error) = write_new_file(&file.path, &file.content) {
                for path in &created {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
            created.push(file.path.clone());
        }
        let source_path = root.join(format!(
            "src/adapters/views/{}.rs",
            to_snake_case(&self.name)
        ));
        if let Err(error) = format_generated_rust(&source_path) {
            for path in &created {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }

        let mut appended: Vec<(PathBuf, String)> = Vec::new();
        for file in files.iter().filter(|file| {
            file.operation == "append" && file.path.file_name().is_some_and(|name| name == "mod.rs")
        }) {
            let declaration = file.content.trim();
            if let Err(error) = append_module(&file.path, declaration) {
                for (path, declaration) in appended {
                    remove_appended_module(&path, &declaration);
                }
                for path in &created {
                    let _ = fs::remove_file(path);
                }
                return Err(error);
            }
            appended.push((file.path.clone(), declaration.to_owned()));
        }
        let route_entry = files
            .iter()
            .find(|file| file.operation == "append" && file.path.ends_with("routes.rs"))
            .expect("page plan contains a route registration");
        let route_file = root.join("src/adapters/views/routes.rs");
        if let Err(error) = insert_after_marker(
            &route_file,
            "    // <forge:page-route-registrations>",
            &route_entry.content,
        ) {
            for (path, declaration) in appended {
                remove_appended_module(&path, &declaration);
            }
            for path in &created {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        Ok((self.route_path(), root))
    }

    fn plan(&self, current_dir: &Path) -> Result<(PathBuf, Vec<PreviewFile>), CliError> {
        validate_resource_name(&self.name).map_err(|_| CliError::InvalidPageName {
            name: self.name.clone(),
        })?;
        let root = find_application_dir(current_dir)?;
        let snake = to_snake_case(&self.name);
        let view_path = root.join(format!("src/adapters/views/{snake}.rs"));
        let views_module = root.join("src/adapters/views/mod.rs");
        let routes_module = root.join("src/adapters/views/routes.rs");
        for path in [&view_path, &views_module, &routes_module] {
            ensure_no_symlink_components(&root, path)?;
        }
        for path in [&views_module, &routes_module] {
            let metadata = fs::symlink_metadata(path).map_err(|source| {
                if source.kind() == std::io::ErrorKind::NotFound {
                    CliError::PageScaffoldMissing(path.to_path_buf())
                } else {
                    CliError::ReadFile {
                        path: path.to_path_buf(),
                        source,
                    }
                }
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(CliError::PageScaffoldMissing(path.to_path_buf()));
            }
        }
        let module_contents =
            fs::read_to_string(&views_module).map_err(|source| CliError::ReadFile {
                path: views_module.clone(),
                source,
            })?;
        let declaration = format!("pub mod {snake};");
        if module_contents
            .lines()
            .any(|line| line.trim() == declaration)
            || view_path.exists()
        {
            return Err(CliError::DestinationExists(view_path));
        }
        let route_registration = format!("crate::adapters::views::{snake}::register(router)?;");
        let route_contents =
            fs::read_to_string(&routes_module).map_err(|source| CliError::ReadFile {
                path: routes_module.clone(),
                source,
            })?;
        let marker = "    // <forge:page-route-registrations>";
        if route_contents.matches(marker).count() != 1 {
            return Err(CliError::PageScaffoldMissing(routes_module));
        }
        if route_contents.contains(&route_registration) {
            return Err(CliError::DestinationExists(routes_module));
        }

        Ok((
            root,
            vec![
                PreviewFile {
                    path: view_path,
                    operation: "create",
                    content: render_page(&self.name, &self.route_path()),
                },
                PreviewFile {
                    path: views_module,
                    operation: "append",
                    content: declaration,
                },
                PreviewFile {
                    path: routes_module,
                    operation: "append",
                    content: route_registration,
                },
            ],
        ))
    }

    fn route_path(&self) -> String {
        format!("/{}", to_snake_case(&self.name).replace('_', "-"))
    }
}

fn insert_after_marker(path: &Path, marker: &str, insertion: &str) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| CliError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CliError::PageScaffoldMissing(path.to_path_buf()));
    }
    let original = fs::read_to_string(path).map_err(|source| CliError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    if original.matches(marker).count() != 1 || original.contains(insertion) {
        return Err(CliError::PageScaffoldMissing(path.to_path_buf()));
    }
    let replacement = format!("{marker}\n    {insertion}");
    let updated = original.replacen(marker, &replacement, 1);
    let temporary = path.with_extension("rs.forge-tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                CliError::DestinationExists(temporary.clone())
            } else {
                CliError::WriteFile {
                    path: temporary.clone(),
                    source,
                }
            }
        })?;
    if let Err(source) = fs::set_permissions(&temporary, metadata.permissions()) {
        let _ = fs::remove_file(&temporary);
        return Err(CliError::WriteFile {
            path: temporary,
            source,
        });
    }
    if let Err(source) = file
        .write_all(updated.as_bytes())
        .and_then(|()| file.sync_all())
    {
        let _ = fs::remove_file(&temporary);
        return Err(CliError::WriteFile {
            path: temporary,
            source,
        });
    }
    fs::rename(&temporary, path).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        CliError::WriteFile {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn render_page(name: &str, path: &str) -> String {
    let title = name
        .chars()
        .enumerate()
        .fold(String::new(), |mut title, (index, character)| {
            if character.is_ascii_uppercase() && index > 0 {
                title.push(' ');
            }
            title.push(character);
            title
        });
    let title = title.trim_end_matches(" Page").to_owned();
    let escaped_title = title.replace('"', "\\\"");
    format!(
        "//! Server-rendered page for the `{name}` feature.\n\n\
         use forge::http::{{Method, Request, Response, RouteError, Router, StatusCode}};\n\n\
         /// Registers `GET {path}`. The generated views route table calls this automatically.\n\
         pub fn register(router: &mut Router) -> Result<(), RouteError> {{\n\
             router.route(Method::GET, \"{path}\", show)?;\n\
             Ok(())\n\
         }}\n\n\
         async fn show(_request: Request) -> Response {{\n\
             forge::web::html(StatusCode::OK, render(\"{escaped_title}\", \"Welcome to your {escaped_title} page.\"))\n\
         }}\n\n\
         /// Renders plain text values in a semantic, responsive HTML document.\n\
         /// Values are escaped as text; this function does not accept raw markup.\n\
         #[must_use]\n\
         pub fn render(title: &str, content: &str) -> String {{\n\
             format!(\n\
                 \"<!doctype html><html lang=\\\"en\\\"><head><meta charset=\\\"utf-8\\\"><meta name=\\\"viewport\\\" content=\\\"width=device-width, initial-scale=1\\\"><title>{{}}</title></head><body><main><h1>{{}}</h1><p>{{}}</p></main></body></html>\",\n\
                 forge::web::escape_html(title),\n\
                 forge::web::escape_html(title),\n\
                 forge::web::escape_html(content),\n\
             )\n\
         }}\n",
    )
}
