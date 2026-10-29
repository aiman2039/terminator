//! Native file and folder pickers opened from modal flags.
use crate::*;

impl App {
    pub(super) fn add_project_picker(&mut self, frame: &eframe::Frame) {
        if !self.add_project || self.picker_active {
            return;
        }
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!("Fixture native project picker opened"));
        }
        self.add_project = false;
        self.picker_active = true;
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title("Open project");
        let cwd = self.dialog_directory();
        self.selection_generation = self.selection_generation.wrapping_add(1);
        let generation = self.selection_generation;
        let service = self.services.clone();
        self.services.dialog(async move {
            let cwd = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(services::existing_directory(cwd))
                })
                .await?;
            let dialog = dialog.set_directory(cwd);
            let path = dialog.pick_folder().await.map(|p| p.path().to_path_buf());
            service
                .emit(Update::PickedProject(path, generation))
                .await?;
            Ok(())
        });
    }

    pub(super) fn test_editor_picker(&mut self, frame: &eframe::Frame) {
        if !self.test_editor || self.picker_active {
            return;
        }
        self.test_editor = false;
        self.picker_active = true;
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title("Test external editor");
        let cwd = self.dialog_directory();
        let settings = self.settings_draft.clone();
        let jobs = self.jobs.clone();
        let service = self.services.clone();
        self.services.dialog(async move {
            let cwd = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(services::existing_directory(cwd))
                })
                .await?;
            let path = dialog.set_directory(cwd).pick_file().await;
            if let Some(path) = path {
                let _ = jobs.send(Job::TestExternal(
                    path.path().into(),
                    settings.external_editor,
                    settings.external_args,
                ));
            }
            service.emit(Update::TestPickerClosed).await?;
            Ok(())
        });
    }

    pub(super) fn audio_files_picker(&mut self, frame: &eframe::Frame) {
        if !self.pick_audio || self.picker_active {
            return;
        }
        self.pick_audio = false;
        self.picker_active = true;
        let cwd = self.dialog_directory();
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title("Add audio files")
            .add_filter("Audio", player::AUDIO_EXTENSIONS);
        let service = self.services.clone();
        self.services.dialog(async move {
            let cwd = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(services::existing_directory(cwd))
                })
                .await?;
            let dialog = dialog.set_directory(cwd);
            let paths = dialog
                .pick_files()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|file| file.path().to_path_buf())
                .collect();
            service.emit(Update::PickedAudio(paths)).await?;
            Ok(())
        });
    }

    /// RealVNC Viewer `.vnc` files for the VNC "Import hosts" panel.
    pub(super) fn vnc_files_picker(&mut self, frame: &eframe::Frame) {
        if !self.pick_vnc_files || self.picker_active {
            return;
        }
        self.pick_vnc_files = false;
        self.picker_active = true;
        let cwd = self.dialog_directory();
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title("Import RealVNC connections")
            .add_filter("VNC connection", &["vnc"]);
        let service = self.services.clone();
        self.services.dialog(async move {
            let cwd = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(services::existing_directory(cwd))
                })
                .await?;
            let paths = dialog
                .set_directory(cwd)
                .pick_files()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|file| file.path().to_path_buf())
                .collect();
            service.emit(Update::PickedVncFiles(paths)).await?;
            Ok(())
        });
    }

    pub(super) fn audio_directory_picker(&mut self, frame: &eframe::Frame) {
        if !self.pick_audio_dir || self.picker_active {
            return;
        }
        self.pick_audio_dir = false;
        self.picker_active = true;
        let cwd = self.dialog_directory();
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title("Add audio directory");
        let service = self.services.clone();
        self.services.dialog(async move {
            let cwd = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(services::existing_directory(cwd))
                })
                .await?;
            let dialog = dialog.set_directory(cwd);
            let folder = dialog.pick_folder().await.map(|f| f.path().to_path_buf());
            let paths = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(
                        folder
                            .map(|path| player::audio_from_dir(&path))
                            .unwrap_or_default(),
                    )
                })
                .await?;
            service.emit(Update::PickedAudio(paths)).await?;
            Ok(())
        });
    }

    pub(super) fn open_file_picker(&mut self, frame: &eframe::Frame) {
        if !self.open_path || self.picker_active {
            return;
        }
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!("Fixture native file picker opened"));
        }
        self.open_path = false;
        self.picker_active = true;
        let cwd = self.dialog_directory();
        let project = self.selected.clone();
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title("Open file");
        if !self.path_text.is_empty() {
            if let Some(name) = std::path::Path::new(&self.path_text).file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
            self.path_text.clear();
        }
        let service = self.services.clone();
        self.services.dialog(async move {
            let selected = cwd.clone();
            let directory = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(services::existing_directory(selected))
                })
                .await?;
            let dialog = dialog.set_directory(directory);
            let path = dialog.pick_file().await.map(|p| p.path().to_path_buf());
            service
                .emit(Update::PickedFile { path, project, cwd })
                .await?;
            Ok(())
        });
    }

    pub(super) fn browse_target_picker(&mut self, frame: &eframe::Frame) {
        let Some(target) = self.browse_target.take() else {
            return;
        };
        if self.picker_active {
            self.browse_target = Some(target);
            return;
        }
        self.picker_active = true;
        let folder = matches!(target, BrowseTarget::WorktreeDest);
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(match target {
                BrowseTarget::Shell => "Choose shell",
                BrowseTarget::Editor => "Choose editor",
                BrowseTarget::External => "Choose external editor",
                BrowseTarget::WorktreeDest => "Choose parent folder",
                BrowseTarget::Lsp(_) => "Choose language server",
            });
        let cwd = self.dialog_directory();
        let service = self.services.clone();
        self.services.dialog(async move {
            let cwd = service
                .fs()
                .run(&async_service::CancellationToken::new(), move || {
                    anyhow::Ok(services::existing_directory(cwd))
                })
                .await?;
            let dialog = dialog.set_directory(cwd);
            let path = if folder {
                dialog.pick_folder().await.map(|p| p.path().to_path_buf())
            } else {
                dialog.pick_file().await.map(|p| p.path().to_path_buf())
            };
            service.emit(Update::PickedPath { path, target }).await?;
            Ok(())
        });
    }
}
