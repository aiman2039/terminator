//! Review notes on diff lines, sent to an agent in one batch. GUI-only
//! state: notes are kept per repository and are lost when the GUI exits.
//! They never change Git state.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One note on file lines (1-based, inclusive).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Note {
    /// Absolute path of the file the note is about.
    pub path: PathBuf,
    pub first: usize,
    pub last: usize,
    /// The noted change only removes lines; `first` is where they were.
    pub removed: bool,
    pub text: String,
}

/// A note being typed in the diff's notice bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Draft {
    pub first: usize,
    pub last: usize,
    pub removed: bool,
    pub text: String,
    /// Give the field keyboard focus on its first frame.
    pub focus: bool,
}

impl Draft {
    pub(crate) fn new(first: usize, last: usize, removed: bool) -> Self {
        Self {
            first: first.max(1),
            last: last.max(first).max(1),
            removed,
            text: String::new(),
            focus: true,
        }
    }

    /// `L4` or `L4-9`.
    pub(crate) fn lines(&self) -> String {
        lines_label(self.first, self.last)
    }

    /// The note for `path`, unless the text is blank.
    pub(crate) fn finish(&self, path: &Path) -> Option<Note> {
        let text = one_line(&self.text);
        (!text.is_empty()).then(|| Note {
            path: path.to_path_buf(),
            first: self.first,
            last: self.last,
            removed: self.removed,
            text,
        })
    }
}

fn lines_label(first: usize, last: usize) -> String {
    if first == last {
        format!("L{first}")
    } else {
        format!("L{first}-{last}")
    }
}

/// Notes by repository root, in the order they were added.
#[derive(Default)]
pub(crate) struct Notes {
    by_repo: HashMap<PathBuf, Vec<Note>>,
}

impl Notes {
    pub(crate) fn add(&mut self, repo: &Path, note: Note) {
        self.by_repo
            .entry(repo.to_path_buf())
            .or_default()
            .push(note);
    }

    pub(crate) fn list(&self, repo: &Path) -> &[Note] {
        self.by_repo.get(repo).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn remove(&mut self, repo: &Path, index: usize) {
        if let Some(notes) = self.by_repo.get_mut(repo)
            && index < notes.len()
        {
            notes.remove(index);
        }
    }

    pub(crate) fn clear(&mut self, repo: &Path) {
        self.by_repo.remove(repo);
    }
}

/// Text on one line: line breaks and other control characters become
/// spaces, so a paste never holds an Enter.
pub(crate) fn one_line(text: &str) -> String {
    text.split(char::is_control)
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

/// One line for the agent: each note with its `@path#L…` reference
/// relative to `root` (the agent's project).
pub(crate) fn batch_text(notes: &[Note], root: Option<&Path>) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let items: Vec<String> = notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let reference =
                crate::native_editor::agent_reference(&note.path, root, note.first, note.last);
            let removed = if note.removed { " (removed lines)" } else { "" };
            format!(
                "{}) {}{removed}: {}",
                index.saturating_add(1),
                one_line(&reference),
                note.text
            )
        })
        .collect();
    format!("Review notes: {}", items.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, lines: (usize, usize), text: &str) -> Note {
        Note {
            path: path.into(),
            first: lines.0,
            last: lines.1,
            removed: false,
            text: text.into(),
        }
    }

    #[test]
    fn batch_lists_every_note_with_a_project_relative_reference() {
        let mut removed = note("/w/app/src/b.rs", (12, 12), "why removed?");
        removed.removed = true;
        let notes = [note("/w/app/src/a.rs", (4, 9), "rename this"), removed];
        assert_eq!(
            batch_text(&notes, Some(Path::new("/w/app"))),
            "Review notes: 1) @src/a.rs#L4-9: rename this; \
             2) @src/b.rs#L12 (removed lines): why removed?"
        );
        assert_eq!(batch_text(&[], None), "");
    }

    #[test]
    fn a_multi_line_note_becomes_one_line_so_no_enter_is_pasted() {
        let draft = Draft {
            text: "first line\nsecond\r\nthird\tend".into(),
            ..Draft::new(3, 3, false)
        };
        let note = draft.finish(Path::new("/w/a.rs")).unwrap();
        assert_eq!(note.text, "first line second third end");
        let text = batch_text(&[note], Some(Path::new("/w")));
        let bytes = egui_term::paste_input(&text, egui_term::TerminalMode::empty());
        assert!(!bytes.contains(&b'\r') && !bytes.contains(&b'\n'));
        let blank = Draft {
            text: " \n ".into(),
            ..Draft::new(1, 1, false)
        };
        assert!(blank.finish(Path::new("/w/a.rs")).is_none());
    }

    #[test]
    fn notes_are_kept_per_repository_and_removed_one_by_one() {
        let mut notes = Notes::default();
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        notes.add(a, note("/a/x.rs", (1, 1), "one"));
        notes.add(a, note("/a/y.rs", (2, 3), "two"));
        notes.add(b, note("/b/z.rs", (5, 5), "three"));
        notes.remove(a, 0);
        notes.remove(a, 7);
        assert_eq!(notes.list(a).len(), 1);
        assert_eq!(notes.list(a)[0].text, "two");
        notes.clear(b);
        assert!(notes.list(b).is_empty());
    }
}
