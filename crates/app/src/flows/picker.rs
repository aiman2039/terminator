//! The Flows window: the project's flow files with Run and Edit, and New
//! flow. New flow and Edit open the native flow editor. Making or editing
//! a flow needs no trust answer; Run does.
use super::LoadedFlows;
use crate::*;

/// One click in the Flows window.
enum Pick {
    Run(usize),
    Edit(usize),
    New,
    Trust(bool),
}

impl App {
    /// The Flows window.
    pub(crate) fn flow_dialog(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "test-support")]
        self.fixture_flow_editor();
        if !self.flows.open {
            return;
        }
        if let Some(receiver) = &self.flows.loading
            && let Ok(loaded) = receiver.try_recv()
        {
            self.flows.loading = None;
            self.flows.loaded = Some(loaded);
        }
        let mut open = true;
        let mut pick = None;
        let mut name = std::mem::take(&mut self.flows.new_name);
        self.popups
            .window(ctx, "Flows")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                let Some(loaded) = &self.flows.loaded else {
                    ui.weak("Reading flow files…");
                    return;
                };
                let answer = self.lsp.trust_answer(&loaded.root);
                pick = flow_trust_bar(ui, loaded, answer)
                    .map(Pick::Trust)
                    .or(flow_rows(ui, loaded, answer))
                    .or(new_flow_row(ui, &mut name));
            });
        self.flows.new_name = name;
        if self.flows.loading.is_some() {
            ctx.request_repaint();
        }
        let loaded = self.flows.loaded.clone().unwrap_or_default();
        match pick {
            Some(Pick::Trust(trusted)) => self.answer_task_trust(&loaded.root, trusted),
            Some(Pick::Run(index)) => {
                if let Some(file) = loaded.files.get(index) {
                    self.run_flow(&loaded, &file.file);
                    open = false;
                }
            }
            Some(Pick::Edit(index)) => {
                if let Some(file) = loaded.files.get(index) {
                    self.open_flow_tab(&loaded.project, &loaded.root, &file.file);
                    open = false;
                }
            }
            Some(Pick::New) => {
                let name = std::mem::take(&mut self.flows.new_name);
                // A refused name stays in the field.
                if self.open_flow_tab(&loaded.project, &loaded.root, &name) {
                    open = false;
                } else {
                    self.flows.new_name = name;
                }
            }
            None => {}
        }
        self.flows.open = open;
    }

    /// Test builds only: `TERMINATOR_FIXTURE_FLOW_EDITOR=NAME` opens the
    /// editor with flow NAME (or a new flow) of the selected project one
    /// time, so a fixture can look at the page without a click.
    #[cfg(feature = "test-support")]
    fn fixture_flow_editor(&mut self) {
        static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let Ok(name) = std::env::var("TERMINATOR_FIXTURE_FLOW_EDITOR") else {
            return;
        };
        let Some(project) = self.selected_project() else {
            return;
        };
        let (id, root) = (project.id.clone(), project.path.clone());
        if DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        self.open_flow_tab(&id, &root, &name);
    }
}

/// Trust question for a project's flow files. Returns the clicked answer.
fn flow_trust_bar(ui: &mut egui::Ui, loaded: &LoadedFlows, answer: Option<bool>) -> Option<bool> {
    if loaded.files.is_empty() || answer == Some(true) {
        return None;
    }
    let text = if answer == Some(false) {
        "This project is not trusted, so its flows stay off."
    } else {
        "Trust this project? A flow starts its commands and agents in your shell, with Enter."
    };
    settings_controls::description_line(ui, text);
    ui.horizontal(|ui| {
        if ui.button("Trust project").clicked() {
            Some(true)
        } else if answer.is_none() && ui.button("Don't trust").clicked() {
            Some(false)
        } else {
            None
        }
    })
    .inner
}

/// One row per flow file with Run and Edit. Returns the clicked button.
fn flow_rows(ui: &mut egui::Ui, loaded: &LoadedFlows, answer: Option<bool>) -> Option<Pick> {
    if loaded.files.is_empty() {
        settings_controls::description_line(
            ui,
            "No flows in this project. Make one below: it opens in the flow editor, where \
             you add steps and gates and connect them in the order that they run.",
        );
        return None;
    }
    let mut clicked = None;
    // One row per flow: the name and the file, then the actions in one
    // column for all rows.
    egui::Grid::new("run-flow-rows")
        .num_columns(2)
        .spacing([24.0, 10.0])
        .show(ui, |ui| {
            for (index, file) in loaded.files.iter().enumerate() {
                let allowed = file.flow.is_ok() && answer == Some(true);
                let why = if file.flow.is_err() {
                    "Correct the flow first"
                } else {
                    "Trust the project first"
                };
                ui.vertical(|ui| {
                    // A long error wraps; it does not make the window wide.
                    ui.set_max_width(300.0);
                    match &file.flow {
                        Ok((name, steps)) => {
                            ui.strong(name);
                            ui.weak(format!("{}.toml · {steps} steps", file.file));
                        }
                        Err(error) => {
                            ui.strong(&file.file);
                            let first = error.lines().next().unwrap_or_default();
                            ui.colored_label(ui.visuals().error_fg_color, first)
                                .on_hover_text(error);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(allowed, |ui| {
                        let tip = if allowed { "Run this flow" } else { why };
                        if tool(ui, ("Play", "Run"), tip) {
                            clicked = Some(Pick::Run(index));
                        }
                    });
                    if tool(ui, ("Pencil", "Edit"), "Open the flow in the flow editor") {
                        clicked = Some(Pick::Edit(index));
                    }
                });
                ui.end_row();
            }
        });
    clicked
}

/// A frameless button with an icon and a text.
fn tool(ui: &mut egui::Ui, (icon, label): (&str, &str), tip: &str) -> bool {
    appearance::icon_label_button(appearance::IconLabelButtonInput {
        ui,
        icon,
        label,
        height: appearance::TOOLBAR_BUTTON,
    })
    .on_hover_text(tip)
    .on_disabled_hover_text(tip)
    .clicked()
}

/// The name field and the New flow button.
fn new_flow_row(ui: &mut egui::Ui, name: &mut String) -> Option<Pick> {
    ui.separator();
    ui.add_space(4.0);
    ui.label(egui::RichText::new("New flow").small().weak());
    ui.horizontal(|ui| {
        let field = ui.add(
            appearance::singleline(name)
                .desired_width(260.0)
                .hint_text("Name, for example feature"),
        );
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let made = tool(
            ui,
            ("Plus", "Create"),
            "Open the flow editor with a new flow",
        );
        ((made || entered) && !name.trim().is_empty()).then_some(Pick::New)
    })
    .inner
}
