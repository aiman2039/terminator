//! Remote-apps connect and state long-poll job.
use super::super::submit::Services;
use crate::Update;
use anyhow::Result;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

/// Longest one state poll waits in the daemon for a change.
const WAIT_MS: u32 = 400;

pub(crate) struct RemoteAppsInput {
    pub(in crate::gui_services) project: String,
    pub(in crate::gui_services) session: Option<String>,
    pub(in crate::gui_services) since: u64,
    pub(in crate::gui_services) cancel: CancellationToken,
}

impl Services {
    pub(in crate::gui_services) async fn remote_apps(
        &self,
        input: RemoteAppsInput,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let RemoteAppsInput {
            project,
            session,
            since,
            cancel,
        } = input;
        let client = self.client();
        let runtime = client.paths.runtime.clone();
        let request = match session {
            None => Request::RemoteAppsConnect {
                project: project.clone(),
            },
            Some(session) => Request::RemoteAppsState {
                session,
                since,
                wait_ms: WAIT_MS,
            },
        };
        let result = match client.rpc(request).await {
            Ok(Response::RemoteApps(state)) => self
                .cpu()
                .run(&cancel, move || {
                    crate::remote_apps_ui::decode(*state, since, &runtime)
                })
                .await
                .map(|frame| Some(Box::new(frame)))
                .map_err(|e: anyhow::Error| format!("{e:#}")),
            Ok(Response::Unchanged) => Ok(None),
            Ok(_) => Err("Unexpected remote-apps response".into()),
            Err(err) => Err(format!("{err}", err = terminator_core::ErrorChain(&err))),
        };
        updates.push(Update::RemoteApps(project, result));
        Ok(())
    }
}
