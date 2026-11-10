//! Settings → Agents: agent presets and the catalog agents on PATH. Preset
//! edits go to the UI preferences at once; they are not part of the
//! Settings draft.
use crate::preferences::AgentPreset;
use crate::*;
use terminator_core::agents;

impl App {
    pub(super) fn agent_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Agents");
        ui.weak(
            "New terminal with an agent types its command. Press Enter in the terminal to start it. Preset changes save at once.",
        );
        ui.add_space(8.0);
        if self.field_visible("Presets", "preset program arguments") {
            self.agent_preset_grid(ui);
        }
        if self.field_visible("Installed agents", "path detected") {
            ui.add_space(12.0);
            ui.strong("Installed agents");
            if self.installed_agents.is_empty() {
                ui.weak("No catalog agent found on PATH.");
            }
            for agent in &self.installed_agents {
                settings_controls::status_badge(
                    ui,
                    &format!("{} · {}", agents::display_name(&agent.kind), agent.program),
                    true,
                );
            }
        }
    }

    fn agent_preset_grid(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        egui::Grid::new("agent-presets")
            .num_columns(5)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                for title in ["Name", "Icon", "Program", "Arguments", ""] {
                    ui.strong(title);
                }
                ui.end_row();
                for (index, preset) in self.preferences.agent_presets.iter_mut().enumerate() {
                    if preset_row(ui, index, preset) {
                        remove = Some(index);
                    }
                    ui.end_row();
                }
            });
        if let Some(index) = remove {
            self.preferences.agent_presets.remove(index);
        }
        if ui.button("Add preset").clicked() {
            let number = self.preferences.agent_presets.len().saturating_add(1);
            self.preferences.agent_presets.push(AgentPreset {
                name: format!("Preset {number}"),
                ..AgentPreset::default()
            });
        }
    }
}

/// One editable preset; returns true when Delete was clicked.
fn preset_row(ui: &mut egui::Ui, index: usize, preset: &mut AgentPreset) -> bool {
    ui.add(
        appearance::singleline(&mut preset.name)
            .hint_text("Name")
            .desired_width(120.0),
    );
    let shown = agents::catalog(&preset.kind).map_or("Generic", |info| info.name);
    egui::ComboBox::from_id_salt(("agent-preset-kind", index))
        .selected_text(shown)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut preset.kind, String::new(), "Generic");
            for info in agents::CATALOG {
                ui.selectable_value(&mut preset.kind, info.kind.to_owned(), info.name);
            }
        });
    ui.add(
        appearance::singleline(&mut preset.program)
            .hint_text("claude")
            .desired_width(120.0),
    );
    args_field(ui, index, &mut preset.args);
    appearance::sidebar_action(ui, "Trash", "Delete preset").clicked()
}

/// Arguments as shell words. The typed text stays while the field has
/// focus; each edit that `shlex` can split replaces the arguments.
fn args_field(ui: &mut egui::Ui, index: usize, args: &mut Vec<String>) {
    let id = egui::Id::unique(("agent-preset-args", index));
    let mut text = ui
        .data(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| shlex::try_join(args.iter().map(String::as_str)).unwrap_or_default());
    let response = ui.add(
        appearance::singleline(&mut text)
            .hint_text("--model opus")
            .desired_width(200.0),
    );
    let words = shlex::split(&text);
    if response.changed()
        && let Some(words) = words.clone()
    {
        *args = words;
    }
    if words.is_none() {
        response.on_hover_text("Close the open quote.");
    } else if !response.has_focus() {
        ui.data_mut(|data| data.remove::<String>(id));
        return;
    }
    ui.data_mut(|data| data.insert_temp(id, text));
}
