//! Filesystem staging for a reviewed rename: validate edits, build the
//! preview, and write every file or none.
use super::*;

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

pub(in crate::native_editor) fn prepare_rename(
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

pub(in crate::native_editor) fn apply_rename(
    prepared: PreparedRename,
) -> anyhow::Result<Vec<PathBuf>> {
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
