//! Restore workspace after a reboot: one explicit action per project that
//! opens each interrupted shell pane again, in the same layout position,
//! as a fresh shell in the same folder with its saved output above the
//! prompt (`restore-shell-v1`). Nothing runs without that click, nothing
//! is typed into the new shell, and no agent is launched: the interrupted
//! session stays in History with its Resume conversation command.
use crate::{After, App, Job, Tab};
use terminator_core::{Lifecycle, Request, Session, SessionKind, agent_workflow};

/// One interrupted shell waiting for its saved output.
pub(crate) struct PendingRestore {
    pub old: String,
}

fn restore_key(session: &str) -> String {
    format!("restore:{session}")
}

impl App {
    /// The daemon can open a shell with saved output (`restore-shell-v1`).
    pub(crate) fn restore_available(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|c| c == agent_workflow::RESTORE_CAPABILITY)
    }

    /// Interrupted shell sessions of `project` that still have a pane.
    pub(crate) fn restorable_sessions(&self, project: &str) -> Vec<Session> {
        self.state
            .sessions
            .iter()
            .filter(|s| {
                s.project_id == project
                    && s.lifecycle == Lifecycle::Interrupted
                    && s.kind == SessionKind::Shell
                    && !s.review
                    && self.session_has_pane(&s.id)
            })
            .cloned()
            .collect()
    }

    fn session_has_pane(&self, sid: &str) -> bool {
        let target = Tab::Terminal(sid.to_owned());
        let in_layouts = self.layouts.values().any(|workspace| {
            workspace.tabs.iter().any(|group| {
                group.primary.as_ref() == Some(&target) || group.layout.find_tab(&target).is_some()
            })
        });
        in_layouts
            || self
                .floating
                .iter()
                .filter_map(|window| window.dock.as_ref())
                .any(|dock| dock.find_tab(&target).is_some())
            || self
                .preferences
                .ide_strip_docks
                .0
                .values()
                .any(|dock| dock.find_tab(&target).is_some())
    }

    /// The Restore workspace click: read each interrupted shell's saved
    /// output, then open its replacement. Returns how many panes restore.
    pub(crate) fn restore_workspace(&mut self, project: &str) -> usize {
        if !self.restore_available() {
            self.info = Some(
                "The running session service cannot restore panes. Use Open a fresh shell here."
                    .into(),
            );
            return 0;
        }
        let sessions = self.restorable_sessions(project);
        for session in &sessions {
            if self.triage.restores.iter().any(|r| r.old == session.id) {
                continue;
            }
            let key = restore_key(&session.id);
            self.texts.remove(&key);
            self.loading.insert(key.clone());
            let _ = self.jobs.send(Job::rpc(
                Request::History {
                    session: session.id.clone(),
                },
                After::Text(key),
            ));
            self.triage.restores.push(PendingRestore {
                old: session.id.clone(),
            });
        }
        self.info = Some(match sessions.len() {
            0 => "No interrupted panes to restore.".into(),
            1 => "Restoring 1 pane…".into(),
            n => format!("Restoring {n} panes…"),
        });
        sessions.len()
    }

    /// Sends the shell request of each restore whose saved output arrived.
    pub(crate) fn continue_restores(&mut self) {
        let ready: Vec<String> = self
            .triage
            .restores
            .iter()
            .filter(|r| self.texts.contains_key(&restore_key(&r.old)))
            .map(|r| r.old.clone())
            .collect();
        for old in ready {
            self.triage.restores.retain(|r| r.old != old);
            let text = self.texts.remove(&restore_key(&old)).unwrap_or_default();
            let Some(session) = self.state.sessions.iter().find(|s| s.id == old) else {
                continue;
            };
            let request = Request::CreateRestored {
                project: session.project_id.clone(),
                cwd: session.cwd.clone(),
                label: session.label.clone(),
                seed: agent_workflow::restore_seed(&text),
            };
            let _ = self.jobs.send(Job::rpc(request, After::Restore(old)));
        }
    }

    /// "Restore workspace (N)" in a project menu, while it has interrupted
    /// panes and the daemon can restore them.
    pub(crate) fn restore_workspace_item(&mut self, ui: &mut eframe::egui::Ui, project: &str) {
        if !self.restore_available() {
            return;
        }
        let count = self.restorable_sessions(project).len();
        if count == 0 {
            return;
        }
        let label = format!("Restore workspace ({count})");
        let response = crate::appearance::menu_item(crate::appearance::MenuItemInput {
            ui,
            label: &label,
            icon: "History",
            shortcut: "",
        })
        .on_hover_text(
            "Open each interrupted shell again in its pane, with its saved output above a fresh prompt. Agents are not started.",
        );
        if response.clicked() {
            self.restore_workspace(project);
            ui.close();
        }
    }

    /// The new shell takes the interrupted pane's place in every layout.
    pub(crate) fn handle_restored(&mut self, session: Session, old: &str) {
        let new = session.id.as_str();
        for workspace in self.layouts.values_mut() {
            for group in &mut workspace.tabs {
                retarget_option(&mut group.primary, old, new);
                retarget_dock(&mut group.layout, old, new);
            }
        }
        for window in &mut self.floating {
            if let Some(dock) = window.dock.as_mut() {
                retarget_dock(dock, old, new);
            }
        }
        for dock in self.preferences.ide_strip_docks.0.values_mut() {
            retarget_dock(dock, old, new);
        }
        self.detach_backend(old);
        self.pending_layout_save = true;
    }
}

fn retarget_option(tab: &mut Option<Tab>, old: &str, new: &str) {
    if matches!(tab, Some(Tab::Terminal(sid)) if sid == old) {
        *tab = Some(Tab::Terminal(new.to_owned()));
    }
}

fn retarget_dock(dock: &mut egui_dock::DockState<Tab>, old: &str, new: &str) {
    for (_, tab) in dock.iter_all_tabs_mut() {
        if matches!(tab, Tab::Terminal(sid) if sid == old) {
            *tab = Tab::Terminal(new.to_owned());
        }
    }
}
