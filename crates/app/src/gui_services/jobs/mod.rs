mod daemon;
pub(crate) use daemon::CloseEditorsInput;
pub(crate) use daemon::CloseIdleInput;
pub(crate) use daemon::ExitSaveInput;
pub(crate) use daemon::PrepareLayoutsInput;
pub(crate) use daemon::RepairInstallationInput;
pub(crate) use daemon::SaveLayoutInput;
mod files;
pub(crate) use files::OpenProjectInput;
pub(crate) use files::ResolveTargetInput;
pub(crate) use files::SearchInput;
pub(crate) use files::WorkspaceOpInput;
mod git;
pub(crate) use git::DiffActionInput;
pub(crate) use git::DiffContextInput;
mod platform;
pub(crate) use platform::TestExternalInput;
mod remote;
pub(crate) use remote::RemoteAddInput;
pub(crate) use remote::RemoteAssetInput;
pub(crate) use remote::RemoteTransferInput;
mod settings;
pub(crate) use settings::InstallInput;
pub(crate) use settings::SaveAppearanceInput;
pub(crate) use settings::TestNtfyInput;
mod vnc;
pub(crate) use vnc::VncFrameInput;
pub(crate) use vnc::VncPickTransferInput;

use super::submit::Services;
use crate::{After, Job, Tab, Update, editor_close, image_preview};
use anyhow::Result;
use std::path::PathBuf;
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_core::{Request, async_client, async_service};

