use eframe::egui;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    Open,
    Text,
    Split,
    External,
    Browser,
    Copy,
    StagedDiff,
    WorkingDiff,
    NativeStagedDiff,
    NativeWorkingDiff,
}
pub fn review_action(group: crate::services::GitGroup) -> Option<FileAction> {
    use crate::services::GitGroup;
    match group {
        GitGroup::Staged => Some(FileAction::StagedDiff),
        GitGroup::Changes | GitGroup::Untracked => Some(FileAction::WorkingDiff),
        GitGroup::Conflicts => None,
    }
}

pub struct FileMenu {
    pub file: bool,
    pub browser: bool,
    pub git: Option<crate::services::GitGroup>,
    pub neovim: bool,
}

pub fn items(
    FileMenu {
        file,
        browser,
        git,
        neovim,
    }: FileMenu,
) -> Vec<(&'static str, FileAction)> {
    let review = git.and_then(review_action);
    let staged = review == Some(FileAction::StagedDiff);
    let working = review == Some(FileAction::WorkingDiff);
    [
        (working, "Native diff", FileAction::NativeWorkingDiff),
        (staged, "Native staged diff", FileAction::NativeStagedDiff),
        (
            neovim && working,
            "Working tree diff",
            FileAction::WorkingDiff,
        ),
        (neovim && staged, "Staged diff", FileAction::StagedDiff),
        (file, "Open file", FileAction::Open),
        (file, "Open as text", FileAction::Text),
        (file, "Open in editor split", FileAction::Split),
        (file, "Open externally", FileAction::External),
        (browser, "Open in browser", FileAction::Browser),
        (true, "Copy target", FileAction::Copy),
    ]
    .into_iter()
    .filter(|(available, _, _)| *available)
    .map(|(_, label, action)| (label, action))
    .collect()
}

pub fn menu(ui: &mut egui::Ui, spec: FileMenu) -> Option<FileAction> {
    for (label, action) in items(spec) {
        let icon = match action {
            FileAction::Open | FileAction::Text => "FileCode",
            FileAction::Split => "PanelRightClose",
            FileAction::External | FileAction::Browser => "ExternalLink",
            FileAction::Copy => "Copy",
            FileAction::StagedDiff
            | FileAction::WorkingDiff
            | FileAction::NativeStagedDiff
            | FileAction::NativeWorkingDiff => "FileDiff",
        };
        let response = crate::appearance::menu_item(ui, label, icon, "");
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(ui.ctx(), label, response.rect);
        if response.clicked() {
            ui.close();
            return Some(action);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{Change, GitGroup};

    #[test]
    fn unstaged_files_offer_a_working_diff_before_staging_or_committing() {
        for status in [" M", "??", " D"] {
            let change = Change {
                path: "main.rs".into(),
                status: status.into(),
            };
            let group = GitGroup::ALL
                .into_iter()
                .find(|group| change.in_group(*group))
                .unwrap();
            assert_eq!(review_action(group), Some(FileAction::WorkingDiff));
        }
    }

    #[test]
    fn partially_staged_files_review_the_side_of_the_selected_group() {
        let change = Change {
            path: "main.rs".into(),
            status: "MM".into(),
        };
        assert!(change.in_group(GitGroup::Staged));
        assert!(change.in_group(GitGroup::Changes));
        assert_eq!(
            review_action(GitGroup::Staged),
            Some(FileAction::StagedDiff)
        );
        assert_eq!(
            review_action(GitGroup::Changes),
            Some(FileAction::WorkingDiff)
        );
        assert_eq!(review_action(GitGroup::Conflicts), None);
    }

    #[test]
    fn git_changes_menu_leads_with_native_diff() {
        let entries = items(FileMenu {
            file: true,
            browser: false,
            git: Some(GitGroup::Changes),
            neovim: false,
        });
        assert_eq!(entries[0], ("Native diff", FileAction::NativeWorkingDiff));
        assert!(
            !entries
                .iter()
                .any(|(label, _)| *label == "Working tree diff")
        );
    }

    #[test]
    fn git_staged_menu_leads_with_native_staged_diff() {
        let entries = items(FileMenu {
            file: true,
            browser: false,
            git: Some(GitGroup::Staged),
            neovim: false,
        });
        assert_eq!(
            entries[0],
            ("Native staged diff", FileAction::NativeStagedDiff)
        );
    }

    #[test]
    fn neovim_mode_keeps_native_diff_beside_the_review_action() {
        let entries = items(FileMenu {
            file: true,
            browser: false,
            git: Some(GitGroup::Untracked),
            neovim: true,
        });
        assert_eq!(entries[0], ("Native diff", FileAction::NativeWorkingDiff));
        assert!(entries.contains(&("Working tree diff", FileAction::WorkingDiff)));
    }
}
