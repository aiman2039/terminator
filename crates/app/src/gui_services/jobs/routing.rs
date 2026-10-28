//! Job routing: scheduling context, editor serialization keys, and busy replies.
use crate::{After, Job, Tab, Update, editor_close};
use terminator_core::async_service::{OperationContext, Policy};
use terminator_core::{Request, async_client};

/// Scheduling route: subsystem, resource key, and policy.
type Route = (&'static str, String, Policy);

/// Stateless routing rules for GUI service jobs.
pub(in crate::gui_services) struct JobRouting;

impl JobRouting {
    pub(in crate::gui_services) fn editor_ids(job: &Job) -> Vec<String> {
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

    pub(in crate::gui_services) fn rejection(job: &Job) -> Vec<Update> {
        Self::remote_rejection(job).unwrap_or_else(|| Self::local_rejection(job))
    }

    fn busy() -> String {
        "Services are busy; the action was not accepted. Retry it.".to_string()
    }

    fn remote_rejection(job: &Job) -> Option<Vec<Update>> {
        let busy = Self::busy;
        let updates = match job {
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
            Job::SshConfigHosts(generation) => {
                vec![Update::SshConfigHosts(*generation, Vec::new())]
            }
            Job::SshControlStatus(_) => vec![Update::SshControlStatus(Vec::new())],
            Job::Forwards(request) => vec![Update::Forwards(
                crate::forwards_ui::forward_key(request),
                Err(busy().into()),
            )],
            _ => return None,
        };
        Some(updates)
    }

    fn local_rejection(job: &Job) -> Vec<Update> {
        let busy = Self::busy;
        match job {
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

    pub(in crate::gui_services) fn context(job: &Job) -> OperationContext {
        let (subsystem, resource, policy) = Self::local_route(job)
            .or_else(|| Self::remote_route(job))
            .unwrap_or_else(|| ("daemon", "workspace".into(), Policy::OrderedMutation));
        let mut context = OperationContext::new(subsystem, resource, policy);
        context.session = Self::editor_ids(job).first().cloned();
        Self::scope_context(job, &mut context);
        if Self::remote_deadline(job) {
            context.deadline =
                std::time::Instant::now().checked_add(std::time::Duration::from_secs(45));
        }
        context
    }

    fn local_route(job: &Job) -> Option<Route> {
        let route = match job {
            Job::PrepareLayouts(_, _) => ("layout", "snapshot".into(), Policy::ReplaceableRead),
            Job::SaveLayout(project, _, _) => (
                "daemon",
                format!("layout:{project}"),
                Policy::OrderedMutation,
            ),
            Job::Control(request, _) => Self::control_route(request),
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
            _ => return None,
        };
        Some(route)
    }

    fn control_route(request: &Request) -> Route {
        let resource = match request {
            Request::EditorSave { session }
            | Request::EditorStatus { session }
            | Request::EditorCompare { session }
            | Request::Stop { session }
            | Request::Rename { session, .. }
            | Request::Focus { session } => format!("editor:{session}"),
            _ => "workspace".into(),
        };
        let policy = if async_client::read_only(request) {
            Policy::ReplaceableRead
        } else {
            Policy::OrderedMutation
        };
        ("daemon", resource, policy)
    }

    fn remote_route(job: &Job) -> Option<Route> {
        let route = match job {
            Job::RemoteRead { project, operation } => {
                let kind = if matches!(operation, terminator_core::remote::Operation::List { .. }) {
                    "files"
                } else {
                    "git"
                };
                (
                    "remote-read",
                    format!("{project}:{kind}"),
                    Policy::ReplaceableRead,
                )
            }
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
            _ => return Self::transfer_route(job),
        };
        Some(route)
    }

    fn transfer_route(job: &Job) -> Option<Route> {
        let route = match job {
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
            _ => return None,
        };
        Some(route)
    }

    fn scope_context(job: &Job, context: &mut OperationContext) {
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
    }

    fn remote_deadline(job: &Job) -> bool {
        matches!(
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
        )
    }
}
