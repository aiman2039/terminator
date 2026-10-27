//! Daemon session control, layouts, exit, and service lifecycle jobs.
use super::super::submit::Services;
use crate::workspace::Workspace;
use crate::{After, Update, daemon_connection, editor_close, exit, installation, recovery};
use anyhow::Result;
use std::{fs, time::Duration};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_core::{Request, Response, sanitize_layout};

impl Services {
    /// Reuse a live review session for the same file and side instead of
    /// creating another: returns the updates that reveal it, if any.
    pub(in crate::gui_services) async fn reuse_review(
        &self,
        request: &Request,
        cancel: &CancellationToken,
    ) -> Result<Option<Vec<Update>>> {
        let Request::CreateReview {
            project,
            path,
            staged,
            ..
        } = request
        else {
            return Ok(None);
        };
        let client = self.client();
        let Response::State(state) = client.snapshot(None).await? else {
            anyhow::bail!("Expected review session inventory");
        };
        let candidates: Vec<_> = state
            .sessions
            .iter()
            .filter(|session| {
                session.project_id == *project
                    && session.review
                    && session.lifecycle.live()
                    && session.file.as_ref() == Some(path)
                    && daemon_connection::session_owner_available(&state, session)
            })
            .map(|session| session.id.clone())
            .collect();
        let paths = client.paths.clone();
        let staged = *staged;
        let existing = client
            .catalog
            .run(cancel, move || {
                for session in candidates {
                    let owner = terminator_core::generations::owner_for(
                        &paths,
                        &Request::EditorStatus {
                            session: session.clone(),
                        },
                    )?;
                    let config = owner.runtime.join(format!("review-{session}/config.json"));
                    if !fs::metadata(&config).is_ok_and(|metadata| metadata.len() <= 65536) {
                        continue;
                    }
                    let Ok(bytes) = fs::read(config) else {
                        continue;
                    };
                    let Ok(config) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                        continue;
                    };
                    if config.get("left_label").and_then(serde_json::Value::as_str)
                        == Some(if staged { "HEAD" } else { "Index" })
                        && config
                            .get("right_label")
                            .and_then(serde_json::Value::as_str)
                            == Some(if staged { "Index" } else { "Working tree" })
                    {
                        return Ok(Some(session));
                    }
                }
                Ok(None)
            })
            .await?;
        if let Some(session) = existing {
            return Ok(Some(vec![
                Update::State(state),
                Update::RevealFileSession(session, project.clone()),
            ]));
        }
        Ok(None)
    }

    pub(in crate::gui_services) async fn prepare_layouts(
        &self,
        generation: u64,
        layouts: Vec<(String, Workspace)>,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let layouts = self
            .cpu()
            .run(&cancel, move || {
                layouts
                    .into_iter()
                    .map(|(project, layout)| {
                        let value = sanitize_layout(serde_json::to_value(layout)?);
                        let text = value.to_string();
                        Ok((project, value, text))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .await?;
        updates.push(Update::LayoutsPrepared(generation, layouts));
        Ok(())
    }

    pub(in crate::gui_services) async fn save_layout(
        &self,
        project: String,
        layout: serde_json::Value,
        text: String,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = client
            .rpc(Request::SaveLayout {
                project: project.clone(),
                layout,
            })
            .await
            .and_then(|response| {
                anyhow::ensure!(
                    matches!(response, Response::Ok),
                    "Layout save was not acknowledged"
                );
                Ok(())
            })
            .map_err(|e| format!("{e:#}"));
        updates.push(Update::LayoutSaved(project, text, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn control(
        &self,
        request: Box<Request>,
        after: After,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        match client.rpc(*request).await? {
            Response::Created(session) => {
                if let Ok(Response::State(state)) = client.rpc(Request::Snapshot).await {
                    updates.push(Update::State(state));
                }
                match after {
                    After::Create(split) => updates.push(Update::Created(session, split, None)),
                    After::CreateAt(anchors, split) => {
                        updates.push(Update::Created(session, split, Some(anchors)));
                    }
                    After::Workspace(id, anchors) => {
                        updates.push(Update::WorkspaceCreated(session, id, anchors));
                    }
                    After::Strip => {
                        updates.push(Update::StripCreated(session, None, Vec::new()));
                    }
                    After::StripAt(anchors, split) => {
                        updates.push(Update::StripCreated(session, split, anchors));
                    }
                    After::Float(viewport, anchors, split) => {
                        updates.push(Update::FloatCreated(session, viewport, split, anchors));
                    }
                    _ => {}
                }
            }
            Response::VncCreated(session) => {
                if let Ok(Response::State(state)) = client.rpc(Request::Snapshot).await {
                    updates.push(Update::State(state));
                }
                updates.push(Update::VncConnected(session, after));
            }
            Response::Text(text) => {
                if let After::Text(key) = after {
                    updates.push(Update::Text(key, text));
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(in crate::gui_services) async fn close_idle(
        &self,
        target: editor_close::Target,
        generation: String,
        ids: Vec<String>,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = client
            .rpc(Request::CloseIdleSessions {
                generation,
                sessions: ids.clone(),
            })
            .await
            .and_then(|response| match response {
                Response::IdleSessionsClosed(outcomes) => Ok(outcomes),
                _ => anyhow::bail!("Unexpected idle-close response"),
            })
            .map_err(|e| format!("{e:#}"));
        updates.push(Update::IdleClosed(target, ids, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn close_editors(
        &self,
        target: editor_close::Target,
        ids: Vec<String>,
        mode: editor_close::Mode,
        timeout: Duration,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = editor_close::close_async(client, &ids, mode, timeout)
            .await
            .map_err(|e| format!("{e:#}"));
        updates.push(Update::EditorsClosed(target, ids, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn exit_save(
        &self,
        id: u64,
        checkpoint: exit::Checkpoint,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = async {
            // Quit pauses polling. Recheck owners here as the daemon can
            // have exited since the last accepted GUI snapshot.
            let Response::State(state) = client.snapshot(None).await? else {
                anyhow::bail!("Could not verify the session service before closing")
            };
            self.recover_service(*state, &cancel).await?;
            let Response::State(state) = client.snapshot(None).await? else {
                anyhow::bail!("Could not verify the recovered session service")
            };
            checkpoint.for_state(&state).save_async(client).await
        }
        .await;
        updates.push(Update::ExitSaved(id, result.map_err(|e| format!("{e:#}"))));
        Ok(())
    }

    pub(in crate::gui_services) async fn repair_installation(
        &self,
        generation: String,
        checkpoint: exit::Checkpoint,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = async {
            checkpoint.save_async(client).await?;
            let paths = client.paths.clone();
            self.platform()
                .run(&cancel, move || {
                    daemon_connection::repair(&paths, &std::env::current_exe()?, &generation)
                })
                .await
        }
        .await
        .map_err(|e| format!("{e:#}"));
        let mut result = result;
        if let Ok(state) = &mut result {
            client.observe(state);
        }
        updates.push(Update::InstallationRepaired(result));
        Ok(())
    }

    pub(in crate::gui_services) async fn restart_session_service(
        &self,
        inventory: recovery::RestartInventory,
        cancel: CancellationToken,
    ) -> Result<()> {
        let client = self.client();
        let paths = client.paths.clone();
        let watched = self
            .platform()
            .run(&cancel, move || {
                installation::begin_restart(
                    installation::restart_invocation(&std::env::current_exe()?, &paths)?,
                    inventory,
                )
            })
            .await?;
        let service = self.clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let mut context = OperationContext::new(
            "platform",
            "restart-observer".into(),
            Policy::ServiceLifetime,
        );
        context.deadline = None;
        self.handle().submit(context, cancel, async move {
            use tokio::io::AsyncReadExt;
            let mut completion = watched.completion.into_tokio()?;
            let mut bytes = [0; 256];
            loop {
                tokio::select! {
                    () = token.cancelled() => return Ok(Vec::new()),
                    n = completion.read(&mut bytes) => if n? == 0 { break; }
                }
            }
            let message = service
                .fs()
                .run(&token, move || {
                    Ok(installation::restart_message(
                        watched.log_path,
                        watched.log_start,
                    ))
                })
                .await?;
            Ok(vec![Update::RestartFinished(message)])
        })?;
        Ok(())
    }

    pub(in crate::gui_services) async fn start_session_service(
        &self,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let paths = client.paths.clone();
        let result = self
            .platform()
            .run(&cancel, move || {
                daemon_connection::ensure_running(&paths, &std::env::current_exe()?)
            })
            .await
            .map_err(|error| format!("{error:#}"));
        updates.push(Update::ServiceStarted(result));
        Ok(())
    }
}
