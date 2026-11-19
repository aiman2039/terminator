//! The Run flow window and the "Flows" section of the Flows tool window:
//! the project's flow files with Run and Edit, and New flow. New flow and
//! Edit open the native flow editor. Making or editing a flow needs no
//! trust answer; Run does, and only an explicit click runs.
use super::LoadedFlows;
use super::panel::{Act, ROW_HEIGHT, section_title};
use super::status::Tone;
use crate::*;

impl App {
    /// The tool window's flow files: Edit and Run on each, the trust
    /// question inline above them, and New flow. A broken file shows red
    /// with its error.
    pub(super) fn flow_files_section(&mut self, ui: &mut egui::Ui, act: &mut Option<Act>) {
        let Some(loaded) = self.tool_flows().cloned() else {
            section_title(ui, "Flows", 0);
            ui.weak("Reading flow files…");
            return;
        };
        section_title(ui, "Flows", loaded.files.len());
        let answer = self.lsp.trust_answer(&loaded.root);
        if let Some(trusted) = flow_trust_bar(ui, &loaded, answer) {
            *act = Some(Act::Trust(trusted));
        }
        for file in &loaded.files {
            self.flow_file_row(ui, (file, answer == Some(true)), act);
        }
        let new = appearance::row(appearance::RowInput {
            ui,
            label: "New flow…",
            icon: "Plus",
            selected: false,
            height: ROW_HEIGHT,
            trailing: "",
            tint: Tone::Dim.color(&self.theme),
        })
        .on_hover_text("Name a new flow, then make it in the flow editor");
        if new.clicked() {
            *act = Some(Act::NewFlow);
        }
    }

    /// One flow file: its name and step count, or the file name and its
    /// error in red; Edit, and Run once the project is `trusted`.
    fn flow_file_row(
        &self,
        ui: &mut egui::Ui,
        (file, trusted): (&super::FlowFile, bool),
        act: &mut Option<Act>,
    ) {
        let failed = Tone::Failed.color(&self.theme);
        let (label, trailing, icon, tint) = match &file.flow {
            Ok((name, steps)) => {
                let steps = appearance::count_noun(*steps, "step");
                let text = appearance::color(&self.theme.text);
                let dim = Tone::Dim.color(&self.theme);
                (name.clone(), steps, ("Workflow", dim), text)
            }
            // The error goes on its own line below: a trailing text that
            // does not fit would not show.
            Err(_) => (
                file.file.clone(),
                String::new(),
                ("CircleAlert", failed),
                failed,
            ),
        };
        let hover = match &file.flow {
            Ok(_) => format!("{}.toml", file.file),
            Err(error) => format!("{}.toml: {error}", file.file),
        };
        let why = if file.flow.is_err() {
            "Correct the flow first"
        } else {
            "Trust the project first"
        };
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(file.flow.is_ok() && trusted, |ui| {
                    let run = appearance::sidebar_action(ui, "Play", "Run this flow")
                        .on_disabled_hover_text(why);
                    if run.clicked() {
                        *act = Some(Act::Run(file.file.clone()));
                    }
                });
                let edit = "Open the flow in the flow editor";
                if appearance::sidebar_action(ui, "Pencil", edit).clicked() {
                    *act = Some(Act::Edit(file.file.clone()));
                }
                appearance::status_row(appearance::StatusRowInput {
                    row: appearance::RowInput {
                        ui,
                        label: &label,
                        icon: icon.0,
                        selected: false,
                        height: ROW_HEIGHT,
                        trailing: &trailing,
                        tint,
                    },
                    icon_tint: icon.1,
                    spin: false,
                })
                .on_hover_text(hover.clone());
            });
        });
        if let Err(error) = &file.flow {
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                let first = error.lines().next().unwrap_or_default();
                let text = egui::RichText::new(first)
                    .size(appearance::FONT_CAPTION)
                    .color(failed);
                ui.add(egui::Label::new(text).truncate())
                    .on_hover_text(hover);
            });
        }
    }
}

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
        self.flow_inputs_dialog(ctx);
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
        let pick = self.run_flow_window(ctx, &mut open);
        if self.flows.loading.is_some() {
            ctx.request_repaint();
        }
        let loaded = self.flows.loaded.clone().unwrap_or_default();
        match pick {
            Some(Pick::Trust(trusted)) => self.answer_task_trust(&loaded.root, trusted),
            Some(Pick::Run(index)) => {
                if let Some(file) = loaded.files.get(index) {
                    let origin = super::inputs_form::Origin::Window;
                    self.request_flow_run(&loaded, &file.file, (&file.inputs, origin));
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

    /// The Run flow window. It opens at the click, or centered on the
    /// focused pane after a key (`Popups::window`), and stays inside the
    /// app window: when its list does not fit below its top, it moves up,
    /// and what still does not fit scrolls in it.
    fn run_flow_window(&mut self, ctx: &egui::Context, open: &mut bool) -> Option<Pick> {
        let mut pick = None;
        let mut name = std::mem::take(&mut self.flows.new_name);
        let bounds = ctx.content_rect();
        let max_height = (bounds.height() - 96.0).max(160.0);
        let mut window = self
            .popups
            .window(ctx, "Run flow")
            .open(open)
            .collapsible(false)
            // Sized by its content, not egui's default window height.
            .auto_sized();
        if let Some(top) = self.flows.picker_top.take() {
            window = window.movable(false).current_pos(top);
        }
        let shown = window.show(ctx, |ui| {
            ui.set_width(WIDTH);
            let Some(loaded) = &self.flows.loaded else {
                ui.weak("Reading flow files…");
                return None;
            };
            let answer = self.lsp.trust_answer(&loaded.root);
            let scroll = egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    pick = flow_trust_bar(ui, loaded, answer)
                        .map(Pick::Trust)
                        .or(flow_rows(ui, loaded, answer))
                        .or(new_flow_row(ui, &mut name));
                });
            Some(scroll.content_size.y - scroll.inner_rect.height())
        });
        self.flows.new_name = name;
        // Hidden rows: move the window up by as much, inside the app window.
        if let Some(shown) = shown
            && let Some(Some(hidden)) = shown.inner
            && hidden > 1.0
        {
            let rect = shown.response.rect;
            let top = (rect.top() - hidden).max(bounds.top() + 8.0);
            if top < rect.top() - 0.5 {
                self.flows.picker_top = Some(egui::pos2(rect.left(), top));
                ctx.request_repaint();
            }
        }
        pick
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
        // `TERMINATOR_FIXTURE_FLOW_WINDOW` also opens the Flows window.
        if std::env::var_os("TERMINATOR_FIXTURE_FLOW_WINDOW").is_some() {
            self.open_flow_picker();
        }
        // `TERMINATOR_FIXTURE_FLOW_INPUTS` opens the Run inputs form of the
        // flow, as a Run click does. It runs nothing.
        if std::env::var_os("TERMINATOR_FIXTURE_FLOW_INPUTS").is_some() {
            let loaded = super::load_flows(id, root);
            if let Some(flow) = loaded.files.iter().find(|file| file.file == name) {
                let origin = super::inputs_form::Origin::Window;
                let form = super::inputs_form::InputsForm::new(
                    loaded.clone(),
                    &name,
                    &flow.inputs,
                    origin,
                );
                self.flows.inputs_form = Some(form);
            }
        }
    }
}

