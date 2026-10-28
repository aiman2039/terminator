//! The `Snapshot` UI request: selection, layouts, window geometry, and,
//! with `test-support`, the fixture diagnostics scenario tests read.

use anyhow::Result;
#[cfg(feature = "test-support")]
use std::collections::HashMap;

use crate::*;

struct SnapshotHelpers;

impl SnapshotHelpers {
    /// Sets `key` when `value` is an object; other values are left alone.
    fn insert(value: &mut serde_json::Value, key: &str, next: serde_json::Value) {
        if let Some(object) = value.as_object_mut() {
            object.insert(key.to_owned(), next);
        }
    }
}

impl App {
    pub(super) fn ui_snapshot(&self, ctx: &egui::Context) -> Result<serde_json::Value> {
        let gui_ppp = ctx.pixels_per_point();
        let mut snapshot = serde_json::json!({"selected_project":self.selected,"active_session":self.active_session,"workspaces":self.layouts,
            "controls":{
                "header-drag":self.fixture_rect(ctx,"header-drag"),
                "project-add":self.fixture_rect(ctx,"project-add"),
                "resize-se":self.fixture_rect(ctx,"window-resize-3")
            },
            "window":ctx.input(|i|serde_json::json!({"inner":i.viewport().inner_rect.map(|r|[r.min.x,r.min.y,r.width(),r.height()]),"outer":i.viewport().outer_rect.map(|r|[r.min.x,r.min.y,r.width(),r.height()]),"maximized":i.viewport().maximized,"minimized":i.viewport().minimized,"gui_ppp":gui_ppp,"native_ppp":i.viewport().native_pixels_per_point}))});
        SnapshotHelpers::insert(&mut snapshot, "services", self.services.diagnostics());
        if let Some(services) = snapshot.get_mut("services") {
            SnapshotHelpers::insert(
                services,
                "ui_processing_peak_ms",
                serde_json::json!(self.ui_service_peak_ms),
            );
        }
        #[cfg(feature = "test-support")]
        {
            self.fixture_snapshot_chrome(ctx, &mut snapshot);
            self.fixture_snapshot_views(ctx, &mut snapshot)?;
        }
        Ok(snapshot)
    }

    /// Header controls, updater, installation, and attention state.
    #[cfg(feature = "test-support")]
    fn fixture_snapshot_chrome(&self, ctx: &egui::Context, snapshot: &mut serde_json::Value) {
        if let Some(controls) = snapshot.get_mut("controls") {
            for key in ["left-agent-bar", "project-header-menu"] {
                SnapshotHelpers::insert(
                    controls,
                    key,
                    serde_json::json!(crate::diagnostics::current_rect(ctx, key).map(|r| [
                        r.min.x,
                        r.min.y,
                        r.width(),
                        r.height()
                    ])),
                );
            }
        }
        SnapshotHelpers::insert(
            snapshot,
            "updater_available",
            serde_json::json!(self.updater.available()),
        );
        SnapshotHelpers::insert(
            snapshot,
            "update_menu",
            serde_json::json!(self.updater.menu_installed()),
        );
        SnapshotHelpers::insert(snapshot, "installation", self.fixture_installation());
        SnapshotHelpers::insert(
            snapshot,
            "attention",
            serde_json::json!(ctx.data(|data| {
                data.get_temp::<(usize, bool)>(egui::Id::unique("attention-state"))
            })),
        );
        SnapshotHelpers::insert(
            snapshot,
            "left_agents",
            serde_json::json!(self.preferences.left_agents),
        );
        SnapshotHelpers::insert(
            snapshot,
            "agent_bar_badge",
            serde_json::json!(
                ctx.data(|data| { data.get_temp::<String>(egui::Id::unique("agent-bar-badge")) })
            ),
        );
    }

    #[cfg(feature = "test-support")]
    fn fixture_installation(&self) -> serde_json::Value {
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
        })
    }

    /// Markdown, terminals, editor, sidebar, and player state.
    #[cfg(feature = "test-support")]
    fn fixture_snapshot_views(
        &self,
        ctx: &egui::Context,
        snapshot: &mut serde_json::Value,
    ) -> Result<()> {
        SnapshotHelpers::insert(snapshot, "markdown", self.markdown.diagnostics());
        SnapshotHelpers::insert(
            snapshot,
            "markdown_modes",
            serde_json::to_value(&self.preferences.markdown_modes)?,
        );
        SnapshotHelpers::insert(
            snapshot,
            "visible_terminals",
            serde_json::to_value(&self.visible_sessions)?,
        );
        SnapshotHelpers::insert(
            snapshot,
            "fixture_actions_completed",
            serde_json::json!(self.diagnostics.actions_completed()),
        );
        SnapshotHelpers::insert(
            snapshot,
            "terminal_scroll",
            self.fixture_terminal_scroll(ctx),
        );
        SnapshotHelpers::insert(
            snapshot,
            "editor_rect",
            serde_json::to_value(self.fixture_rect(ctx, "editor-terminal"))?,
        );
        SnapshotHelpers::insert(
            snapshot,
            "sidebar_projects",
            serde_json::json!(
                self.visible_projects()
                    .iter()
                    .map(|p| &p.id)
                    .collect::<Vec<_>>()
            ),
        );
        SnapshotHelpers::insert(
            snapshot,
            "project_sort",
            serde_json::to_value(self.preferences.project_sort)?,
        );
        SnapshotHelpers::insert(
            snapshot,
            "player",
            serde_json::json!({
                "chrome":self.fixture_rect(ctx,"player-chrome"),
                "project":self.player.project,
                "engine":self.player.fixture_diagnostics(),
            }),
        );
        SnapshotHelpers::insert(
            snapshot,
            "markdown_header",
            serde_json::json!({
                "title":self.fixture_rect(ctx,"markdown-title"),
                "edit":self.fixture_rect(ctx,"markdown-mode:Edit"),
                "preview":self.fixture_rect(ctx,"markdown-mode:Preview"),
                "split":self.fixture_rect(ctx,"markdown-mode:Split"),
                "refresh":self.fixture_rect(ctx,"markdown-refresh")
            }),
        );
        Ok(())
    }

    /// Per-terminal scroll offset, modes, and which numbered sample lines
    /// are on screen.
    #[cfg(feature = "test-support")]
    fn fixture_terminal_scroll(&self, ctx: &egui::Context) -> serde_json::Value {
        serde_json::json!(self.backends.iter().map(|(sid, backend)| {
            let content = backend.last_content();
            let text: String = content.grid.display_iter().map(|cell| cell.c).collect();
            let samples: Vec<_> = (1..=160).filter(|n| text.contains(&format!("TSAMPLE{n:03}"))).collect();
            let updates: Vec<_> = (1..=100).filter(|n| text.contains(&format!("TUPDATE{n:03}"))).collect();
            (sid.clone(), serde_json::json!({"ui_pass":ctx.cumulative_pass_nr(), "window_occluded":ctx.input(|i| i.viewport().occluded), "offset":content.display_offset, "modes":content.terminal_mode.bits(), "focused":self.active_session.as_ref()==Some(sid), "samples":samples, "updates":updates, "rect":self.fixture_rect(ctx,&format!("terminal:{sid}"))}))
        }).collect::<HashMap<_,_>>())
    }
}
