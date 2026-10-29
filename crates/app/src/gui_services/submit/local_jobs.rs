//! Session, settings, and file/platform job dispatch.
use super::{MISROUTED, Services};
use crate::gui_services::jobs::{
    CloseEditorsInput, CloseIdleInput, ExitSaveInput, InstallInput, OpenProjectInput,
    PrepareLayoutsInput, RepairInstallationInput, ResolveTargetInput, SaveAppearanceInput,
    SaveLayoutInput, SearchInput, TestExternalInput, TestNtfyInput, WorkspaceOpInput,
};
use crate::{Job, Update};
use anyhow::Result;
use std::path::PathBuf;
use terminator_core::async_service::CancellationToken;

impl Services {
    pub(super) async fn session_job(
        &self,
        job: Job,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        match job {
            Job::PrepareLayouts(generation, layouts) => {
                self.prepare_layouts(PrepareLayoutsInput {
                    generation,
                    layouts,
                    cancel,
                    updates,
                })
                .await
            }
            Job::SaveLayout(project, layout, text) => {
                self.save_layout(SaveLayoutInput {
                    project,
                    layout,
                    text,
                    updates,
                })
                .await
            }
            Job::Control(request, after) => self.control(request, after, updates).await,
            Job::CloseIdle(target, generation, ids) => {
                self.close_idle(CloseIdleInput {
                    target,
                    generation,
                    ids,
                    updates,
                })
                .await
            }
            Job::CloseEditors(target, ids, mode, timeout) => {
                self.close_editors(CloseEditorsInput {
                    target,
                    ids,
                    mode,
                    timeout,
                    updates,
                })
                .await
            }
            Job::ExitSave(id, checkpoint) => {
                self.exit_save(ExitSaveInput {
                    id,
                    checkpoint,
                    cancel,
                    updates,
                })
                .await
            }
            Job::ExitDrain(id, serial) => {
                updates.push(Update::ExitDrained(id, serial));
                Ok(())
            }
            Job::RepairInstallation(generation, checkpoint) => {
                self.repair_installation(RepairInstallationInput {
                    generation,
                    checkpoint,
                    cancel,
                    updates,
                })
                .await
            }
            Job::RestartSessionService(inventory) => {
                self.restart_session_service(inventory, cancel).await
            }
            Job::StartSessionService => self.start_session_service(cancel, updates).await,
            _ => anyhow::bail!(MISROUTED),
        }
    }

    pub(super) async fn settings_job(
        &self,
        job: Job,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        match job {
            Job::Preferences(prefs) => self.save_preferences(prefs, cancel, updates).await,
            Job::SaveAppearance(theme, expected) => {
                self.save_appearance(SaveAppearanceInput {
                    theme,
                    expected,
                    cancel,
                    updates,
                })
                .await
            }
            Job::MigrateAttention => self.migrate_settings(true, updates).await,
            Job::MigrateTypography => self.migrate_settings(false, updates).await,
            Job::HookStatus => self.hook_status(cancel, updates).await,
            Job::Install(kind, remove) => {
                self.install(InstallInput {
                    kind,
                    remove,
                    cancel,
                    updates,
                })
                .await
            }
            Job::TestNtfy { channel, machine } => {
                self.test_ntfy(TestNtfyInput {
                    channel,
                    machine,
                    cancel,
                    updates,
                })
                .await
            }
            _ => anyhow::bail!(MISROUTED),
        }
    }

    pub(super) async fn files_job(
        &self,
        job: Job,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        match job {
            Job::ResolveTarget(key, text, cwd) => {
                self.resolve_target(ResolveTargetInput {
                    key,
                    text,
                    cwd,
                    cancel,
                    updates,
                })
                .await
            }
            Job::OpenProject(path, generation) => {
                self.open_project(OpenProjectInput {
                    path,
                    generation,
                    cancel,
                    updates,
                })
                .await
            }
            Job::Workspace(root, op) => {
                self.workspace_op(WorkspaceOpInput {
                    root,
                    op,
                    cancel,
                    updates,
                })
                .await
            }
            Job::Search {
                id,
                root,
                query,
                show_ignored,
            } => {
                self.search(SearchInput {
                    id,
                    root,
                    query,
                    show_ignored,
                    cancel,
                    updates,
                })
                .await
            }
            Job::LinePreviews(generation, files) => {
                self.line_previews(LinePreviewsInput {
                    generation,
                    files,
                    cancel,
                    updates,
                })
                .await
            }
            Job::PasteClipboard(session) => self.paste_clipboard(session, cancel, updates).await,
            Job::Browser(url) => self.open_browser(url, cancel).await,
            Job::Favicon(url) => {
                let bytes = tokio::select! {
                    () = cancel.cancelled() => return Ok(()),
                    bytes = crate::browser::favicon::download(&url) => bytes,
                };
                updates.push(Update::Favicon(url, bytes));
                Ok(())
            }
            Job::External(path) => self.open_external(path, cancel).await,
            Job::TestExternal(path, program, args) => {
                self.test_external(TestExternalInput {
                    path,
                    program,
                    args,
                    cancel,
                })
                .await
            }
            _ => anyhow::bail!(MISROUTED),
        }
    }

    async fn line_previews(&self, input: LinePreviewsInput<'_>) -> Result<()> {
        let LinePreviewsInput {
            generation,
            files,
            cancel,
            updates,
        } = input;
        let lines = self
            .fs()
            .run(&cancel, move || {
                anyhow::Ok(crate::native_editor::read_line_previews(files))
            })
            .await
            .unwrap_or_default();
        updates.push(Update::LinePreviews(generation, lines));
        Ok(())
    }
}

struct LinePreviewsInput<'a> {
    generation: u64,
    files: Vec<(PathBuf, Vec<u32>)>,
    cancel: CancellationToken,
    updates: &'a mut Vec<Update>,
}