pub(super) fn editor_ids(job: &Job) -> Vec<String> {
    let mut ids = match job {
        Job::CloseEditors(_, ids, _, _) => ids.clone(),
        Job::Control(request, _) => match request.as_ref() {
            Request::CreateReview {
                project,
                cwd,
                path,
                staged,
            } => vec![format!("review:{project}:{cwd:?}:{path:?}:{staged}")],
            Request::EditorSave { session }
            | Request::EditorStatus { session }
            | Request::EditorCompare { session }
            | Request::Stop { session } => vec![session.clone()],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    ids.sort();
    ids.dedup();
    ids
}
pub(super) fn rejection(job: &Job) -> Vec<Update> {
    let busy = || "Services are busy; the action was not accepted. Retry it.".to_string();
    match job {
        Job::RemoteAdd {
            project,
            generation,
            ..
        } => vec![Update::RemoteAdded(
            project.clone(),
            *generation,
            Err(busy().into()),
        )],
        Job::RemoteRead { project, operation } => vec![Update::RemoteReply(
            project.clone(),
            operation.clone(),
            Err(busy().into()),
        )],
        Job::RemoteTransfer { project, .. } => vec![Update::RemoteTransfer(
            project.clone(),
            None,
            Err(busy().into()),
        )],
        Job::HostKey { ssh, keys: None } => {
            vec![Update::HostKeyScanned(ssh.clone(), Err(busy()))]
        }
        Job::HostKey { ssh, keys: Some(_) } => {
            vec![Update::HostKeyTrusted(ssh.clone(), Err(busy()))]
        }
        Job::SshConfigHosts(generation) => vec![Update::SshConfigHosts(*generation, Vec::new())],
        Job::SshControlStatus(_) => vec![Update::SshControlStatus(Vec::new())],
        Job::Forwards(request) => vec![Update::Forwards(
            crate::forwards_ui::forward_key(request),
            Err(busy().into()),
        )],
        Job::SaveLayout(project, _, text) => vec![Update::LayoutSaved(
            project.clone(),
            text.clone(),
            Err(busy()),
        )],
        Job::CloseEditors(target, ids, _, _) => vec![Update::EditorsClosed(
            target.clone(),
            ids.clone(),
            Err(busy()),
        )],
        Job::CloseIdle(target, _, ids) => {
            vec![Update::IdleClosed(target.clone(), ids.clone(), Err(busy()))]
        }
        Job::Preferences(_) => vec![Update::PreferencesSaved(Box::new(Err(busy())))],
        Job::RepairInstallation(..) => vec![Update::InstallationRepaired(Err(busy()))],
        Job::RestartSessionService(_) => vec![Update::RestartFinished(busy())],
        Job::StartSessionService => vec![Update::ServiceStarted(Err(busy()))],
        Job::Diff(tab) | Job::DiffContext(tab, _) | Job::DiffExpand(tab, ..) => {
            vec![Update::Diff(tab.key(), Err(busy()))]
        }
        Job::DiffAction(tab, _) => vec![Update::DiffActionFailed(tab.key(), busy())],
        Job::ResolveTarget(key, _, _) => vec![Update::ResolvedTarget(key.clone(), None)],
        Job::ExitSave(id, _) => vec![Update::ExitSaved(*id, Err(busy()))],
        Job::GitView(tab) => vec![Update::GitView(
            tab.key(),
            crate::GitView::failed(tab, busy()),
        )],
        _ => Vec::new(),
    }
}

pub(super) fn context(job: &Job) -> OperationContext {
    let (subsystem, resource, policy) = match job {
        Job::PrepareLayouts(_, _) => ("layout", "snapshot".into(), Policy::ReplaceableRead),
        Job::SaveLayout(project, _, _) => (
            "daemon",
            format!("layout:{project}"),
            Policy::OrderedMutation,
        ),
        Job::Control(request, _) => {
            let resource = match request.as_ref() {
                Request::EditorSave { session }
                | Request::EditorStatus { session }
                | Request::EditorCompare { session }
                | Request::Stop { session }
                | Request::Rename { session, .. }
                | Request::Focus { session } => format!("editor:{session}"),
                _ => "workspace".into(),
            };
            (
                "daemon",
                resource,
                if async_client::read_only(request) {
                    Policy::ReplaceableRead
                } else {
                    Policy::OrderedMutation
                },
            )
        }
        Job::CloseEditors(_, ids, _, _) => (
            "daemon",
            format!("editor:{}", ids.join(",")),
            Policy::OrderedMutation,
        ),
        Job::Diff(tab) | Job::DiffContext(tab, _) | Job::DiffExpand(tab, ..) => {
            ("diff", tab.key(), Policy::ReplaceableRead)
        }
        Job::DiffAction(tab, _) => ("diff-action", tab.key(), Policy::OrderedMutation),
        Job::ResolveTarget(key, _, _) => ("files", key.clone(), Policy::ReplaceableRead),
        Job::GitView(tab) => ("git-view", tab.key(), Policy::ReplaceableRead),
        Job::LinePreviews(..) => ("files", "line-previews".into(), Policy::ReplaceableRead),
        Job::CommitFiles(_, hash) => (
            "git",
            format!("commit-files:{hash}"),
            Policy::ReplaceableRead,
        ),
        Job::GitBase(path, _) => (
            "git",
            format!("base:{}", path.display()),
            Policy::ReplaceableRead,
        ),
        Job::EditorBlame(path, ..) => (
            "git",
            format!("blame:{}", path.display()),
            Policy::ReplaceableRead,
        ),
        Job::HookStatus => ("catalog", "hooks".into(), Policy::ReplaceableRead),
        Job::TestNtfy { .. } => ("platform", "ntfy-test".into(), Policy::ReplaceableRead),
        Job::Preferences(_) => ("catalog", "preferences".into(), Policy::OrderedMutation),
        Job::SaveAppearance(..) => ("catalog", "appearance".into(), Policy::OrderedMutation),
        Job::RemoteRead { project, operation } => (
            "remote-read",
            format!(
                "{project}:{}",
                if matches!(operation, terminator_core::remote::Operation::List { .. }) {
                    "files"
                } else {
                    "git"
                }
            ),
            Policy::ReplaceableRead,
        ),
        Job::RemoteAsset { project, path, .. } => (
            "remote-file",
            format!("{project}:{path}"),
            Policy::OrderedMutation,
        ),
        Job::RemoteAdd { probe, .. } => (
            "remote-project",
            if *probe { "probe".into() } else { "add".into() },
            if *probe {
                Policy::ReplaceableRead
            } else {
                Policy::OrderedMutation
            },
        ),
        Job::HostKey { ssh, keys } => (
            "remote-host-key",
            ssh.host.clone(),
            if keys.is_some() {
                Policy::OrderedMutation
            } else {
                Policy::ReplaceableRead
            },
        ),
        Job::SshConfigHosts(_) => ("ssh-config", "hosts".into(), Policy::ReplaceableRead),
        Job::SshControlStatus(_) => ("ssh-control", "status".into(), Policy::ReplaceableRead),
        Job::Forwards(request) => match crate::forwards_ui::forward_key(request) {
            Some((project, port)) => (
                "ssh-forward",
                format!("{project}:{port}"),
                Policy::OrderedMutation,
            ),
            None => ("ssh-forward", "status".into(), Policy::ReplaceableRead),
        },
        Job::RemoteTransfer { project, .. } => {
            ("remote-transfer", project.clone(), Policy::OrderedMutation)
        }
        Job::RemoteTransferPoll { project, id } => (
            "remote-transfer-status",
            format!("{project}:{id}"),
            Policy::ReplaceableRead,
        ),
        Job::VncPickTransfer { session, .. } => {
            ("vnc-transfer", session.clone(), Policy::OrderedMutation)
        }
        Job::VncTransferPoll { session, .. } => (
            "vnc-transfer-status",
            session.clone(),
            Policy::ReplaceableRead,
        ),
        Job::VncFrame { session, .. } => {
            ("vnc", format!("frame:{session}"), Policy::ReplaceableRead)
        }
        Job::VncConnectTab { tab_id, .. } => (
            "daemon",
            format!("vnc-connect:{tab_id}"),
            Policy::OrderedMutation,
        ),
        _ => ("daemon", "workspace".into(), Policy::OrderedMutation),
    };
    let mut context = OperationContext::new(subsystem, resource, policy);
    context.session = editor_ids(job).first().cloned();
    match job {
        Job::CloseEditors(editor_close::Target::Workspace(project, tab), _, _, _) => {
            context.project = Some(project.clone());
            context.tab = Some(tab.clone());
        }
        Job::Control(request, after) => {
            if let Request::Create { project, .. }
            | Request::CreateReview { project, .. }
            | Request::SaveLayout { project, .. }
            | Request::SelectProject { project } = request.as_ref()
            {
                context.project = Some(project.clone());
            }
            if let After::Workspace(tab, _) = after {
                context.tab = Some(tab.clone());
            }
        }
        Job::OpenProject(_, generation) => context.generation = *generation,
        _ => {}
    }
    if matches!(
        job,
        Job::RemoteRead { .. }
            | Job::RemoteTransferPoll { .. }
            | Job::RemoteAdd { .. }
            | Job::HostKey { .. }
            | Job::Forwards(_)
            | Job::RemoteAsset { .. }
            | Job::VncFrame { .. }
    ) || matches!(
        job,
        Job::Diff(Tab::Diff {
            remote: Some(_),
            ..
        }) | Job::DiffContext(
            Tab::Diff {
                remote: Some(_),
                ..
            },
            _
        )
    ) {
        context.deadline =
            std::time::Instant::now().checked_add(std::time::Duration::from_secs(45));
    }
    context
}

#[derive(Clone)]
pub struct ImageJobs(pub Services);
impl ImageJobs {
    pub fn try_send(
        &self,
        (path, generation): (PathBuf, u64),
    ) -> Result<CancellationToken, async_service::Failure> {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let service = self.0.clone();
        let mut context = OperationContext::new(
            "images",
            path.to_string_lossy().into_owned(),
            Policy::ReplaceableRead,
        );
        context.generation = generation;
        self.0
            .handle()
            .submit(context, cancel.clone(), async move {
                let result = image_preview::load(&service, path.clone(), &token)
                    .await
                    .map_err(|e| format!("{e:#}"));
                Ok(vec![Update::Image(path, generation, result)])
            })?;
        Ok(cancel)
    }
}
pub(in crate::gui_services) use git::DiffExpandInput;
