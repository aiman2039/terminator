//! Native file and folder pickers, opened one at a time from
//! [`App::picker`].
use crate::*;

impl App {
    /// Opens the requested picker, if any.
    pub(super) fn open_requested_picker(&mut self, frame: &eframe::Frame) {
        let Some(kind) = self.picker.begin() else {
            return;
        };
        match kind {
            PickKind::Project => self.add_project_picker(frame, None),
            PickKind::Locate(project) => self.add_project_picker(frame, Some(project)),
            PickKind::TestEditor => self.test_editor_picker(frame),
            PickKind::Browse(target) => self.browse_target_picker(frame, target),
            PickKind::OpenFile => self.open_file_picker(frame),
            PickKind::AudioFiles => self.audio_files_picker(frame),
            PickKind::VncFiles => self.vnc_files_picker(frame),
            PickKind::AudioDir => self.audio_directory_picker(frame),
        }
    }

    fn add_project_picker(&mut self, frame: &eframe::Frame, locate: Option<String>) {
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!("Fixture native project picker opened"));
        }
        let (generation, title) = self.begin_project_pick(locate);
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(title);
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
            let path = dialog.pick_folder().await.map(|p| p.path().to_path_buf());
            service
                .emit(Update::PickedProject(path, generation))
                .await?;
            Ok(())
        });
    }

    fn test_editor_picker(&mut self, frame: &eframe::Frame) {
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(PickKind::TestEditor.title());
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

    fn audio_files_picker(&mut self, frame: &eframe::Frame) {
        let cwd = self.dialog_directory();
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(PickKind::AudioFiles.title())
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
    fn vnc_files_picker(&mut self, frame: &eframe::Frame) {
        let cwd = self.dialog_directory();
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(PickKind::VncFiles.title())
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

    fn audio_directory_picker(&mut self, frame: &eframe::Frame) {
        let cwd = self.dialog_directory();
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(PickKind::AudioDir.title());
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

    fn open_file_picker(&mut self, frame: &eframe::Frame) {
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!("Fixture native file picker opened"));
        }
        let cwd = self.dialog_directory();
        let project = self.selected.clone();
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(PickKind::OpenFile.title());
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

    fn browse_target_picker(&mut self, frame: &eframe::Frame, target: BrowseTarget) {
        let folder = matches!(target, BrowseTarget::WorktreeDest);
        let dialog = rfd::AsyncFileDialog::new()
            .set_parent(frame)
            .set_title(PickKind::Browse(target).title());
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
