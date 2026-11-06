//! Per-project init command (`project-init-v1`): typed with Enter into each
//! new local shell terminal of the project. Editors and SSH terminals skip it.
use crate::*;

/// The open editor: project ID and its draft text.
#[derive(Clone, Default)]
struct Draft {
    project: String,
    text: String,
}

fn draft_id() -> egui::Id {
    egui::Id::unique("project-init-command-draft")
}

impl App {
    pub(crate) fn has_project_init(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|c| c == PROJECT_INIT_CAPABILITY)
    }

    pub(super) fn init_command_item(&mut self, ui: &mut egui::Ui, p: &Project) {
        if !self.has_project_init()
            || !appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Init command…",
                icon: "Terminal",
                shortcut: "",
            })
            .clicked()
        {
            return;
        }
        let draft = Draft {
            project: p.id.clone(),
            text: self
                .state
                .init_commands
                .get(&p.id)
                .cloned()
                .unwrap_or_default(),
        };
        ui.ctx()
            .data_mut(|data| data.insert_temp(draft_id(), draft));
        ui.close();
    }

    pub(crate) fn init_command_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = ctx.data(|data| data.get_temp::<Draft>(draft_id())) else {
            return;
        };
        let name = self
            .state
            .projects
            .iter()
            .find(|p| p.id == draft.project)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let mut open = true;
        let mut done = None;
        self.popups
            .window(ctx, "Init command")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Runs in each new shell terminal of {name}. Blank runs nothing."
                ));
                ui.add(
                    appearance::singleline(&mut draft.text)
                        .hint_text("source .venv/bin/activate")
                        .desired_width(360.0),
                );
                appearance::footer_row(ui, |ui| {
                    if ui.button("Save").clicked() {
                        done = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(false);
                    }
                });
            });
        if done == Some(true) {
            let command = Some(draft.text.trim().to_owned()).filter(|text| !text.is_empty());
            self.send(Request::SetInitCommand {
                project: draft.project.clone(),
                command,
            });
        }
        ctx.data_mut(|data| {
            if open && done.is_none() {
                data.insert_temp(draft_id(), draft);
            } else {
                data.remove::<Draft>(draft_id());
            }
        });
    }
}
