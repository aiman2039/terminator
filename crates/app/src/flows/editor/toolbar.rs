//! The flow editor's toolbar: the flow name as a title (a click renames
//! it), the save state right after it, the tools, and Save and Run at the
//! right end. When the pane is narrow, the tools go into the More menu, so
//! nothing is cut off.
use super::FlowEditor;
use super::style::Colors;
use crate::appearance;
use crate::flows::status::chip;

/// One click in the editor's toolbar or in a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Click {
    Save,
    Run,
    AddStep,
    AddGate,
    Fit,
    Tidy,
    Float,
    Rules,
    Preview,
    CopyPlan,
    CopyFile,
    ExportPlan,
    Discard,
}

/// The shortcut labels that the toolbar shows.
pub(super) struct Keys {
    pub(super) save: String,
    pub(super) fit: String,
}

/// Widest the title grows.
const TITLE_WIDTH: f32 = 320.0;
/// The width of the tools before they first painted.
const TOOLS_WIDTH: f32 = 520.0;
const RIGHT_WIDTH: f32 = 200.0;

/// A toolbar button with an icon and a text.
fn tool(ui: &mut egui::Ui, icon: &str, label: &str, tip: &str) -> bool {
    appearance::icon_label_button(appearance::IconLabelButtonInput {
        ui,
        icon,
        label,
        height: appearance::TOOLBAR_BUTTON,
    })
    .on_hover_text(tip)
    .clicked()
}

/// The save state chip. A moved card alone is "Layout not saved", a
/// lighter state than a content change.
pub(super) fn save_state(editor: &FlowEditor) -> &'static str {
    if editor.saving.is_some() {
        "Saving…"
    } else if !editor.on_disk {
        "New flow, not saved"
    } else if editor.changed() {
        "Changes not saved"
    } else if editor.dirty() {
        "Layout not saved"
    } else {
        "Saved"
    }
}

/// The flow name as a title. A click shows a field; Enter, Esc or a click
/// elsewhere ends it.
fn title(ui: &mut egui::Ui, editor: &mut FlowEditor) {
    let width = TITLE_WIDTH.min(ui.available_width() * 0.35);
    if editor.editing_title {
        let field = ui.add(
            appearance::singleline(&mut editor.title)
                .desired_width(width)
                .hint_text("Flow name"),
        );
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        if field.lost_focus() {
            editor.editing_title = false;
        }
        return;
    }
    let text = if editor.title.trim().is_empty() {
        egui::RichText::new("No name").weak()
    } else {
        egui::RichText::new(editor.title.trim()).strong()
    };
    ui.scope(|ui| {
        ui.set_max_width(width);
        let label = egui::Label::new(text.size(appearance::FONT_HEADING))
            .truncate()
            .sense(egui::Sense::click());
        let response = ui.add(label).on_hover_text(format!(
            "{}. Its file is {}.toml. Click to rename the flow.",
            editor.title.trim(),
            editor.file
        ));
        if response.clicked() {
            editor.editing_title = true;
        }
    });
}

/// The toolbar of `editor`. `running`: a run of this flow is running.
pub(super) fn toolbar(
    ui: &mut egui::Ui,
    editor: &mut FlowEditor,
    (colors, keys, running): (Colors, &Keys, bool),
) -> Option<Click> {
    let mut click = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.set_min_height(appearance::TOOLBAR_BUTTON + 4.0);
        ui.add_space(8.0);
        title(ui, editor);
        let state = save_state(editor);
        let tint = if matches!(state, "Saved" | "Layout not saved" | "Saving…") {
            colors.weak
        } else {
            colors.gate
        };
        chip(ui, state, tint);
        let id = egui::Id::unique(("flow-toolbar-widths", editor.file.as_str()));
        let (tools, right) = ui
            .ctx()
            .data(|data| data.get_temp::<(f32, f32)>(id))
            .unwrap_or((TOOLS_WIDTH, RIGHT_WIDTH));
        let narrow = ui.available_width() < tools + right + 24.0;
        let mut measured = (tools, right);
        if !narrow {
            ui.separator();
            let shown = ui.scope(|ui| tools_row(ui, &mut click)).response;
            measured.0 = shown.rect.width();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            right_group(ui, editor, (keys, running, narrow), &mut click);
            // The narrow group has fewer tools: keep the full width.
            if !narrow {
                measured.1 = ui.min_rect().width();
            }
        });
        ui.ctx().data_mut(|data| data.insert_temp(id, measured));
    });
    ui.add_space(4.0);
    click
}

