//! Job routing: scheduling context, editor serialization keys, and busy replies.
use crate::{After, Job, Tab, editor_close};
use terminator_core::async_service::{OperationContext, Policy};
use terminator_core::{Request, async_client};

/// Scheduling route: subsystem, resource key, and policy.
type Route = (&'static str, String, Policy);

/// The queue of one root's Git and file actions. A commit job is admitted
/// there too, so every write queued before it has landed when Git commits.
fn workspace_route(root: &std::path::Path) -> Route {
    (
        "workspace-op",
        root.display().to_string(),
        Policy::OrderedMutation,
    )
}

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

    /// What a job refused at submit answers its view; see [`super::replies`].
    #[cfg(test)]
    pub(in crate::gui_services) fn rejection(job: &Job) -> Vec<crate::Update> {
        super::replies::owed(job).map_or_else(Vec::new, |reply| reply(Self::busy()))
    }

    pub(in crate::gui_services) fn busy() -> String {
        "Services are busy; the action was not accepted. Retry it.".to_string()
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
        if matches!(
            job,
            Job::Workspace(
                _,
                crate::workspace_ops::Op::Commit { .. } | crate::workspace_ops::Op::PerforceRun(_)
            )
        ) {
            // A queued commit still runs and replies, however long the
            // actions before it take; the Git panel waits for that reply.
            context.deadline = None;
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
            // Diff and gutter writes queue with their repository's other
            // actions, where a commit is admitted: one sent before Commit
            // lands before Git commits (and its hooks run).
            Job::DiffAction(tab, request) => match request.root() {
                Some(root) => workspace_route(root),
                None => ("diff-action", tab.key(), Policy::OrderedMutation),
            },
            Job::StageGutterBlock { cwd, .. } => workspace_route(cwd),
            Job::ResolveTarget(key, _, _) => ("files", key.clone(), Policy::ReplaceableRead),
            Job::GitView(tab) => ("git-view", tab.key(), Policy::ReplaceableRead),
            // A newer filter or Go to Hash replaces the query still running.
            Job::GitLog(tab, _) => (
                crate::git_log::LOG_SUBSYSTEM,
                tab.key(),
                Policy::ReplaceableRead,
            ),
            Job::LocalHistory(op) => crate::local_history::route(op),
            Job::Fonts(op) => crate::appearance::fonts::route(op),
            Job::LinePreviews(..) => ("files", "line-previews".into(), Policy::ReplaceableRead),
            Job::WorktreeProbe(path, _) => (
                "git",
                format!("worktree-probe:{}", path.display()),
                Policy::ReplaceableRead,
            ),
            // A newer ask for the same worktree replaces the one queued.
            Job::WorktreeStats(path) => (
                "git",
                format!("worktree-stats:{}", path.display()),
                Policy::ReplaceableRead,
            ),
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
            _ => return Self::files_route(job),
        };
        Some(route)
    }

    /// Local file, Git, search and platform jobs. They never queue behind
    /// daemon Control (`daemon`/`workspace`): a slow commit+push or a
    /// full-text search must not hold back a new terminal.
    fn files_route(job: &Job) -> Option<Route> {
        let route = match job {
            Job::Workspace(root, _) => workspace_route(root),
            // A newer query cancels the queued or running walk (#175).
            Job::Search { root, .. } => (
                "search",
                root.display().to_string(),
                Policy::ReplaceableRead,
            ),
            Job::Favicon(url) => ("favicon", url.clone(), Policy::OrderedMutation),
            Job::PasteClipboard(_) => ("platform", "clipboard".into(), Policy::OrderedMutation),
            Job::Browser(_) | Job::External(_) | Job::TestExternal(..) => {
                ("platform", "open".into(), Policy::OrderedMutation)
            }
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
            Job::RemoteApps { project, .. } => (
                "remote-apps",
                format!("apps:{project}"),
                Policy::ReplaceableRead,
            ),
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
            Job::GitLog(_, query) => context.generation = query.generation,
            // A failed diff or gutter write answers its tab or gutter.
            Job::DiffAction(tab, _) => context.tab = Some(tab.key()),
            Job::StageGutterBlock { path, .. } => {
                context.tab = Some(format!("gutter:{}", path.display()));
            }
            // A failed service job still ends its operation in the GUI.
            _ => context.tab = crate::service_op::job_tag(job),
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
                | Job::RemoteApps { .. }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Update;
    use std::path::PathBuf;

    fn key(job: &Job) -> (&'static str, String) {
        let context = JobRouting::context(job);
        (context.subsystem, context.resource)
    }

    #[test]
    fn slow_local_jobs_do_not_share_the_daemon_create_queue() {
        let create = key(&Job::Control(
            Box::new(Request::Create {
                project: "p".into(),
                cwd: None,
                file: None,
                line: None,
                column: None,
                editor: false,
            }),
            After::Create(None),
        ));
        let root = PathBuf::from("/repo");
        let slow = [
            Job::Workspace(
                root.clone(),
                crate::workspace_ops::Op::Stage(root.join("a")),
            ),
            Job::Search {
                id: 1,
                root: root.clone(),
                query: crate::search::Query::default(),
                show_ignored: false,
            },
            Job::Favicon("https://example.com".into()),
            Job::PasteClipboard("s".into()),
            Job::Browser("https://example.com".into()),
            Job::External(root.join("a")),
            Job::TestExternal(root, "true".into(), Vec::new()),
        ];
        for job in &slow {
            assert_ne!(key(job), create);
        }
    }

    #[test]
    fn commit_waits_for_earlier_root_actions_without_a_queue_deadline() {
        let root = PathBuf::from("/repo");
        let commit = Job::Workspace(
            root.clone(),
            crate::workspace_ops::Op::Commit {
                message: "m".into(),
                amend: false,
                signoff: false,
                push: true,
                cancel: terminator_core::async_service::CancellationToken::new(),
            },
        );
        let stage = Job::Workspace(
            root.clone(),
            crate::workspace_ops::Op::Stage(root.join("a")),
        );
        // Same queue: a Stage clicked before Commit finishes first.
        assert_eq!(key(&commit), key(&stage));
        assert!(JobRouting::context(&commit).deadline.is_none());
        assert!(JobRouting::context(&stage).deadline.is_some());
        // A refused commit still replies, so the panel leaves "Committing…".
        assert!(matches!(
            JobRouting::rejection(&commit).as_slice(),
            [Update::Workspace(
                _,
                crate::workspace_ops::Op::Commit { .. },
                Err(_)
            )]
        ));
        assert!(JobRouting::rejection(&stage).is_empty());
    }

    #[test]
    fn a_refused_compare_query_still_answers_the_popup() {
        use crate::workspace_ops::Op;
        let folder = PathBuf::from("/repo/src");
        for op in [Op::CompareTargets, Op::VerifyRevision("HEAD~1".into())] {
            let refused = JobRouting::rejection(&Job::Workspace(folder.clone(), op));
            assert!(
                matches!(refused.as_slice(), [Update::Workspace(_, _, Err(_))]),
                "the popup leaves its spinner"
            );
        }
    }
}
