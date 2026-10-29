//! UI control requests from the local control socket and fixtures.

use crate::app::OpenFileModeInput;
use anyhow::{Context, Result};
use terminator_core::ui_control::Request as Ui;

use crate::*;

mod snapshot;

/// What a request handler wants sent back: `Some` replies at once, `None`
/// saves layouts, repaints, and reports the request accepted.
type Reply = Option<serde_json::Value>;

impl App {
    pub(crate) fn ui_request(
        &mut self,
        ctx: &egui::Context,
        request: terminator_core::ui_control::Request,
    ) -> Result<serde_json::Value> {
        request.validate()?;
        anyhow::ensure!(!self.exit.active(), "Terminator is saving before closing");
        let reply = match request {
            #[cfg(not(feature = "test-support"))]
            Ui::FixturePlayer { .. } => anyhow::bail!("Player fixture control is disabled"),
            #[cfg(feature = "test-support")]
            Ui::FixturePlayer { action, url } => self.ui_fixture_player(&action, url)?,
            Ui::Ping => {
                Some(serde_json::json!({"capabilities":[terminator_core::ui_control::CAPABILITY]}))
            }
            Ui::Snapshot => Some(self.ui_snapshot(ctx)?),
            Ui::Focus { session } => self.ui_focus(&session)?,
            Ui::ShowSession {
                session,
                anchor,
                split,
            } => self.ui_show_session(session, anchor, split)?,
            Ui::OpenFile {
                project,
                path,
                as_text,
            } => self.ui_open_file(project, path, as_text)?,
            Ui::OpenBrowser { url } => {
                let _ = self.jobs.send(Job::Browser(metadata::http_url(&url)?));
                None
            }
            Ui::Window { action } => {
                self.ui_window(ctx, &action);
                None
            }
        };
        if let Some(reply) = reply {
            return Ok(reply);
        }
        self.save_layouts();
        ctx.request_repaint();
        Ok(serde_json::json!({"accepted":true}))
    }

    #[cfg(feature = "test-support")]
    fn ui_fixture_player(&mut self, action: &str, url: Option<String>) -> Result<Reply> {
        anyhow::ensure!(
            std::env::var_os("TERMINATOR_TEST_RESPONSIVENESS").is_some(),
            "Player fixture control is disabled"
        );
        match action {
            "play" => self.player.fixture_play(
                self.selected.as_deref().unwrap_or("fixture"),
                url.context("Missing fixture URL")?,
            ),
            "pause" => self.player.pause(),
            "resume" => self.player.resume(),
            "stop" => self.player.stop(),
            _ => anyhow::bail!("Invalid player fixture action"),
        }
        Ok(None)
    }

    fn ui_focus(&mut self, session: &str) -> Result<Reply> {
        anyhow::ensure!(
            self.state.sessions.iter().any(|s| s.id == session),
            "Unknown session"
        );
        self.go_session(session);
        Ok(None)
    }

    fn ui_show_session(
        &mut self,
        session: String,
        anchor: Option<String>,
        split: Option<String>,
    ) -> Result<Reply> {
        let record = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == session && s.lifecycle.live())
            .context("Unknown live session")?
            .clone();
        anyhow::ensure!(
            !self.layout_readonly.contains(&record.project_id),
            "Project has an unsupported layout version"
        );
        let tab = Tab::Terminal(session.clone());
        if self
            .layouts
            .get(&record.project_id)
            .is_some_and(|d| d.contains(&tab))
        {
            self.go_session(&session);
            return Ok(Some(serde_json::json!({"accepted":true,"existing":true})));
        }
        if let Some(anchor) = anchor {
            self.focus_anchor_pane(&record.project_id, anchor)?;
            self.insert(&record.project_id, tab, split.as_deref());
        } else {
            self.layouts
                .entry(record.project_id.clone())
                .or_insert_with(Workspace::empty)
                .add(id(), tab);
        }
        self.select_project(record.project_id);
        self.active_session = Some(session);
        Ok(None)
    }

    /// Focuses the pane of `anchor`, a session in `project`, so the next
    /// insert lands beside it.
    fn focus_anchor_pane(&mut self, project: &str, anchor: String) -> Result<()> {
        anyhow::ensure!(
            self.state
                .sessions
                .iter()
                .any(|s| s.id == anchor && s.project_id == project),
            "Anchor belongs to a different project"
        );
        let dock = self
            .layouts
            .get_mut(project)
            .context("Missing project layout")?;
        let anchor = Tab::Terminal(anchor);
        anyhow::ensure!(
            dock.activate_containing(&anchor),
            "Anchor is not in a visible workspace"
        );
        let path = dock.find_tab(&anchor).context("Missing anchor pane")?;
        dock.set_focused_node_and_surface(path.node_path());
        Ok(())
    }

    fn ui_open_file(
        &mut self,
        project: String,
        path: std::path::PathBuf,
        as_text: bool,
    ) -> Result<Reply> {
        anyhow::ensure!(
            self.state.projects.iter().any(|p| p.id == project),
            "Unknown project"
        );
        anyhow::ensure!(
            !self.layout_readonly.contains(&project),
            "Project has an unsupported layout version"
        );
        self.select_project(project);
        self.open_file_mode(OpenFileModeInput {
            path,
            line: None,
            split: None,
            external: false,
            text: as_text,
        });
        Ok(None)
    }

    fn ui_window(&mut self, ctx: &egui::Context, action: &str) {
        ctx.send_viewport_cmd(match action {
            "minimize" => egui::ViewportCommand::Minimized(true),
            "maximize" => egui::ViewportCommand::Maximized(true),
            "restore" => {
                self.exit_vnc_fullscreen(ctx);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                egui::ViewportCommand::Maximized(false)
            }
            "close" => egui::ViewportCommand::Close,
            _ => egui::ViewportCommand::Focus,
        });
    }

    pub(crate) fn send(&self, request: Request) {
        let _ = self.jobs.send(Job::rpc(request, After::None));
    }
}
