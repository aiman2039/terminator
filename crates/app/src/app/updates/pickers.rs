//! Native file, folder, and project picker results.

use std::path::PathBuf;

use crate::native_editor::OpenNativeInput;
use crate::*;

impl App {
    pub(super) fn handle_picked_project(&mut self, path: Option<PathBuf>, generation: u64) {
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!(
                "Fixture native project picker selected={}",
                path.is_some()
            ));
        }
        self.picker_active = false;
        if let Some(path) = path
            && generation == self.selection_generation
        {
            let _ = self.jobs.send(Job::OpenProject(path, generation));
        }
    }

    pub(super) fn handle_picked_path(&mut self, path: Option<PathBuf>, target: BrowseTarget) {
        self.picker_active = false;
        let Some(path) = path else {
            return;
        };
        let text = path.display().to_string();
        match target {
            BrowseTarget::Shell => self.settings_draft.shell = text,
            BrowseTarget::Editor => self.settings_draft.editor_program = text,
            BrowseTarget::External => {
                self.settings_draft.external_editor = text;
                self.editor_preset = external_editor::CUSTOM;
            }
            BrowseTarget::Lsp(language) => {
                self.settings_draft
                    .lsp_servers
                    .insert(language.to_owned(), text);
            }
            BrowseTarget::WorktreeDest => {
                if let Some(draft) = &mut self.worktree_draft {
                    let leaf = draft
                        .dest
                        .file_name()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from("terminator-task"));
                    draft.dest = path.join(leaf);
                }
            }
        }
    }

    pub(super) fn handle_picked_audio(&mut self, paths: Vec<PathBuf>) {
        self.picker_active = false;
        self.add_audio_files(paths);
    }

    pub(super) fn handle_picked_file(
        &mut self,
        path: Option<PathBuf>,
        project: Option<String>,
        cwd: PathBuf,
    ) {
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!(
                "Fixture native file picker selected={}",
                path.is_some()
            ));
        }
        self.picker_active = false;
        let Some(path) = path else {
            return;
        };
        if image_preview::supported(&path)
            && let Some(project) = &project
        {
            self.open_image(project, path, None);
        } else if crate::browser::supported_file(&path)
            && let Some(project) = &project
        {
            self.open_html(project, path, None);
        } else if player::supported(&path)
            && let Some(project) = &project
        {
            self.open_audio(project, path, None);
        } else if self.state.settings.editor_mode == EditorMode::External {
            let _ = self.jobs.send(Job::External(path));
        } else if self.state.settings.editor_mode == EditorMode::Native
            && let Some(project) = &project
        {
            self.open_native(OpenNativeInput {
                project,
                path,
                line: None,
                split: None,
            });
        } else if let Some(project) = project {
            self.open_picked_in_editor_session(project, path, cwd);
        }
    }

    /// Opens a picked file in a terminal editor session, reusing one that
    /// already shows it.
    fn open_picked_in_editor_session(&mut self, project: String, path: PathBuf, cwd: PathBuf) {
        if self.reuse_file_session(&project, &path) {
            return;
        }
        self.hide_center_overlay();
        let after = self.editor_target(&project, None, None);
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project,
                cwd: Some(cwd),
                file: Some(path),
                line: None,
                column: None,
                editor: true,
            },
            after,
        ));
    }
}
