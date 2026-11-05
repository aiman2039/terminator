//! Replacement text, and the worker that turns a replace plan into
//! checked edits. A line's matches are found as the search found them
//! (empty matches skipped, like the find bar); in regex mode `$1` and
//! `${name}` expand from the match. The worker re-reads each file, makes
//! sure every planned line still reads as the search saw it, and hands
//! the edits to the rename staging.
use super::super::search::MAX_PREVIEW_CHARS;
use super::*;
use crate::native_editor::rename::{MAX_RENAME_BYTES, MAX_RENAME_EDITS, prepare_rename};
use anyhow::Context as _;
use std::ops::Range;

/// How one line is replaced: the search regex, the replacement, and
/// whether `$1` groups expand (regex mode) or the text goes in as typed.
pub(crate) struct LineReplace<'a> {
    pub regex: &'a regex::Regex,
    pub replacement: &'a str,
    pub expand: bool,
}

/// The non-empty matches of `line` (byte ranges), each with its new text.
pub(crate) fn line_replacements(line: &str, how: &LineReplace<'_>) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    for captures in how.regex.captures_iter(line) {
        let Some(found) = captures.get(0).filter(|found| !found.is_empty()) else {
            continue;
        };
        let mut text = String::new();
        if how.expand {
            captures.expand(how.replacement, &mut text);
        } else {
            text.push_str(how.replacement);
        }
        out.push((found.range(), text));
    }
    out
}

/// One line after the replace, with the char ranges of the matches in
/// the old line and of their replacements in the new one.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LineChange {
    pub new: String,
    pub old_marks: Vec<(usize, usize)>,
    pub new_marks: Vec<(usize, usize)>,
}

pub(crate) fn change_line(line: &str, how: &LineReplace<'_>) -> LineChange {
    let mut change = LineChange {
        new: String::with_capacity(line.len()),
        old_marks: Vec::new(),
        new_marks: Vec::new(),
    };
    let (mut byte, mut old_at, mut new_at) = (0usize, 0usize, 0usize);
    for (range, text) in line_replacements(line, how) {
        let gap = line.get(byte..range.start).unwrap_or_default();
        let found = line.get(range.clone()).unwrap_or_default();
        let gap_chars = gap.chars().count();
        change.new.push_str(gap);
        change.new.push_str(&text);
        let old_start = old_at.saturating_add(gap_chars);
        let new_start = new_at.saturating_add(gap_chars);
        old_at = old_start.saturating_add(found.chars().count());
        new_at = new_start.saturating_add(text.chars().count());
        change.old_marks.push((old_start, old_at));
        change.new_marks.push((new_start, new_at));
        byte = range.end;
    }
    change.new.push_str(line.get(byte..).unwrap_or_default());
    change
}

/// One file of a replace: its counted 0-based lines, each with its text
/// as the search saw it (cut like the preview).
pub(crate) struct PlannedFile {
    pub path: PathBuf,
    pub lines: Vec<(u32, String)>,
}

/// Everything a replace needs off the GUI thread.
pub(crate) struct ReplacePlan {
    pub root: PathBuf,
    /// The query the results came from.
    pub query: crate::search::Query,
    pub replacement: String,
    pub files: Vec<PlannedFile>,
}

/// A planned line no longer reads as the search saw it.
#[derive(Debug)]
pub(crate) struct Stale(pub String);

impl std::fmt::Display for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Stale {}