/// Step, Gate, Rules, Preview and Export.
fn tools_row(ui: &mut egui::Ui, click: &mut Option<Click>) {
    let mut set = |on: bool, value: Click| {
        if on {
            *click = Some(value);
        }
    };
    let tip = "Add a step: a command in a new terminal";
    set(tool(ui, "Terminal", "Step", tip), Click::AddStep);
    let tip = "Add a gate: the run waits for your approval";
    set(tool(ui, "ShieldQuestion", "Gate", tip), Click::AddGate);
    ui.separator();
    let tip = "Global rules of this flow, in the inspector";
    set(tool(ui, "Settings2", "Rules", tip), Click::Rules);
    set(
        tool(ui, "Eye", "Preview", "The plan as text"),
        Click::Preview,
    );
    appearance::text_menu_button(ui, "Export", |ui| export_items(ui, click));
}

fn export_items(ui: &mut egui::Ui, click: &mut Option<Click>) {
    let items = [
        ("Copy plan as Markdown", "Copy", Click::CopyPlan),
        ("Copy flow file text", "Copy", Click::CopyFile),
        ("Save plan as Markdown file", "Download", Click::ExportPlan),
    ];
    for (label, icon, value) in items {
        if appearance::menu_action(ui, true, label, icon).clicked() {
            *click = Some(value);
        }
    }
}

/// Right to left: Run, Save, More, Float, Tidy and Fit.
fn right_group(
    ui: &mut egui::Ui,
    editor: &FlowEditor,
    (keys, running, narrow): (&Keys, bool, bool),
    click: &mut Option<Click>,
) {
    let (dirty, saving) = (editor.dirty(), editor.saving.is_some());
    // A moved card alone is not a content change: Run stays on.
    let can_run = !editor.changed() && !running && !saving;
    ui.add_enabled_ui(can_run, |ui| {
        let why = if running {
            "A run of this flow is running."
        } else {
            "Save the flow, then run it."
        };
        let run = run_button(ui)
            .on_hover_text("Run the saved flow: one new terminal per step. The cards show the run.")
            .on_disabled_hover_text(why);
        if run.clicked() {
            *click = Some(Click::Run);
        }
    });
    ui.add_enabled_ui(dirty && !saving, |ui| {
        let tip = format!("Check the flow and write its file ({})", keys.save);
        if tool(ui, "Save", "Save", &tip) {
            *click = Some(Click::Save);
        }
    });
    ui.separator();
    appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
        if narrow {
            more_tools(ui, keys, click);
            ui.separator();
        }
        let discard = appearance::menu_danger_action(ui, dirty, "Discard changes", "Trash");
        if discard.clicked() {
            *click = Some(Click::Discard);
        }
    })
    .response
    .on_hover_text("More");
    if narrow {
        return;
    }
    let views = [
        ("ExternalLink", "Float window".to_owned(), Click::Float),
        ("LayoutDashboard", "Tidy layout".to_owned(), Click::Tidy),
        (
            "Maximize",
            format!("Fit to view ({})", keys.fit),
            Click::Fit,
        ),
    ];
    for (icon, tip, value) in views {
        if appearance::sidebar_action(ui, icon, &tip).clicked() {
            *click = Some(value);
        }
    }
}

/// Run, the editor's main action: a Play glyph and "Run" on the primary
/// fill. Disabled, it keeps a faint primary fill and a weak label, so it
/// still reads as the main action that does not apply now, not as one more
/// tool beside Save.
fn run_button(ui: &mut egui::Ui) -> egui::Response {
    let (fill, label) = appearance::filled_button_colors(appearance::primary_fill());
    let (fill, label) = if ui.is_enabled() {
        (fill, label)
    } else {
        (fill.gamma_multiply(0.22), ui.visuals().weak_text_color())
    };
    let icon = egui::Image::new(crate::icons::source("Play"))
        .tint(label)
        .fit_to_exact_size(egui::Vec2::splat(appearance::TOOLBAR_ICON));
    let response = ui.add(
        egui::Button::image_and_text(icon, egui::RichText::new("Run").strong().color(label))
            .fill(fill)
            .stroke(egui::Stroke::NONE)
            .min_size(egui::vec2(0.0, appearance::TOOLBAR_BUTTON)),
    );
    appearance::paint_focus(ui, &response);
    response
}

/// The tools in the More menu of a narrow toolbar.
fn more_tools(ui: &mut egui::Ui, keys: &Keys, click: &mut Option<Click>) {
    let items = [
        ("Add step", "Terminal", "", Click::AddStep),
        ("Add gate", "ShieldQuestion", "", Click::AddGate),
        ("Rules", "Settings2", "", Click::Rules),
        ("Preview", "Eye", "", Click::Preview),
        ("Fit to view", "Maximize", keys.fit.as_str(), Click::Fit),
        ("Tidy layout", "LayoutDashboard", "", Click::Tidy),
        ("Float window", "ExternalLink", "", Click::Float),
    ];
    for (label, icon, shortcut, value) in items {
        let item = appearance::menu_item(appearance::MenuItemInput {
            ui,
            label,
            icon,
            shortcut,
        });
        if item.clicked() {
            *click = Some(value);
            ui.close();
        }
    }
    ui.separator();
    export_items(ui, click);
}
