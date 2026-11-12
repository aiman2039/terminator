//! The one page that fills the main pane between the sidebars. Every page
//! opener goes through `App::show_center`, and every "is the workspace
//! covered" question derives from `App::center`, so a page can never stay
//! open (and keep terminal input off) under another one.
use super::super::*;

/// What the main pane shows. `Workspace` is the project's dock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum CenterPage {
    #[default]
    Workspace,
    Settings,
    Player,
    Vnc,
    /// The SSH page: address book, host Properties or import.
    Ssh,
    /// The worktree list (a new-worktree form can show above it).
    Worktrees,
    /// The new task worktree form on its own.
    WorktreeDraft,
    /// Scrollback search of this terminal.
    Search(String),
    /// Shell integration commands of this terminal.
    Commands(String),
    /// The page data is `App::local_history`.
    LocalHistory,
    /// Changes since task start of a worktree; data is `App::task_changes`.
    TaskChanges,
}

impl CenterPage {
    fn is_worktree(&self) -> bool {
        matches!(self, Self::Worktrees | Self::WorktreeDraft)
    }
}

impl App {
    /// The only writer of `center`: leaves the current page, then shows
    /// `page`. Page data that `page` does not use is dropped here.
    pub(crate) fn show_center(&mut self, page: CenterPage) {
        if self.center == CenterPage::Settings && page != CenterPage::Settings {
            self.leave_settings_page();
        }
        // A changed SSH host form stays for the SSH page to show again.
        if page != CenterPage::Ssh && !self.remote_ui.draft_changed() {
            self.remote_ui.close();
            self.remote_ui.import = None;
        }
        if !page.is_worktree() {
            self.worktree_draft = None;
        }
        if page != CenterPage::LocalHistory {
            self.local_history = None;
        }
        if page != CenterPage::TaskChanges {
            self.task_changes = None;
        }
        self.shortcut_capture = None;
        if self.palette.open {
            self.close_palette();
        }
        self.center = page;
    }

    /// Show the workspace again (navigation, or a page's Close).
    pub(crate) fn hide_center_overlay(&mut self) {
        self.show_center(CenterPage::Workspace);
    }

    /// Close `page` if it is the one shown; another page stays.
    pub(crate) fn close_center_page(&mut self, page: &CenterPage) {
        if &self.center == page {
            self.hide_center_overlay();
        }
    }

    /// A dirty Settings session asks before it is discarded; the session
    /// itself stays so Settings can show it again.
    fn leave_settings_page(&mut self) {
        if self.settings_dirty() {
            self.settings_pending = Some(settings_ui::SettingsPending::Close);
        }
    }

    /// A center page replaces the workspace (the Search popup floats over
    /// it).
    pub(crate) fn center_covers_workspace(&self) -> bool {
        self.center != CenterPage::Workspace
    }

    pub(crate) fn settings_open(&self) -> bool {
        self.center == CenterPage::Settings
    }

    pub(crate) fn player_open(&self) -> bool {
        self.center == CenterPage::Player
    }

    /// Settings and the Player draw above the Search popup, so it hides
    /// them and only a few shortcuts reach the app while they show.
    pub(crate) fn center_above_palette(&self) -> bool {
        self.settings_open() || self.player_open()
    }

    /// Center pages that take the keyboard like a dialog.
    pub(crate) fn center_is_dialog(&self) -> bool {
        matches!(
            self.center,
            CenterPage::Search(_)
                | CenterPage::Commands(_)
                | CenterPage::Worktrees
                | CenterPage::WorktreeDraft
        )
    }

    /// The terminal whose scrollback the Search page searches.
    pub(crate) fn search_session(&self) -> Option<&str> {
        match &self.center {
            CenterPage::Search(sid) => Some(sid),
            _ => None,
        }
    }

    /// The terminal whose commands the Commands page lists.
    pub(crate) fn commands_session(&self) -> Option<&str> {
        match &self.center {
            CenterPage::Commands(sid) => Some(sid),
            _ => None,
        }
    }
}
