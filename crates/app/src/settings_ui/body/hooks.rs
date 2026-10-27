use crate::*;

impl App {
    pub(super) fn hook_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Agent Hooks");
        ui.weak("Agents are launched manually. Install hooks so Terminator can show waiting and done states.");
        egui::Grid::new("hook-settings")
            .num_columns(5)
            .show(ui, |ui| {
                for kind in terminator_integrations::AGENTS {
                    self.hook_row(ui, kind);
                }
            });
        ui.weak("See docs/INTEGRATIONS.md for event limitations.");
        ui.weak("Cursor reports completion and errors. Its permission and question dialogs have no notification hook. Select Completed under OS notifications to show completion alerts.");
        ui.add_space(8.0);
        self.hello_test_buttons(ui);
    }

    fn hook_row(&mut self, ui: &mut egui::Ui, kind: &str) {
        let installed = self.hook_status.get(kind).copied();
        let binary = settings_controls::agent_binary(kind);
        ui.label(kind);
        match binary {
            Some(path) => {
                settings_controls::status_badge(ui, &format!("Detected · {}", path.display()), true)
            }
            None => settings_controls::status_badge(ui, "Not on PATH", false),
        }
        ui.weak(match installed {
            Some(true) => "Configured",
            Some(false) => "Not configured",
            None => "Checking…",
        })
        .on_hover_text(
            terminator_integrations::settings_path(
                Path::new(&std::env::var("HOME").unwrap_or_default()),
                kind,
            )
            .display()
            .to_string(),
        );
        if ui
            .button(if installed == Some(true) {
                "Repair"
            } else {
                "Install"
            })
            .clicked()
        {
            let _ = self.jobs.send(Job::Install(kind.to_string(), false));
            let _ = self.jobs.send(Job::HookStatus);
        }
        if ui
            .add_enabled(installed == Some(true), egui::Button::new("Remove"))
            .clicked()
        {
            let _ = self.jobs.send(Job::Install(kind.to_string(), true));
            let _ = self.jobs.send(Job::HookStatus);
        }
        ui.end_row();
    }

    fn hello_test_buttons(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("hello test", "hello test verify notification") {
            return;
        }
        ui.weak("Send a hello from every agent to a live terminal. Uses saved settings; Apply notification changes first.");
        ui.horizontal(|ui| {
            let hello = ui.button("Send hello from all agents");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "hook-send-hello", hello.rect);
            if hello.clicked() {
                self.send_hello_test();
            }
            let copy = ui.button("Copy hello command");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "hook-copy-hello", copy.rect);
            if copy.clicked() {
                let helper = std::env::current_exe()
                    .map(|exe| exe.with_file_name("terminator-hook").display().to_string())
                    .unwrap_or_else(|_| "terminator-hook".into());
                ui.ctx().copy_text(notify_test::hello_command(&helper));
                self.info =
                    Some("Hello command copied. Paste it inside a Terminator terminal.".into());
            }
        });
    }

    fn send_hello_test(&mut self) {
        let live = |id: &str| {
            self.state
                .sessions
                .iter()
                .any(|s| s.id == id && s.lifecycle.live())
        };
        let sid = self
            .active_session
            .clone()
            .filter(|id| live(id))
            .or_else(|| {
                self.state
                    .sessions
                    .iter()
                    .find(|s| s.lifecycle.live())
                    .map(|s| s.id.clone())
            });
        let Some(sid) = sid else {
            self.error = Some("Open a terminal first, then send the hello test.".into());
            return;
        };
        let label = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .map(|s| s.label.clone())
            .unwrap_or_default();
        for event in notify_test::hello_events(&sid) {
            self.send(Request::Hook(event));
        }
        let mut note = format!(
            "Sent hello from {} agents to '{label}'. Check the Agents inbox.",
            terminator_integrations::AGENTS.len()
        );
        if !self
            .state
            .settings
            .events
            .contains(&AgentState::WaitingInput)
        {
            note.push_str(
                " Waiting input is off in saved Notifications → Events, so enable it and Apply.",
            );
        } else if !(self.state.settings.ntfy_enabled
            && !self.state.settings.ntfy_channel.is_empty())
        {
            note.push_str(" ntfy is off in saved settings, so phones stay silent.");
        }
        self.info = Some(note);
    }
}
