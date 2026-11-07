//! The native file picker: one request at a time.
//!
//! A click asks for a picker and the next frame opens it. A second request
//! before it opens replaces the first. While a picker is open, a new request
//! is refused with a status note. No request waits behind an open picker, so
//! no queued request keeps terminal input off.

use crate::*;

/// What a native picker chooses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickKind {
    /// Add Project: a folder to open as a project.
    Project,
    /// Locate Folder…: the new folder of this missing project.
    Locate(String),
    /// Settings → Test draft: a file for the unsaved external editor.
    TestEditor,
    /// A program or folder for a Settings or worktree form field.
    Browse(BrowseTarget),
    /// Open File: a file to open in the selected project.
    OpenFile,
    /// Audio files for the player.
    AudioFiles,
    /// RealVNC `.vnc` files for the VNC import panel.
    VncFiles,
    /// A folder of audio files for the player.
    AudioDir,
}

impl PickKind {
    /// The dialog title, also used to name an open picker.
    pub(crate) fn title(&self) -> &'static str {
        match self {
            Self::Project => "Open project",
            Self::Locate(_) => "Locate folder",
            Self::TestEditor => "Test external editor",
            Self::Browse(BrowseTarget::Shell) => "Choose shell",
            Self::Browse(BrowseTarget::Editor) => "Choose editor",
            Self::Browse(BrowseTarget::External) => "Choose external editor",
            Self::Browse(BrowseTarget::WorktreeDest) => "Choose parent folder",
            Self::Browse(BrowseTarget::Lsp(_)) => "Choose language server",
            Self::OpenFile => "Open file",
            Self::AudioFiles => "Add audio files",
            Self::VncFiles => "Import RealVNC connections",
            Self::AudioDir => "Add audio directory",
        }
    }
}

/// The one native picker of the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Picker {
    #[default]
    Idle,
    /// Opens on the next frame.
    Requested(PickKind),
    /// The dialog is on screen; its result closes it.
    Open(PickKind),
}

impl Picker {
    /// A picker is requested or open. Terminal input and shortcuts wait.
    pub(crate) fn busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// `kind` waits to open.
    #[cfg(test)]
    pub(crate) fn requested(&self, kind: &PickKind) -> bool {
        matches!(self, Self::Requested(requested) if requested == kind)
    }

    /// Takes the request and marks its picker open.
    pub(crate) fn begin(&mut self) -> Option<PickKind> {
        let Self::Requested(kind) = self else {
            return None;
        };
        let kind = kind.clone();
        *self = Self::Open(kind.clone());
        Some(kind)
    }

    /// The picker returned (a choice, a cancel, or an error).
    pub(crate) fn close(&mut self) {
        *self = Self::Idle;
    }
}

impl App {
    /// Asks for a native picker. While another picker is open, the request
    /// is refused with a status note and this returns false.
    pub(crate) fn request_pick(&mut self, kind: PickKind) -> bool {
        if let Picker::Open(open) = &self.picker {
            self.info = Some(format!(
                "Close the \"{}\" dialog first, then try again.",
                open.title()
            ));
            return false;
        }
        self.picker = Picker::Requested(kind);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_request_replaces_the_first_before_it_opens() {
        let mut picker = Picker::Requested(PickKind::Project);
        assert!(picker.busy());
        picker = Picker::Requested(PickKind::OpenFile);
        assert_eq!(picker.begin(), Some(PickKind::OpenFile));
        assert!(picker.is_open());
        assert_eq!(picker.begin(), None);
        picker.close();
        assert!(!picker.busy());
    }
}
