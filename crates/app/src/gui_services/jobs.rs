use super::submit::Services;
use crate::{After, Job, Update, editor_close, image_preview};
use anyhow::Result;
use std::path::PathBuf;
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_core::{Request, async_client, async_service};

pub(super) fn editor_ids(job: &Job) -> Vec<String> {
    let mut ids = match job {
        Job::CloseEditors(_, ids, _, _) => ids.clone(),
        Job::Control(request, _) => match request.as_ref() {
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
        Job::Diff(tab) => vec![Update::Diff(tab.key(), Err(busy()))],
        Job::ResolveTarget(key, _, _) => vec![Update::ResolvedTarget(key.clone(), None)],
        Job::ExitSave(id, _) => vec![Update::ExitSaved(*id, Err(busy()))],
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
        Job::Diff(tab) => ("diff", tab.key(), Policy::ReplaceableRead),
        Job::ResolveTarget(key, _, _) => ("files", key.clone(), Policy::ReplaceableRead),
        Job::HookStatus => ("catalog", "hooks".into(), Policy::ReplaceableRead),
        Job::TestNtfy { .. } => ("platform", "ntfy-test".into(), Policy::ReplaceableRead),
        Job::Preferences(_) => ("catalog", "preferences".into(), Policy::OrderedMutation),
        Job::SaveAppearance(..) => ("catalog", "appearance".into(), Policy::OrderedMutation),
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
