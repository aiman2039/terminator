use anyhow::{Context, Result};
use eframe::egui::{self};
#[cfg(feature = "test-support")]
use std::collections::HashMap;
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn ui_request(
        &mut self,
        ctx: &egui::Context,
        request: terminator_core::ui_control::Request,
    ) -> Result<serde_json::Value> {
        use terminator_core::ui_control::Request as Ui;
        request.validate()?;
        anyhow::ensure!(!self.exit.active(), "Terminator is saving before closing");
        let gui_ppp = ctx.pixels_per_point();
        match request {
            #[cfg(not(feature = "test-support"))]
            Ui::FixturePlayer { .. } => anyhow::bail!("Player fixture control is disabled"),
            #[cfg(feature = "test-support")]
            Ui::FixturePlayer { action, url } => {
                anyhow::ensure!(
                    std::env::var_os("TERMINATOR_TEST_RESPONSIVENESS").is_some(),
                    "Player fixture control is disabled"
                );
                match action.as_str() {
                    "play" => self.player.fixture_play(
                        self.selected.as_deref().unwrap_or("fixture"),
                        url.context("Missing fixture URL")?,
                    ),
                    "pause" => self.player.pause(),
                    "resume" => self.player.resume(),
                    "stop" => self.player.stop(),
                    _ => anyhow::bail!("Invalid player fixture action"),
                }
            }

            Ui::Ping => {
                return Ok(
                    serde_json::json!({"capabilities":[terminator_core::ui_control::CAPABILITY]}),
                );
            }
            Ui::Snapshot => {
                #[allow(unused_mut)]
                let mut snapshot = serde_json::json!({"selected_project":self.selected,"active_session":self.active_session,"workspaces":self.layouts,
                    "controls":{
                        "header-drag":self.fixture_rect(ctx,"header-drag"),
                        "project-add":self.fixture_rect(ctx,"project-add"),
                        "resize-se":self.fixture_rect(ctx,"window-resize-3")
                    },
                    "window":ctx.input(|i|serde_json::json!({"inner":i.viewport().inner_rect.map(|r|[r.min.x,r.min.y,r.width(),r.height()]),"outer":i.viewport().outer_rect.map(|r|[r.min.x,r.min.y,r.width(),r.height()]),"maximized":i.viewport().maximized,"minimized":i.viewport().minimized,"gui_ppp":gui_ppp,"native_ppp":i.viewport().native_pixels_per_point}))});
                let insert = |value: &mut serde_json::Value, key: &str, next: serde_json::Value| {
                    if let Some(object) = value.as_object_mut() {
                        object.insert(key.to_owned(), next);
                    }
                };
                insert(&mut snapshot, "services", self.services.diagnostics());
                if let Some(services) = snapshot.get_mut("services") {
                    insert(
                        services,
                        "ui_processing_peak_ms",
                        serde_json::json!(self.ui_service_peak_ms),
                    );
                }
                #[cfg(feature = "test-support")]
                {
                    insert(
                        &mut snapshot,
                        "updater_available",
                        serde_json::json!(self.updater.available()),
                    );
                    insert(
                        &mut snapshot,
                        "update_menu",
                        serde_json::json!(self.updater.menu_installed()),
                    );
                    insert(
                        &mut snapshot,
                        "installation",
                        serde_json::json!({
                            "connected":self.connected,
                            "problem":self.installation_problem(),
                            "repair_pending":self.repair_pending,
                            "restart_pending":self.restart_pending,
                            "restart_confirm":self.restart_confirm,
                            "can_repair":self.connected && can_retire_daemon(&self.state),
                            "can_restart":self.connected && can_restart_service(&self.state),
                            "settings_visible":self.settings_open && self.settings_section == SettingsSection::Updates,
                            "generation":self.state.generation,
                            "generations":self.state.generations,
                            "live_count":self.state.sessions.iter().filter(|s| s.lifecycle.live()).count(),
                            "error":self.error,
                        }),
                    );
                    insert(
                        &mut snapshot,
                        "attention",
                        serde_json::json!(ctx.data(|data| {
                            data.get_temp::<(usize, bool)>(egui::Id::new("attention-state"))
                        })),
                    );
                    insert(
                        &mut snapshot,
                        "left_agents",
                        serde_json::json!(self.preferences.left_agents),
                    );
                    insert(
                        &mut snapshot,
                        "agent_bar_badge",
                        serde_json::json!(ctx.data(|data| {
                            data.get_temp::<String>(egui::Id::new("agent-bar-badge"))
                        })),
                    );
                    insert(&mut snapshot, "markdown", self.markdown.diagnostics());
                    insert(
                        &mut snapshot,
                        "markdown_modes",
                        serde_json::to_value(&self.preferences.markdown_modes)?,
                    );
                    insert(
                        &mut snapshot,
                        "visible_terminals",
                        serde_json::to_value(&self.visible_sessions)?,
                    );
                    insert(
                        &mut snapshot,
                        "fixture_actions_completed",
                        serde_json::json!(self.diagnostics.actions_completed()),
                    );
                    insert(
                        &mut snapshot,
                        "terminal_scroll",
                        serde_json::json!(self.backends.iter().map(|(sid, backend)| {
                            let content = backend.last_content();
                            let text: String = content.grid.display_iter().map(|cell| cell.c).collect();
                            let samples: Vec<_> = (1..=160).filter(|n| text.contains(&format!("TSAMPLE{n:03}"))).collect();
                            let updates: Vec<_> = (1..=100).filter(|n| text.contains(&format!("TUPDATE{n:03}"))).collect();
                            (sid.clone(), serde_json::json!({"ui_pass":ctx.cumulative_pass_nr(), "window_occluded":ctx.input(|i| i.viewport().occluded), "offset":content.display_offset, "modes":content.terminal_mode.bits(), "focused":self.active_session.as_ref()==Some(sid), "samples":samples, "updates":updates, "rect":self.fixture_rect(ctx,&format!("terminal:{sid}"))}))
                        }).collect::<HashMap<_,_>>()),
                    );
                    insert(
                        &mut snapshot,
                        "editor_rect",
                        serde_json::to_value(self.fixture_rect(ctx, "editor-terminal"))?,
                    );
                    insert(
                        &mut snapshot,
                        "sidebar_projects",
                        serde_json::json!(
                            self.visible_projects()
                                .iter()
                                .map(|p| &p.id)
                                .collect::<Vec<_>>()
                        ),
                    );
                    insert(
                        &mut snapshot,
                        "project_sort",
                        serde_json::to_value(self.preferences.project_sort)?,
                    );
                    insert(
                        &mut snapshot,
                        "player",
                        serde_json::json!({
                            "chrome":self.fixture_rect(ctx,"player-chrome"),
                            "project":self.player.project,
                            "engine":self.player.fixture_diagnostics(),
                        }),
                    );
                    insert(
                        &mut snapshot,
                        "markdown_header",
                        serde_json::json!({
                            "title":self.fixture_rect(ctx,"markdown-title"),
                            "edit":self.fixture_rect(ctx,"markdown-mode:Edit"),
                            "preview":self.fixture_rect(ctx,"markdown-mode:Preview"),
                            "split":self.fixture_rect(ctx,"markdown-mode:Split"),
                            "refresh":self.fixture_rect(ctx,"markdown-refresh")
                        }),
                    );
                }
                return Ok(snapshot);
            }
            Ui::Focus { session } => {
                anyhow::ensure!(
                    self.state.sessions.iter().any(|s| s.id == session),
                    "Unknown session"
                );
                self.go_session(&session);
            }
            Ui::ShowSession {
                session,
                anchor,
                split,
            } => {
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
                    return Ok(serde_json::json!({"accepted":true,"existing":true}));
                }
                if let Some(anchor) = anchor {
                    anyhow::ensure!(
                        self.state
                            .sessions
                            .iter()
                            .any(|s| s.id == anchor && s.project_id == record.project_id),
                        "Anchor belongs to a different project"
                    );
                    let dock = self
                        .layouts
                        .get_mut(&record.project_id)
                        .context("Missing project layout")?;
                    let anchor = Tab::Terminal(anchor);
                    anyhow::ensure!(
                        dock.activate_containing(&anchor),
                        "Anchor is not in a visible workspace"
                    );
                    let path = dock.find_tab(&anchor).context("Missing anchor pane")?;
                    dock.set_focused_node_and_surface(path.node_path());
                    self.insert(&record.project_id, tab, split.as_deref());
                } else {
                    self.layouts
                        .entry(record.project_id.clone())
                        .or_insert_with(Workspace::empty)
                        .add(id(), tab);
                }
                self.select_project(record.project_id);
                self.active_session = Some(session);
            }
            Ui::OpenFile {
                project,
                path,
                as_text,
            } => {
                anyhow::ensure!(
                    self.state.projects.iter().any(|p| p.id == project),
                    "Unknown project"
                );
                anyhow::ensure!(
                    !self.layout_readonly.contains(&project),
                    "Project has an unsupported layout version"
                );
                self.select_project(project);
                self.open_file_mode(path, None, None, false, as_text);
            }
            Ui::OpenBrowser { url } => {
                let _ = self.jobs.send(Job::Browser(metadata::http_url(&url)?));
            }
            Ui::Window { action } => ctx.send_viewport_cmd(match action.as_str() {
                "minimize" => egui::ViewportCommand::Minimized(true),
                "maximize" => egui::ViewportCommand::Maximized(true),
                "restore" => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    egui::ViewportCommand::Maximized(false)
                }
                "close" => egui::ViewportCommand::Close,
                _ => egui::ViewportCommand::Focus,
            }),
        }
        self.save_layouts();
        ctx.request_repaint();
        Ok(serde_json::json!({"accepted":true}))
    }
    pub(crate) fn send(&self, request: Request) {
        let _ = self.jobs.send(Job::rpc(request, After::None));
    }
}
