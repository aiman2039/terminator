//! Preview-first, GUI-owned LSP rename. The server proposes text edits;
//! filesystem workers validate and stage them before an explicit Apply.

mod dialog;
mod staging;
#[cfg(test)]
mod tests;

use super::*;
use anyhow::Context as _;
pub(super) use staging::{apply_rename, prepare_rename};
use std::collections::HashSet;
use std::io::Write as _;

type RenameResult<T> = std::result::Result<T, String>;

const MAX_RENAME_FILES: usize = 64;
const MAX_RENAME_EDITS: usize = 4096;
const MAX_RENAME_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct RenameDialog {
    path: PathBuf,
    root: PathBuf,
    position: native_lsp::Position,
    draft: String,
    stage: RenameStage,
}

enum RenameStage {
    Input,
    WaitingServer,
    Preparing(tokio::sync::mpsc::UnboundedReceiver<RenameResult<PreparedRename>>),
    Preview(PreparedRename),
    Applying {
        rx: tokio::sync::mpsc::UnboundedReceiver<RenameResult<Vec<PathBuf>>>,
        paths: Vec<PathBuf>,
    },
    Error(String),
}

pub(super) struct PreparedRename {
    root: PathBuf,
    files: Vec<PreparedFile>,
    edit_count: usize,
}

struct PreparedFile {
    path: PathBuf,
    original: Vec<u8>,
    changed: Vec<u8>,
    version: Option<i32>,
    edit_count: usize,
    diff: String,
}

impl RenameDialog {
    pub(super) fn applying_to(&self, path: &Path) -> bool {
        matches!(&self.stage, RenameStage::Applying { paths, .. } if paths.iter().any(|item| item == path))
    }
}

impl App {
    pub(super) fn begin_native_rename(
        &mut self,
        path: &Path,
        project: Option<&str>,
        position: native_lsp::Position,
    ) {
        if self
            .native_rename
            .as_ref()
            .is_some_and(|dialog| matches!(dialog.stage, RenameStage::Applying { .. }))
        {
            return;
        }
        let Some(root) = project
            .and_then(|id| self.state.projects.iter().find(|item| item.id == id))
            .map(|item| item.path.clone())
        else {
            self.info = Some("Select a project before renaming a symbol.".into());
            return;
        };
        self.native_rename = Some(RenameDialog {
            path: path.to_owned(),
            root,
            position,
            draft: String::new(),
            stage: RenameStage::Input,
        });
    }

    fn rename_open_docs_are_clean(&self, prepared: &PreparedRename) -> RenameResult<()> {
        for file in &prepared.files {
            if let Some(doc) = self.native_docs.get(&file.path)
                && (doc.dirty() || doc.saving || doc.loading)
            {
                return Err(format!(
                    "Save {} before applying the rename.",
                    file.path.display()
                ));
            }
            if let Some(doc) = self.native_docs.get(&file.path)
                && let Some(buffer) = doc.doc.as_ref()
                && buffer.text().as_bytes() != file.original.as_slice()
            {
                return Err(format!(
                    "{} differs from the loaded editor. Reload it and try again.",
                    file.path.display()
                ));
            }
            if let Some(version) = file.version
                && self.lsp.document_version(&file.path) != Some(version)
            {
                return Err(format!(
                    "{} changed since the preview. Try again.",
                    file.path.display()
                ));
            }
        }
        Ok(())
    }

    fn start_rename_prepare(
        &self,
        root: &Path,
        edits: Vec<native_lsp::RenameFileEdit>,
    ) -> RenameResult<tokio::sync::mpsc::UnboundedReceiver<RenameResult<PreparedRename>>> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let services = self.services.clone();
        let service = services.clone();
        let root = root.to_owned();
        let resource = root.display().to_string();
        services
            .handle()
            .submit(
                OperationContext::new("native-rename-preview", resource, Policy::ReplaceableRead),
                CancellationToken::new(),
                async move {
                    let token = CancellationToken::new();
                    let result = service
                        .fs()
                        .run(&token, move || prepare_rename(&root, edits))
                        .await;
                    let _ = tx.send(result.map_err(|error| format!("{error:#}")));
                    Ok(Vec::new())
                },
            )
            .map_err(|error| format!("Could not prepare rename: {error}"))?;
        Ok(rx)
    }

    fn start_rename_apply(
        &self,
        prepared: PreparedRename,
    ) -> RenameResult<tokio::sync::mpsc::UnboundedReceiver<RenameResult<Vec<PathBuf>>>> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let services = self.services.clone();
        let service = services.clone();
        let resource = prepared.root.display().to_string();
        services
            .handle()
            .submit(
                OperationContext::new("native-rename-apply", resource, Policy::OrderedMutation),
                CancellationToken::new(),
                async move {
                    let token = CancellationToken::new();
                    let result = service
                        .fs()
                        .run(&token, move || apply_rename(prepared))
                        .await;
                    let _ = tx.send(result.map_err(|error| format!("{error:#}")));
                    Ok(Vec::new())
                },
            )
            .map_err(|error| format!("Could not apply rename: {error}"))?;
        Ok(rx)
    }
}