/// Read the planned files, check them, and stage their edits through the
/// rename staging. A file that changed since the search fails with
/// [`Stale`].
pub(crate) fn prepare_replace(plan: &ReplacePlan) -> anyhow::Result<PreparedRename> {
    let regex = crate::search::build_typed_regex(&plan.query).map_err(anyhow::Error::msg)?;
    let how = LineReplace {
        regex: &regex,
        replacement: &plan.replacement,
        expand: plan.query.use_regex,
    };
    let (mut edits, mut read) = (Vec::new(), Vec::new());
    let (mut total_bytes, mut total_edits) = (0usize, 0usize);
    for file in &plan.files {
        let name = crate::workspace_ops::relative_display(&plan.root, &file.path);
        // A read-only file (Perforce: not opened for edit) is not ours to write.
        let metadata =
            std::fs::metadata(&file.path).with_context(|| format!("Could not read {name}"))?;
        anyhow::ensure!(
            !metadata.permissions().readonly(),
            "{name} is read-only. Make it writable (Perforce: p4 edit) and try again."
        );
        let bytes = std::fs::read(&file.path).with_context(|| format!("Could not read {name}"))?;
        total_bytes = total_bytes.saturating_add(bytes.len());
        anyhow::ensure!(
            u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= MAX_NATIVE_BYTES,
            "{name} is larger than 1 MB. Uncheck it and try again."
        );
        anyhow::ensure!(
            total_bytes <= MAX_RENAME_BYTES,
            "Replace in Files reads at most 16 MB at once. Uncheck some files and try again."
        );
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("{name} is not UTF-8 text. Uncheck it and try again."))?;
        let file_edits = file_edits(text, &file.lines, &how)
            .ok_or_else(|| Stale(format!("{name} changed since the search.")))?;
        total_edits = total_edits.saturating_add(file_edits.len());
        anyhow::ensure!(
            total_edits <= MAX_RENAME_EDITS,
            "Replace in Files changes at most {MAX_RENAME_EDITS} matches at once. \
             Uncheck some results and try again."
        );
        if file_edits.is_empty() {
            continue;
        }
        let uri = native_lsp::file_uri(&file.path)
            .with_context(|| format!("{name} is not a local file"))?;
        edits.push(native_lsp::RenameFileEdit {
            uri: uri.to_string(),
            version: None,
            edits: file_edits,
        });
        read.push((file.path.as_path(), bytes));
    }
    anyhow::ensure!(
        !edits.is_empty(),
        "Nothing to replace: every match is empty."
    );
    let prepared = prepare_rename(&plan.root, edits).context("Replace in Files stopped")?;
    // The staging read each file again: it must be what the edits came from.
    for (path, bytes) in &read {
        if prepared
            .original(path)
            .is_some_and(|original| original != bytes.as_slice())
        {
            let name = crate::workspace_ops::relative_display(&plan.root, path);
            return Err(Stale(format!("{name} changed while the replace read it.")).into());
        }
    }
    Ok(prepared)
}

/// Edits that replace the matches on `lines` (sorted) of `text`, as
/// language-server ranges. `None` when a planned line is gone or reads
/// otherwise than the search saw it. Line breaks split as `str::lines`.
fn file_edits(
    text: &str,
    lines: &[(u32, String)],
    how: &LineReplace<'_>,
) -> Option<Vec<native_lsp::RenameTextEdit>> {
    let mut wanted = lines.iter().peekable();
    let mut edits = Vec::new();
    for (index, piece) in text.split_inclusive('\n').enumerate() {
        let Some((line, seen)) =
            wanted.next_if(|(line, _)| usize::try_from(*line).is_ok_and(|line| line == index))
        else {
            continue;
        };
        let body = piece
            .strip_suffix('\n')
            .map_or(piece, |body| body.strip_suffix('\r').unwrap_or(body));
        if !body.chars().take(MAX_PREVIEW_CHARS).eq(seen.chars()) {
            return None;
        }
        for (range, new_text) in line_replacements(body, how) {
            let at = |byte: usize| native_lsp::Position {
                line: *line,
                character: utf16_len(body.get(..byte).unwrap_or_default()),
            };
            edits.push(native_lsp::RenameTextEdit {
                range: native_lsp::Range {
                    start: at(range.start),
                    end: at(range.end),
                },
                new_text,
            });
        }
        if wanted.peek().is_none() {
            break;
        }
    }
    wanted.peek().is_none().then_some(edits)
}

fn utf16_len(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
}
