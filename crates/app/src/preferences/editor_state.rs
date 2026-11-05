//! Where native editor files were left (`JetBrains`): the caret, the top
//! line and the folds per file, kept in `ui-preferences.json` so a file
//! reopens where it was, also after a GUI restart.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Files remembered at most; the ones closed longest ago drop off.
pub const MAX_EDITOR_STATES: usize = 500;

/// One file's editor position: 0-based buffer lines and char columns.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorState {
    /// Absolute path of the file.
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    /// First buffer line shown at the top of the view.
    pub top: usize,
    /// First lines of the folded regions.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<usize>,
}

/// Remembered editor positions, newest first. An entry that does not
/// parse (damaged, or from a newer build) is dropped on load instead of
/// failing every other preference.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct EditorStates(Vec<EditorState>);

impl<'de> Deserialize<'de> for EditorStates {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let serde_json::Value::Array(items) = serde_json::Value::deserialize(d)? else {
            return Ok(Self::default());
        };
        let mut states: Vec<EditorState> = items
            .into_iter()
            .filter_map(|item| serde_json::from_value::<EditorState>(item).ok())
            .filter(|state| state.path.is_absolute())
            .collect();
        states.truncate(MAX_EDITOR_STATES);
        Ok(Self(states))
    }
}

impl EditorStates {
    /// The remembered position of `path`.
    pub fn get(&self, path: &Path) -> Option<&EditorState> {
        self.0.iter().find(|state| state.path == path)
    }

    /// Keep `state` as the newest entry for its file; past
    /// [`MAX_EDITOR_STATES`] the oldest entries drop off. A path that is
    /// not UTF-8 is not kept: the preferences file stores paths as text.
    pub fn remember(&mut self, state: EditorState) {
        if state.path.to_str().is_none() {
            return;
        }
        self.0.retain(|old| old.path != state.path);
        self.0.insert(0, state);
        self.0.truncate(MAX_EDITOR_STATES);
    }

    /// Follow a rename or move: `moved` maps an old path to its new one.
    pub fn remap(&mut self, moved: impl Fn(&Path) -> Option<PathBuf>) {
        for state in &mut self.0 {
            if let Some(next) = moved(&state.path) {
                state.path = next;
            }
        }
    }

    /// Remembered files, newest first.
    #[cfg(test)]
    pub fn paths(&self) -> Vec<&Path> {
        self.0.iter().map(|state| state.path.as_path()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::UiPreferences;

    /// An absolute path on every platform; `/src` is relative on Windows.
    fn src(name: &str) -> String {
        let root = if cfg!(windows) { "C:/src" } else { "/src" };
        format!("{root}/{name}")
    }

    fn state(path: &str, line: usize) -> EditorState {
        EditorState {
            path: PathBuf::from(path),
            line,
            column: 3,
            top: line.saturating_sub(5),
            folds: vec![1, 9],
        }
    }

    #[test]
    fn editor_states_survive_a_preferences_save_and_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut prefs = UiPreferences::default();
        prefs.editor_states.remember(state(&src("a.rs"), 40));
        let unfolded = EditorState {
            folds: Vec::new(),
            ..state(&src("b.rs"), 7)
        };
        prefs.editor_states.remember(unfolded.clone());
        prefs.save(dir.path()).expect("save");
        let loaded = UiPreferences::load(dir.path()).expect("load");
        assert_eq!(loaded.editor_states, prefs.editor_states);
        assert_eq!(
            loaded.editor_states.get(Path::new(&src("b.rs"))),
            Some(&unfolded)
        );
        assert_eq!(
            loaded.editor_states.paths(),
            [Path::new(&src("b.rs")), Path::new(&src("a.rs"))]
        );
    }

    #[test]
    fn damaged_or_relative_entries_are_dropped_on_load() {
        let json = serde_json::json!([
            {"path": src("a.rs"), "line": 4},
            {"path": "rel.rs"},
            {"path": src("b.rs"), "line": -2},
            {"line": 3},
            "junk"
        ]);
        let states: EditorStates = serde_json::from_value(json).expect("parse");
        assert_eq!(states.paths(), [Path::new(&src("a.rs"))]);
        let kept = states.get(Path::new(&src("a.rs"))).expect("kept");
        assert_eq!((kept.line, kept.column, kept.top), (4, 0, 0));
        assert!(kept.folds.is_empty(), "missing fields take defaults");
        let other: EditorStates = serde_json::from_str(r#"{"a":1}"#).expect("parse");
        assert!(other.paths().is_empty(), "not a list: nothing kept");
    }

    #[test]
    fn only_the_most_recent_files_are_kept() {
        let mut states = EditorStates::default();
        for index in 0..=MAX_EDITOR_STATES {
            states.remember(state(&src(&format!("{index}.rs")), index));
        }
        assert_eq!(states.paths().len(), MAX_EDITOR_STATES);
        assert!(
            states.get(Path::new(&src("0.rs"))).is_none(),
            "oldest dropped"
        );
        // Remembering a file again moves it to the front, no duplicate.
        states.remember(state(&src("1.rs"), 99));
        assert_eq!(states.paths().len(), MAX_EDITOR_STATES);
        assert_eq!(states.paths().first(), Some(&Path::new(&src("1.rs"))));
        assert_eq!(
            states.get(Path::new(&src("1.rs"))).map(|s| s.line),
            Some(99)
        );
        // A longer list from a file is cut to the cap on load.
        let many: Vec<EditorState> = (0..=MAX_EDITOR_STATES)
            .map(|index| state(&src(&format!("{index}.rs")), index))
            .collect();
        let json = serde_json::to_string(&many).expect("serialize");
        let loaded: EditorStates = serde_json::from_str(&json).expect("parse");
        assert_eq!(loaded.paths().len(), MAX_EDITOR_STATES);
    }
}
