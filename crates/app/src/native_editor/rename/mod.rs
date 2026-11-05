//! Preview-first, GUI-owned LSP rename. The server proposes text edits;
//! filesystem workers validate and stage them before an explicit Apply.
//! Replace in Files writes through the same staging and buffer checks.

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

pub(super) const MAX_RENAME_FILES: usize = 64;
pub(super) const MAX_RENAME_EDITS: usize = 4096;
pub(super) const MAX_RENAME_BYTES: usize = 16 * 1024 * 1024;

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

impl PreparedRename {
    /// Files the change rewrites (unchanged files dropped out).
    pub(super) fn file_count(&self) -> usize {
        self.files.len()
    }

    pub(super) fn edit_count(&self) -> usize {
        self.edit_count
    }

    /// The bytes `path` had when the change was prepared.
    pub(super) fn original(&self, path: &Path) -> Option<&[u8]> {
        self.files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.original.as_slice())
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
        self.open_docs_are_clean(prepared, "applying the rename")
    }

    /// An open native buffer of `path` blocks a write while it has unsaved
    /// edits or a load or save runs (`action` ends "Save it before …"),
    /// and when it was loaded from other bytes than `original`. The buffer
    /// is compared as a save writes it: its encoding and byte-order mark.
    pub(super) fn open_doc_is_clean(
        &self,
        path: &Path,
        original: Option<&[u8]>,
        action: &str,
    ) -> RenameResult<()> {
        let Some(doc) = self.native_docs.get(path) else {
            return Ok(());
        };
        if doc.dirty() || doc.saving || doc.loading {
            return Err(format!("Save {} before {action}.", path.display()));
        }
        if let Some(original) = original
            && let Some(text) = doc.disk_text()
            && doc.encoding.encode(&text).ok().as_deref() != Some(original)
        {
            return Err(format!(
                "{} differs from the loaded editor. Reload it and try again.",
                path.display()
            ));
        }
        Ok(())
    }

    /// Every open buffer of `prepared`'s files passes
    /// [`Self::open_doc_is_clean`], and language-server versions still
    /// match.
    pub(super) fn open_docs_are_clean(
        &self,
        prepared: &PreparedRename,
        action: &str,
    ) -> RenameResult<()> {
        for file in &prepared.files {
            self.open_doc_is_clean(&file.path, Some(&file.original), action)?;
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

    /// Write a checked change set on the file pool, every file or none.
    pub(super) fn start_rename_apply(
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