/// Trust question for a project's flow files. Returns the clicked answer.
pub(super) fn flow_trust_bar(
    ui: &mut egui::Ui,
    loaded: &LoadedFlows,
    answer: Option<bool>,
) -> Option<bool> {
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
        if appearance::primary_button(ui, "Trust project").clicked() {
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
    section(ui, "Flows of this project");
    for (index, file) in loaded.files.iter().enumerate() {
        let allowed = file.flow.is_ok() && answer == Some(true);
        let why = if file.flow.is_err() {
            "Correct the flow first"
        } else {
            "Trust the project first"
        };
        // One card per flow: the name and the file on the left, Edit and
        // the main action Run on the right.
        row_card(ui, |ui| {
            ui.vertical(|ui| {
                // A long error wraps; it does not push the buttons out.
                ui.set_max_width(WIDTH - 190.0);
                match &file.flow {
                    Ok((name, steps)) => {
                        ui.strong(name);
                        let about = format!("{}.toml · {steps} steps", file.file);
                        ui.label(egui::RichText::new(about).small().weak());
                    }
                    Err(error) => {
                        ui.strong(&file.file);
                        let first = error.lines().next().unwrap_or_default();
                        ui.colored_label(ui.visuals().error_fg_color, first)
                            .on_hover_text(error);
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(allowed, |ui| {
                    let tip = if allowed {
                        "Start this flow. Its tab shows each step of the run."
                    } else {
                        why
                    };
                    let run = appearance::primary_button(ui, "Run")
                        .on_hover_text(tip)
                        .on_disabled_hover_text(tip);
                    if run.clicked() {
                        clicked = Some(Pick::Run(index));
                    }
                });
                let edit = appearance::secondary_button(ui, "Edit")
                    .on_hover_text("Open the flow in the flow editor");
                if edit.clicked() {
                    clicked = Some(Pick::Edit(index));
                }
            });
        });
        ui.add_space(6.0);
    }
    clicked
}

/// Width of the window's contents: fixed, so a row's buttons can be at
/// its right end.
const WIDTH: f32 = 460.0;

/// The small heading of a part of the window.
fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).small().weak());
    ui.add_space(2.0);
}

/// One row as a card on the whole width.
fn row_card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(appearance::RADIUS_MEDIUM)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(WIDTH - 24.0);
            ui.horizontal(add);
        });
}

/// The name field and the Create button.
fn new_flow_row(ui: &mut egui::Ui, name: &mut String) -> Option<Pick> {
    ui.add_space(8.0);
    section(ui, "New flow");
    let mut pick = None;
    row_card(ui, |ui| {
        let field = ui.add(
            appearance::singleline(name)
                .desired_width(WIDTH - 130.0)
                .hint_text("Name, for example feature"),
        );
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let made = ui
                .button("Create")
                .on_hover_text("Open the flow editor with a new flow")
                .clicked();
            if (made || entered) && !name.trim().is_empty() {
                pick = Some(Pick::New);
            }
        });
    });
    pick
}
