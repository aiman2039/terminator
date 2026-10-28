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
mod routing;
pub(super) use routing::JobRouting;
mod vnc;
pub(crate) use vnc::VncFrameInput;
pub(crate) use vnc::VncPickTransferInput;

use super::submit::Services;
use crate::{Update, image_preview};
use anyhow::Result;
use std::path::PathBuf;
use terminator_core::async_service;
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

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
