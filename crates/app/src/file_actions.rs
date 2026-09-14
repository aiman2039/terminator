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
}
pub fn review_action(group: crate::services::GitGroup) -> Option<FileAction> {
    use crate::services::GitGroup;
    match group {
        GitGroup::Staged => Some(FileAction::StagedDiff),
        GitGroup::Changes | GitGroup::Untracked => Some(FileAction::WorkingDiff),
        GitGroup::Conflicts => None,
    }
}

pub fn menu(
    ui: &mut egui::Ui,
    file: bool,
    browser: bool,
    git: Option<crate::services::GitGroup>,
) -> Option<FileAction> {
    let review = git.and_then(review_action);
    for (available, label, action) in [
        (
            review == Some(FileAction::StagedDiff),
            "Staged diff",
            FileAction::StagedDiff,
        ),
        (
            review == Some(FileAction::WorkingDiff),
            "Working tree diff",
            FileAction::WorkingDiff,
        ),
        (file, "Open file", FileAction::Open),
        (file, "Open as text", FileAction::Text),
        (file, "Open in editor split", FileAction::Split),
        (file, "Open externally", FileAction::External),
        (browser, "Open in browser", FileAction::Browser),
        (true, "Copy target", FileAction::Copy),
    ] {
        if available {
            let icon = match action {
                FileAction::Open | FileAction::Text => "FileCode",
                FileAction::Split => "PanelRightClose",
                FileAction::External | FileAction::Browser => "ExternalLink",
                FileAction::Copy => "Copy",
                FileAction::StagedDiff | FileAction::WorkingDiff => "FileDiff",
            };
            let response = crate::appearance::menu_item(ui, label, icon, "");
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(ui.ctx(), label, response.rect);
            if response.clicked() {
                ui.close();
                return Some(action);
            }
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
}
