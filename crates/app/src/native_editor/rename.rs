//! Preview-first, GUI-owned LSP rename. The server proposes text edits;
//! filesystem workers validate and stage them before an explicit Apply.

use super::*;
use anyhow::Context as _;
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

struct PreparedRename {
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

    pub(crate) fn show_native_rename_dialog(&mut self, ctx: &egui::Context) {
        self.lsp.poll();
        let Some(mut dialog) = self.native_rename.take() else {
            return;
        };

        if matches!(dialog.stage, RenameStage::WaitingServer)
            && let Some(result) = self.lsp.take_rename()
            && result.from == dialog.path
            && result.new_name == dialog.draft.trim()
        {
            dialog.stage = match result.edits {
                Ok(edits) if !edits.is_empty() => {
                    match self.start_rename_prepare(&dialog.root, edits) {
                        Ok(rx) => RenameStage::Preparing(rx),
                        Err(error) => RenameStage::Error(error),
                    }
                }
                Ok(_) => RenameStage::Error("The language server found no rename edits.".into()),
                Err(error) => RenameStage::Error(error),
            };
        }
        let settled = match &mut dialog.stage {
            RenameStage::Preparing(rx) => match rx.try_recv() {
                Ok(Ok(prepared)) if !prepared.files.is_empty() => {
                    Some(match self.rename_open_docs_are_clean(&prepared) {
                        Ok(()) => RenameStage::Preview(prepared),
                        Err(error) => RenameStage::Error(error),
                    })
                }
                Ok(Ok(_)) => Some(RenameStage::Error(
                    "The rename would not change any file.".into(),
                )),
                Ok(Err(error)) => Some(RenameStage::Error(error)),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    Some(RenameStage::Error("Rename preview worker stopped.".into()))
                }
            },
            RenameStage::Applying { rx, .. } => match rx.try_recv() {
                Ok(result) => Some(match result {
                    Ok(paths) => {
                        let mut retained_dirty = false;
                        for path in &paths {
                            if let Some(doc) = self.native_docs.get_mut(path) {
                                if doc.dirty() {
                                    retained_dirty = true;
                                    doc.save_error = Some("Rename changed the file on disk while this buffer had edits. Review it before saving.".into());
                                } else {
                                    doc.doc = None;
                                    doc.load_rx = None;
                                    doc.loading = false;
                                    doc.load_error = None;
                                }
                            }
                        }
                        self.lsp.invalidate_paths(&paths);
                        self.info = Some(if retained_dirty {
                            "Rename applied, but an edited buffer was kept for review.".into()
                        } else {
                            format!("Renamed symbol in {} file(s).", paths.len())
                        });
                        RenameStage::Input
                    }
                    Err(error) => RenameStage::Error(error),
                }),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    Some(RenameStage::Error(
                        "Rename apply worker stopped; check the affected files.".into(),
                    ))
                }
            },
            _ => None,
        };
        if let Some(stage) = settled {
            if matches!(dialog.stage, RenameStage::Applying { .. })
                && matches!(stage, RenameStage::Input)
            {
                return;
            }
            dialog.stage = stage;
        }

        let applying = matches!(dialog.stage, RenameStage::Applying { .. });
        let mut open = true;
        let mut request = false;
        let mut apply = false;
        let mut retry = false;
        let added_color = appearance::color(&self.theme.git_added);
        let removed_color = appearance::color(&self.theme.status_failed);
        let context_color = appearance::color(&self.theme.secondary);
        let window = self.popups.window(ctx, "Rename symbol").collapsible(false);
        let window = if applying {
            window
        } else {
            window.open(&mut open)
        };
        window.show(ctx, |ui| {
            ui.weak(dialog.path.display().to_string());
            match &mut dialog.stage {
                RenameStage::Input => {
                    ui.horizontal(|ui| {
                        ui.label("New name");
                        let field = ui.text_edit_singleline(&mut dialog.draft);
                        #[cfg(feature = "test-support")]
                        crate::diagnostics::record(ui.ctx(), "rename-name", field.rect);
                        if field.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            request = true;
                        }
                    });
                    let preview = ui.button("Preview changes");
                    #[cfg(feature = "test-support")]
                    crate::diagnostics::record(ui.ctx(), "rename-preview", preview.rect);
                    if preview.clicked() {
                        request = true;
                    }
                }
                RenameStage::WaitingServer => {
                    ui.spinner();
                    ui.weak("Waiting for the language server…");
                }
                RenameStage::Preparing(_) => {
                    ui.spinner();
                    ui.weak("Checking files and preparing the preview…");
                }
                RenameStage::Preview(prepared) => {
                    ui.label(format!(
                        "{} edits in {} files. Review before applying.",
                        prepared.edit_count,
                        prepared.files.len()
                    ));
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .show(ui, |ui| {
                            for file in &prepared.files {
                                let relative =
                                    file.path.strip_prefix(&prepared.root).unwrap_or(&file.path);
                                egui::CollapsingHeader::new(format!(
                                    "{} ({} edits)",
                                    relative.display(),
                                    file.edit_count
                                ))
                                .default_open(true)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    for line in file.diff.lines() {
                                        let color = if line.starts_with('+') {
                                            added_color
                                        } else if line.starts_with('-') {
                                            removed_color
                                        } else {
                                            context_color
                                        };
                                        ui.label(
                                            egui::RichText::new(line).monospace().color(color),
                                        );
                                    }
                                });
                            }
                        });
                    let apply_button = ui.button("Apply rename");
                    #[cfg(feature = "test-support")]
                    crate::diagnostics::record(ui.ctx(), "rename-apply", apply_button.rect);
                    if apply_button.clicked() {
                        apply = true;
                    }
                }
                RenameStage::Applying { .. } => {
                    ui.spinner();
                    ui.weak("Applying the reviewed changes…");
                }
                RenameStage::Error(error) => {
                    ui.colored_label(appearance::color(&self.theme.status_failed), error);
                    if ui.button("Edit name and retry").clicked() {
                        retry = true;
                    }
                }
            }
        });
        if !open {
            return;
        }
        if retry {
            dialog.stage = RenameStage::Input;
        }
        if request {
            let name = dialog.draft.trim();
            if name.is_empty() {
                dialog.stage = RenameStage::Error("Enter a new symbol name.".into());
            } else if self
                .native_docs
                .get(&dialog.path)
                .is_some_and(NativeDoc::dirty)
            {
                dialog.stage =
                    RenameStage::Error("Save this file before renaming a symbol.".into());
            } else {
                dialog.stage = match self.lsp.request_rename(&dialog.path, dialog.position, name) {
                    Ok(()) => RenameStage::WaitingServer,
                    Err(error) => RenameStage::Error(error),
                };
            }
        }
        if apply {
            let preview = std::mem::replace(&mut dialog.stage, RenameStage::Input);
            if let RenameStage::Preview(prepared) = preview {
                let paths = prepared
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect();
                dialog.stage = match self
                    .rename_open_docs_are_clean(&prepared)
                    .and_then(|()| self.start_rename_apply(prepared))
                {
                    Ok(rx) => RenameStage::Applying { rx, paths },
                    Err(error) => RenameStage::Error(error),
                };
            }
        }
        self.native_rename = Some(dialog);
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

