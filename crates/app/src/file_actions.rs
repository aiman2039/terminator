use crate::services::Target;
use eframe::egui;
use std::path::Path;
use url::Url;

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

pub fn browser_document(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "html" | "htm" | "xhtml"))
}

pub fn file_url(path: &Path, cwd: &Path) -> Option<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    Url::from_file_path(absolute)
        .ok()
        .map(|url| url.to_string())
}

pub fn target_menu(target: &Target) -> FileMenu {
    FileMenu {
        file: matches!(target, Target::File(..)),
        browser: match target {
            Target::Url(_) => true,
            Target::File(path, ..) => browser_document(path),
        },
        git: None,
        neovim: false,
    }
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
        (neovim && working, "Neovim diff", FileAction::WorkingDiff),
        (
            neovim && staged,
            "Neovim staged diff",
            FileAction::StagedDiff,
        ),
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
        assert!(!entries.iter().any(|(label, _)| *label == "Neovim diff"));
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
        assert!(entries.contains(&("Neovim diff", FileAction::WorkingDiff)));
    }

    #[test]
    fn html_files_offer_open_in_browser() {
        assert!(browser_document(Path::new("docs/index.HTML")));
        assert!(browser_document(Path::new("a.htm")));
        assert!(browser_document(Path::new("page.xhtml")));
        assert!(!browser_document(Path::new("readme.md")));
        let entries = items(FileMenu {
            file: true,
            browser: browser_document(Path::new("page.html")),
            git: None,
            neovim: false,
        });
        assert!(entries.contains(&("Open in browser", FileAction::Browser)));
    }

    #[test]
    fn file_url_uses_the_file_scheme_and_resolves_relative_paths() {
        let absolute = file_url(Path::new("/tmp/page.html"), Path::new("/")).expect("absolute");
        assert!(absolute.starts_with("file://"));
        assert!(absolute.ends_with("/tmp/page.html"));
        let relative = file_url(Path::new("page.html"), Path::new("/tmp")).expect("relative");
        assert_eq!(relative, absolute);
    }

    #[test]
    fn url_targets_and_html_files_offer_the_browser_action() {
        let url = target_menu(&Target::Url("https://example.com".into()));
        assert!(url.browser);
        assert!(!url.file);
        let html = target_menu(&Target::File(Path::new("index.html").into(), None, None));
        assert!(html.browser);
        assert!(html.file);
        let rust = target_menu(&Target::File(Path::new("main.rs").into(), None, None));
        assert!(!rust.browser);
        assert!(rust.file);
    }
}
