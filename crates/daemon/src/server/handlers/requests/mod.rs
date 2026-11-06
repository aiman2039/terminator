mod editor;
mod lifecycle;
mod local_history;
mod notifications;
mod prepare;
mod projects;
mod remote;
mod sessions;
mod vnc;
mod worktrees;

use anyhow::{Result, bail};
use std::sync::{Arc, atomic::Ordering};
use terminator_core::*;

use super::super::shared::{Shared, relock};
use crate::Launch;

/// What a request handler produced.
#[expect(
    clippy::large_enum_variant,
    reason = "short-lived return value; boxing would allocate on every reply"
)]
enum Outcome {
    /// Reply as-is, without persisting state.
    Reply(Response),
    /// State changed: save the shared workspace and persist before replying `Ok`.
    Changed,
    /// High-rate state change (agent hooks): reply `Ok` now; the generation
    /// worker persists within about a second, and any earlier persist
    /// carries it too. Losing it in a crash costs only recent hook state.
    Deferred,
}

impl Shared {
    pub(crate) fn handle(self: &Arc<Self>, mut request: Request) -> Result<Response> {
        // Every shell prompt reports its cwd; skip the coordination lock and
        // the persist when nothing changed.
        if let Request::Cwd { session, path } = &request
            && self.cwd_unchanged(session, path)
        {
            return Ok(Response::Ok);
        }
        self.vnc_keychain_unlocked(&mut request)?;
        let prepared = self.prepare_request(&request)?;
        if let Some(response) = prepared.early {
            return Ok(response);
        }
        let _coordination = prepared.coordination;
        match self.dispatch(request)? {
            Outcome::Reply(response) => Ok(response),
            Outcome::Changed => self.commit_change(prepared.shared_write),
            Outcome::Deferred if prepared.shared_write => self.commit_change(true),
            Outcome::Deferred => {
                self.persist_pending.store(true, Ordering::Release);
                Ok(Response::Ok)
            }
        }
    }

    fn commit_change(&self, shared_write: bool) -> Result<Response> {
        if shared_write && let Some(paths) = &self.catalog_paths {
            generations::Catalog::open(paths)?.save_workspace(&relock(&self.state))?;
        }
        self.persist()?;
        Ok(Response::Ok)
    }

