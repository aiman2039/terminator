//! The reply a job owes the view that is waiting for it, when the job ends
//! without its own: refused at submit, timed out, panicked, failed, or
//! cancelled with no newer request for the same key. Each reply clears
//! the waiting flag (spinner, `pending`, `busy`) its view set.
use crate::{Job, Update};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use terminator_core::async_service::{Failure, OperationContext, OperationId};

/// Builds a job's failure reply from the error text.
pub(crate) type Reply = Box<dyn FnOnce(String) -> Vec<Update> + Send>;

fn reply<T: Send + 'static>(value: T, build: fn(T, String) -> Vec<Update>) -> Option<Reply> {
    Some(Box::new(move |error| build(value, error)))
}

/// The failure reply of `job`, or `None` when nothing waits for it.
pub(crate) fn owed(job: &Job) -> Option<Reply> {
    remote(job).or_else(|| local(job)).or_else(|| waiting(job))
}

fn remote(job: &Job) -> Option<Reply> {
    match job {
        Job::RemoteAdd {
            project,
            generation,
            ..
        } => reply((project.clone(), *generation), |(p, g), e| {
            vec![Update::RemoteAdded(p, g, Err(e.into()))]
        }),
        Job::RemoteRead { project, operation } => {
            reply((project.clone(), operation.clone()), |(p, o), e| {
                vec![Update::RemoteReply(p, o, Err(e.into()))]
            })
        }
        Job::RemoteTransfer { project, .. } => reply(project.clone(), |p, e| {
            vec![Update::RemoteTransfer(p, None, Err(e.into()))]
        }),
        Job::HostKey { ssh, keys: None } => {
            reply(ssh.clone(), |s, e| vec![Update::HostKeyScanned(s, Err(e))])
        }
        Job::HostKey { ssh, keys: Some(_) } => {
            reply(ssh.clone(), |s, e| vec![Update::HostKeyTrusted(s, Err(e))])
        }
        Job::SshConfigHosts(generation) => reply(*generation, |g, _| {
            vec![Update::SshConfigHosts(g, Vec::new())]
        }),
        Job::SshControlStatus(_) => reply((), |(), _| vec![Update::SshControlStatus(Vec::new())]),
        Job::Forwards(request) => reply(crate::forwards_ui::forward_key(request), |k, e| {
            vec![Update::Forwards(k, Err(e.into()))]
        }),
        Job::VncFrame { session, .. } => {
            reply(session.clone(), |s, e| vec![Update::VncFrame(s, Err(e))])
        }
        _ => None,
    }
}

fn local(job: &Job) -> Option<Reply> {
    match job {
        Job::SaveLayout(project, _, text) => reply((project.clone(), text.clone()), |(p, t), e| {
            vec![Update::LayoutSaved(p, t, Err(e))]
        }),
        Job::CloseEditors(target, ids, _, _) => {
            reply((target.clone(), ids.clone()), |(t, i), e| {
                vec![Update::EditorsClosed(t, i, Err(e))]
            })
        }
        Job::CloseIdle(target, _, ids) => reply((target.clone(), ids.clone()), |(t, i), e| {
            vec![Update::IdleClosed(t, i, Err(e))]
        }),
        Job::Preferences(_) => reply((), |(), e| vec![Update::PreferencesSaved(Box::new(Err(e)))]),
        Job::RepairInstallation(..) => {
            reply((), |(), e| vec![Update::InstallationRepaired(Err(e))])
        }
        Job::RestartSessionService(_) => reply((), |(), e| vec![Update::RestartFinished(e)]),
        Job::StartSessionService => reply((), |(), e| vec![Update::ServiceStarted(Err(e))]),
        Job::Diff(tab) | Job::DiffContext(tab, _) | Job::DiffExpand(tab, ..) => {
            reply(tab.key(), |k, e| vec![Update::Diff(k, Err(e))])
        }
        Job::DiffAction(tab, _) => reply(tab.key(), |k, e| vec![Update::DiffActionFailed(k, e)]),
        Job::StageGutterBlock { path, .. } => {
            reply(path.clone(), |p, e| vec![Update::GutterStaged(p, Err(e))])
        }
        Job::ResolveTarget(key, _, _) => {
            reply(key.clone(), |k, _| vec![Update::ResolvedTarget(k, None)])
        }
        Job::ExitSave(id, _) => reply(*id, |i, e| vec![Update::ExitSaved(i, Err(e))]),
        _ => None,
    }
}

