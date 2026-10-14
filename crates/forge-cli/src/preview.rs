use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct GenerationPreview {
    pub(crate) schema_version: u8,
    pub(crate) dry_run: bool,
    pub(crate) paths_are_provisional: bool,
    pub(crate) summary: PreviewSummary,
    pub(crate) files: Vec<PreviewFile>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PreviewSummary {
    pub(crate) creates: usize,
    pub(crate) appends: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct PreviewFile {
    pub(crate) path: PathBuf,
    pub(crate) operation: &'static str,
    pub(crate) content: String,
}

impl GenerationPreview {
    pub(crate) fn new(paths_are_provisional: bool, files: Vec<PreviewFile>) -> Self {
        let summary = PreviewSummary {
            creates: files
                .iter()
                .filter(|file| file.operation == "create")
                .count(),
            appends: files
                .iter()
                .filter(|file| file.operation == "append")
                .count(),
        };
        Self {
            schema_version: 1,
            dry_run: true,
            paths_are_provisional,
            summary,
            files,
        }
    }

    pub(crate) fn relative_to(mut self, root: &Path) -> Self {
        for file in &mut self.files {
            if let Ok(path) = file.path.strip_prefix(root) {
                file.path = path.to_path_buf();
            }
        }
        self
    }
}