fn checked_path(root: &Path, canonical_root: &Path, uri: &str) -> anyhow::Result<PathBuf> {
    let path = native_lsp::uri_to_path(uri)
        .ok_or_else(|| anyhow::anyhow!("Rename target is not a local file: {uri}"))?;
    let relative = path
        .strip_prefix(root)
        .context("Rename target is outside the project")?;
    anyhow::ensure!(
        !relative.as_os_str().is_empty(),
        "Rename target is the project directory"
    );
    let metadata = std::fs::symlink_metadata(&path)?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Rename target is not a regular file"
    );
    let canonical = std::fs::canonicalize(&path)?;
    anyhow::ensure!(
        canonical.starts_with(canonical_root) && canonical == canonical_root.join(relative),
        "Rename target traverses a symlink or leaves the project"
    );
    Ok(path)
}

fn byte_offset(text: &str, position: native_lsp::Position) -> anyhow::Result<usize> {
    let mut start = 0usize;
    for _ in 0..position.line {
        let next = text
            .get(start..)
            .context("Rename edit starts inside a UTF-8 character")?
            .find('\n')
            .ok_or_else(|| anyhow::anyhow!("Rename edit line is out of range"))?;
        start = start.saturating_add(next).saturating_add(1);
    }
    let end = text
        .get(start..)
        .context("Rename edit starts inside a UTF-8 character")?
        .find('\n')
        .map_or(text.len(), |next| start.saturating_add(next));
    let row = text
        .get(start..end)
        .context("Rename edit range is not UTF-8 aligned")?;
    let mut units = 0u32;
    for (at, character) in row.char_indices() {
        if units == position.character {
            return Ok(start.saturating_add(at));
        }
        units = units.saturating_add(u32::try_from(character.len_utf16()).unwrap_or(2));
        anyhow::ensure!(
            units <= position.character,
            "Rename edit splits a UTF-16 character"
        );
    }
    anyhow::ensure!(
        units == position.character,
        "Rename edit column is out of range"
    );
    Ok(end)
}