    // sarah-allow(large_function): one-line dispatch arms over every IPC request variant
    fn dispatch(self: &Arc<Self>, request: Request) -> Result<Outcome> {
        match request {
            Request::VncProfileSave { profile, .. } => self.on_vnc_profile_save(profile),
            Request::VncForgetCredential { id } => self.on_vnc_forget_credential(&id),
            Request::VncTransfer { session, transfer } => self.on_vnc_transfer(&session, transfer),
            Request::VncTransferStatus { session, id } => {
                self.on_vnc_transfer_status(&session, &id)
            }
            Request::VncProfileRemove { id } => self.on_vnc_profile_remove(&id),
            Request::VncConnect {
                options,
                project,
                profile_id,
                host,
                port,
                username,
                password,
                shared,
                view_only,
                label,
            } => self.on_vnc_connect(crate::vnc_session::VncConnectSpec {
                options,
                project,
                profile_id,
                host,
                port,
                username,
                password,
                shared,
                view_only,
                label,
            }),
            Request::VncDisconnect { session } => self.on_vnc_disconnect(&session),
            Request::VncInput { session, events } => self.on_vnc_input(&session, events),
            Request::VncFrame {
                session,
                since,
                regions,
                shm,
            } => self.on_vnc_frame(crate::vnc_session::VncFrameRequest {
                session: &session,
                since,
                regions,
                shm,
            }),
            Request::PruneHistory { budget } => self.on_prune_history(budget),
            Request::Archived {
                generation,
                request,
            } => self
                .handle_archived(generation, request)
                .map(Outcome::Reply),
            Request::CloseIdleSessions {
                generation,
                sessions,
            } => self.close_idle(generation, sessions).map(Outcome::Reply),
            Request::ShellCommand { session } => self.on_shell_command(&session),
            Request::ShellPrompt { session, .. } => self.on_shell_prompt(&session),
            Request::WorktreeList { project } => self.on_worktree_list(&project),
            Request::WorktreeAdd {
                project,
                path,
                branch,
                start,
            } => self.on_worktree_add(worktrees::WorktreeAddInput {
                project,
                path,
                branch,
                start,
            }),
            Request::WorktreeRemove { project } => self.on_worktree_remove(&project),
            Request::Screen { session } => self.on_screen(&session),
            Request::CommandBlocks { session, since } => self.on_command_blocks(&session, since),
            Request::Snapshot => Ok(self.on_snapshot()),
            Request::SessionForPids { pids } => Ok(self.on_session_for_pids(&pids)),
            Request::Heartbeat { focused } => Ok(self.on_heartbeat(focused)),
            Request::Create {
                project,
                cwd,
                file,
                line,
                column,
                editor,
            } => self.on_create(super::sessions::CreateInput {
                project,
                cwd,
                file,
                line,
                column,
                launch: if editor {
                    Launch::Editor
                } else {
                    Launch::Shell
                },
            }),
            Request::CreateReview {
                project,
                cwd,
                path,
                staged,
            } => self.on_create(super::sessions::CreateInput {
                project,
                cwd: Some(cwd),
                file: Some(path),
                line: None,
                column: None,
                launch: Launch::Review { staged },
            }),
            Request::ProbeRemoteProject { project } => self.on_probe_remote_project(&project),
            Request::ScanHostKey { ssh } => self.on_scan_host_key(&ssh),
            Request::TrustHostKey { ssh, keys } => self.on_trust_host_key(&ssh, &keys),
            Request::RemoteProject { project, operation } => {
                self.on_remote_project(&project, operation)
            }
            Request::AddRemoteProject { project } => self.on_add_remote_project(project),
            Request::RemoveRemoteProject { project } => self.on_remove_remote_project(&project),
            Request::SetRemoteProjectGroup { project, group } => {
                self.on_set_remote_project_group(&project, group.as_deref())
            }
            Request::UpdateRemoteProject { project } => self.on_update_remote_project(project),
            Request::SetRemoteProjectForwards { project, forwards } => {
                self.on_set_remote_project_forwards(&project, forwards)
            }
            Request::StartForward {
                project,
                local_port,
            } => self.on_start_forward(&project, local_port),
            Request::StopForward {
                project,
                local_port,
            } => self.on_stop_forward(&project, local_port),
            Request::ForwardStatus => Ok(self.on_forward_status()),
            Request::AddProject { path } => self.on_add_project(&path),
            Request::SaveLayout { project, layout } => self.on_save_layout(&project, layout),
            Request::SelectProject { project } => self.on_select_project(project),
            Request::SetInitCommand { project, command } => {
                self.on_set_init_command(&project, command.as_deref())
            }
            Request::Cwd { session, path } => self.on_cwd(&session, path),
            Request::Focus { session } => Ok(self.on_focus(&session)),
            Request::Notice { id, action } => self.on_notice(&id, &action),
            Request::TerminalNotify {
                session,
                title,
                body,
            } => self.on_terminal_notify(&session, &title, &body),
            Request::DismissTerminalNotice { id } => self.on_dismiss_terminal_notice(&id),
            Request::Settings(settings) => self.on_settings(settings),
            Request::Hook(event) => self.on_hook(event),
            Request::Rename { session, label } => self.on_rename(&session, label),
            Request::Stop { session } => self.on_stop(&session),
            Request::Remove { session } => self.on_remove(session),
            Request::History { session } => self.on_history(&session),
            Request::ClearHistory { session } => self.on_clear_history(session),
            Request::EditorSave { session } => self.on_editor_save(&session),
            Request::EditorCompare { session } => self.on_editor_compare(&session),
            Request::EditorStatus { session } => self.on_editor_status(&session),
            Request::Shutdown | Request::ShutdownIfIdle => self.on_shutdown(),
            Request::RetireIfDraining => self.on_retire_if_draining(),
            Request::LocalHistoryList { path } => self.on_local_history_list(&path),
            Request::LocalHistoryRead { path, id } => self.on_local_history_read(&path, &id),
            Request::LocalHistoryLabel { project, label } => {
                self.on_local_history_label(&project, label)
            }
            Request::LocalHistoryRevert { path, id } => self.on_local_history_revert(&path, &id),
            _ => bail!("Request requires an attached stream"),
        }
    }
}