/// Views and popups that wait for their answer.
fn waiting(job: &Job) -> Option<Reply> {
    match job {
        Job::GitView(tab) => reply(tab.clone(), |t, e| {
            vec![Update::GitView(t.key(), crate::GitView::failed(&t, e))]
        }),
        Job::GitLog(tab, query) => reply((tab.key(), query.clone()), |(k, q), e| {
            vec![Update::GitView(k, crate::GitView::Log(q.failed(e)))]
        }),
        Job::LocalHistory(op) => reply(op.clone(), |o, e| vec![Update::LocalHistory(o.failed(e))]),
        // The Git panel waits for its commit; the Compare popup for its
        // answer; Explorer undo for its renames.
        Job::Workspace(root, op)
            if matches!(
                op,
                crate::workspace_ops::Op::Commit { .. } | crate::workspace_ops::Op::Rename { .. }
            ) || crate::revision_compare::answers(op) =>
        {
            reply((root.clone(), op.clone()), |(r, o), e| {
                vec![Update::Workspace(r, o, Err(e))]
            })
        }
        Job::Search { id, .. } => reply(*id, |i, e| vec![Update::Search(i, Err(e))]),
        Job::GitBase(path, generation) => reply((path.clone(), *generation), |(p, g), _| {
            vec![Update::GitBaseFailed(p, g)]
        }),
        _ => created(job),
    }
}

/// Jobs that create something the GUI waits to place.
fn created(job: &Job) -> Option<Reply> {
    match job {
        // The new tab's insert index and post-create input wait for it.
        Job::Control(request, crate::After::Workspace(tab, _))
            if matches!(**request, terminator_core::Request::Create { .. }) =>
        {
            reply(tab.clone(), |t, e| vec![Update::CreateFailed(t, e)])
        }
        // The worktree wizard waits in its Creating state.
        Job::CreateWorktree(draft) => reply(draft.id, |d, e| vec![Update::WorktreeFailed(d, e)]),
        // The layout signature waits for the prepared layouts.
        Job::PrepareLayouts(generation, _) => {
            reply(*generation, |g, _| vec![Update::LayoutPrepareFailed(g)])
        }
        _ => None,
    }
}

/// One submitted job that owes a reply.
struct Entry {
    key: (&'static str, String),
    reply: Reply,
}

/// Submitted jobs that owe a reply, by operation. The newest operation
/// per key tells a replaced read (its successor answers) from a read
/// cancelled with nothing after it (it still owes its reply).
#[derive(Default)]
pub(crate) struct Owed {
    entries: HashMap<OperationId, Entry>,
    newest: HashMap<(&'static str, String), OperationId>,
}

impl Owed {
    pub(crate) fn record(&mut self, id: OperationId, context: &OperationContext, reply: Reply) {
        let key = (context.subsystem, context.resource.clone());
        self.newest.insert(key.clone(), id);
        self.entries.insert(id, Entry { key, reply });
    }

    /// The updates owed for the operation that ended with `result`, if
    /// any. Success and a replaced read owe nothing.
    pub(crate) fn settle(&mut self, id: OperationId, result: Result<(), &Failure>) -> Vec<Update> {
        let Some(entry) = self.entries.remove(&id) else {
            return Vec::new();
        };
        let newest = self.newest.get(&entry.key) == Some(&id);
        if newest {
            self.newest.remove(&entry.key);
        }
        match result {
            Ok(()) => Vec::new(),
            Err(Failure::Cancelled) if !newest => Vec::new(),
            Err(failure) => (entry.reply)(failure.to_string()),
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// [`Owed`] shared between the submitting threads and the GUI.
#[derive(Default)]
pub(crate) struct SharedOwed(pub(crate) Mutex<Owed>);

impl SharedOwed {
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Owed> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