fn edited_text(text: &str, edits: &[native_lsp::RenameTextEdit]) -> anyhow::Result<String> {
    let mut spans = Vec::with_capacity(edits.len());
    for edit in edits {
        let start = byte_offset(text, edit.range.start)?;
        let end = byte_offset(text, edit.range.end)?;
        anyhow::ensure!(start <= end, "Rename edit has a reversed range");
        spans.push((start, end, &edit.new_text));
    }
    spans.sort_by_key(|(start, end, _)| (*start, *end));
    for pair in spans.windows(2) {
        let [first, second] = pair else { continue };
        let (start, end, _) = *first;
        let (next_start, next_end, _) = *second;
        anyhow::ensure!(
            end <= next_start && !(start == end && next_start == next_end && start == next_start),
            "Rename edits overlap"
        );
    }
    let mut changed = text.to_owned();
    for (start, end, replacement) in spans.into_iter().rev() {
        changed.replace_range(start..end, replacement);
    }
    Ok(changed)
}

fn prepare_rename(
    root: &Path,
    edits: Vec<native_lsp::RenameFileEdit>,
) -> anyhow::Result<PreparedRename> {
    anyhow::ensure!(
        !edits.is_empty() && edits.len() <= MAX_RENAME_FILES,
        "Rename exceeds the file limit"
    );
    let canonical_root = std::fs::canonicalize(root)?;
    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;
    let mut edit_count = 0usize;
    let mut files = Vec::new();
    for item in edits {
        let path = checked_path(root, &canonical_root, &item.uri)?;
        anyhow::ensure!(seen.insert(path.clone()), "Rename lists a file twice");
        edit_count = edit_count.saturating_add(item.edits.len());
        anyhow::ensure!(
            edit_count <= MAX_RENAME_EDITS,
            "Rename exceeds the edit limit"
        );
        let original = std::fs::read(&path)?;
        total_bytes = total_bytes.saturating_add(original.len());
        anyhow::ensure!(
            u64::try_from(original.len()).unwrap_or(u64::MAX) <= MAX_NATIVE_BYTES
                && total_bytes <= MAX_RENAME_BYTES,
            "Rename contains a file that is too large"
        );
        let text = std::str::from_utf8(&original)?;
        let changed = edited_text(text, &item.edits)?;
        anyhow::ensure!(
            u64::try_from(changed.len()).unwrap_or(u64::MAX) <= MAX_NATIVE_BYTES,
            "Rename result is too large"
        );
        if changed.as_bytes() == original {
            continue;
        }
        let mut diff = similar::TextDiff::from_lines(text, &changed)
            .unified_diff()
            .context_radius(2)
            .to_string();
        if diff.len() > 100_000 {
            let end = diff.floor_char_boundary(100_000);
            diff.truncate(end);
            diff.push_str("\n… preview truncated");
        }
        files.push(PreparedFile {
            path,
            original,
            changed: changed.into_bytes(),
            version: item.version,
            edit_count: item.edits.len(),
            diff,
        });
    }
    Ok(PreparedRename {
        root: root.to_owned(),
        files,
        edit_count,
    })
}

fn staged_file(
    path: &Path,
    bytes: &[u8],
    permissions: &std::fs::Permissions,
) -> anyhow::Result<tempfile::NamedTempFile> {
    let parent = path.parent().context("Rename target has no parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().set_permissions(permissions.clone())?;
    file.as_file().sync_all()?;
    Ok(file)
}

fn apply_rename(prepared: PreparedRename) -> anyhow::Result<Vec<PathBuf>> {
    let canonical_root = std::fs::canonicalize(&prepared.root)?;
    let mut stages = Vec::new();
    for file in &prepared.files {
        let uri = native_lsp::file_uri(&file.path).context("Invalid rename path")?;
        let path = checked_path(&prepared.root, &canonical_root, uri.as_str())?;
        anyhow::ensure!(
            std::fs::read(&path)? == file.original,
            "{} changed since the preview",
            path.display()
        );
        let permissions = std::fs::metadata(&path)?.permissions();
        let new = staged_file(&path, &file.changed, &permissions)?;
        let backup = staged_file(&path, &file.original, &permissions)?;
        stages.push((path, new, backup));
    }
    let mut committed: Vec<(PathBuf, tempfile::NamedTempFile)> = Vec::new();
    for (path, new, backup) in stages {
        let current = prepared
            .files
            .iter()
            .find(|file| file.path == path)
            .context("Missing rename snapshot")?;
        let result = (|| -> anyhow::Result<()> {
            let uri = native_lsp::file_uri(&path).context("Invalid rename path")?;
            checked_path(&prepared.root, &canonical_root, uri.as_str())?;
            anyhow::ensure!(
                std::fs::read(&path)? == current.original,
                "{} changed during the rename",
                path.display()
            );
            new.persist(&path).map_err(|error| error.error)?;
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for (previous, old) in committed.into_iter().rev() {
                let expected = prepared.files.iter().find(|file| file.path == previous);
                if expected.is_none_or(|file| {
                    std::fs::read(&previous).ok().as_deref() != Some(file.changed.as_slice())
                }) {
                    rollback_errors.push(format!(
                        "{} changed after apply; manual recovery is needed",
                        previous.display()
                    ));
                    continue;
                }
                if let Err(restore) = old.persist(&previous) {
                    rollback_errors.push(format!("{}: {}", previous.display(), restore.error));
                }
            }
            if rollback_errors.is_empty() {
                return Err(error);
            }
            anyhow::bail!(
                "{error:#}; rollback also failed for {}",
                rollback_errors.join(", ")
            );
        }
        committed.push((path, backup));
    }
    Ok(committed.into_iter().map(|(path, _)| path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(
        path: &Path,
        line: u32,
        start: u32,
        end: u32,
        replacement: &str,
    ) -> native_lsp::RenameFileEdit {
        native_lsp::RenameFileEdit {
            uri: native_lsp::file_uri(path).unwrap().to_string(),
            version: None,
            edits: vec![native_lsp::RenameTextEdit {
                range: native_lsp::Range {
                    start: native_lsp::Position {
                        line,
                        character: start,
                    },
                    end: native_lsp::Position {
                        line,
                        character: end,
                    },
                },
                new_text: replacement.into(),
            }],
        }
    }

    #[test]
    fn reviewed_multifile_rename_uses_utf16_and_changes_both_files() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.rs");
        let b = root.path().join("b.rs");
        std::fs::write(&a, "let 🦀 = foo;\n").unwrap();
        std::fs::write(&b, "foo();\n").unwrap();
        let preview = prepare_rename(
            root.path(),
            vec![edit(&a, 0, 9, 12, "bar"), edit(&b, 0, 0, 3, "bar")],
        )
        .unwrap();
        assert_eq!(preview.files.len(), 2);
        assert!(preview.files[0].diff.contains("bar"));
        assert_eq!(apply_rename(preview).unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(a).unwrap(), "let 🦀 = bar;\n");
        assert_eq!(std::fs::read_to_string(b).unwrap(), "bar();\n");
    }

    #[test]
    fn stale_disk_file_stops_rename_before_any_file_changes() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.rs");
        let b = root.path().join("b.rs");
        std::fs::write(&a, "foo\n").unwrap();
        std::fs::write(&b, "foo\n").unwrap();
        let preview = prepare_rename(
            root.path(),
            vec![edit(&a, 0, 0, 3, "bar"), edit(&b, 0, 0, 3, "bar")],
        )
        .unwrap();
        std::fs::write(&b, "other\n").unwrap();
        assert!(apply_rename(preview).is_err());
        assert_eq!(std::fs::read_to_string(a).unwrap(), "foo\n");
        assert_eq!(std::fs::read_to_string(b).unwrap(), "other\n");
    }

    #[test]
    fn overlapping_or_outside_edits_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let a = root.path().join("a.rs");
        let b = outside.path().join("b.rs");
        std::fs::write(&a, "foobar\n").unwrap();
        std::fs::write(&b, "foo\n").unwrap();
        let mut same = edit(&a, 0, 0, 3, "bar");
        same.edits.extend(edit(&a, 0, 2, 5, "baz").edits);
        assert!(prepare_rename(root.path(), vec![same]).is_err());
        assert!(prepare_rename(root.path(), vec![edit(&b, 0, 0, 3, "bar")]).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_rename_target_is_rejected() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("target.rs");
        let link = root.path().join("link.rs");
        std::fs::write(&target, "foo\n").unwrap();
        symlink(&target, &link).unwrap();
        assert!(prepare_rename(root.path(), vec![edit(&link, 0, 0, 3, "bar")]).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "foo\n");
    }
}
