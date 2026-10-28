//! Info tool rendering: collapsible Process and Resources sections.
use super::{
    BRANCH, CWD, DASH, LOAD, Model, PRESSURE, SESSION, SESSION_CPU, SESSION_MEMORY, STARTED,
    SYSTEM_CPU, SYSTEM_MEMORY, SYSTEM_TOGGLE, SystemRows, Toggles,
};
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{appearance, icons};
use eframe::egui::{self, Color32, RichText};
use terminator_core::appearance::AppearanceConfig;
use terminator_sys::PressureLevel;

pub struct ShowInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub model: &'a Model,
    pub toggles: &'a mut Toggles,
    pub theme: &'a AppearanceConfig,
}

pub fn show(input: ShowInput<'_>) {
    let ShowInput {
        ui,
        model,
        toggles,
        theme,
    } = input;
    ui.spacing_mut().item_spacing.y = 3.0;
    appearance::sidebar_scroll("session-info").show(ui, |ui| {
        ui.set_width(ui.available_width());
        let view = InfoView {
            model,
            theme,
            text: appearance::color(&theme.text),
            secondary: appearance::color(&theme.secondary),
            live: appearance::color(&theme.status_running),
        };
        view.process(ui, toggles);
        ui.add_space(8.0);
        view.resources(ui, toggles);
    });
}

/// A value row's label and its diagnostics marker.
#[derive(Clone, Copy)]
struct RowId {
    label: &'static str,
    target: &'static str,
}

impl RowId {
    const SESSION: Self = Self::new("session", SESSION);
    const CWD: Self = Self::new("cwd", CWD);
    const STARTED: Self = Self::new("started", STARTED);
    const SESSION_CPU: Self = Self::new("cpu", SESSION_CPU);
    const SESSION_MEMORY: Self = Self::new("memory", SESSION_MEMORY);
    const SYSTEM_CPU: Self = Self::new("cpu", SYSTEM_CPU);
    const SYSTEM_MEMORY: Self = Self::new("memory", SYSTEM_MEMORY);
    const PRESSURE: Self = Self::new("pressure", PRESSURE);
    const LOAD: Self = Self::new("load", LOAD);

    const fn new(label: &'static str, target: &'static str) -> Self {
        Self { label, target }
    }
}

/// The model plus theme colors resolved once per frame.
struct InfoView<'a> {
    model: &'a Model,
    theme: &'a AppearanceConfig,
    text: Color32,
    secondary: Color32,
    live: Color32,
}

impl InfoView<'_> {
    fn process(&self, ui: &mut egui::Ui, toggles: &mut Toggles) {
        if !section_header(SectionHeaderInput {
            ui,
            title: "Process",
            open: &mut toggles.process_open,
            system: None,
            text: self.text,
            secondary: self.secondary,
        }) {
            return;
        }
        let model = self.model;
        self.value(ui, RowId::SESSION, &model.session);
        let cwd = self.value(ui, RowId::CWD, &model.cwd);
        if let Some(full) = &model.cwd_full {
            cwd.on_hover_text(full);
        }
        if model.unconfirmed {
            ui.label(
                RichText::new("Last known directory")
                    .small()
                    .color(self.secondary),
            );
        }
        let branch = model.branch.as_deref().unwrap_or(DASH);
        let branch_color = if model.branch.is_some() {
            self.live
        } else {
            self.text
        };
        value_row(ValueRowInput {
            ui,
            label: "branch",
            value: branch,
            value_color: branch_color,
            label_color: self.secondary,
            target: BRANCH,
        });
        self.value(ui, RowId::STARTED, &model.started);
    }

    fn resources(&self, ui: &mut egui::Ui, toggles: &mut Toggles) {
        let system_toggle = toggles.show_system;
        if !section_header(SectionHeaderInput {
            ui,
            title: "Resources",
            open: &mut toggles.resources_open,
            system: Some(&mut toggles.show_system),
            text: self.text,
            secondary: self.secondary,
        }) {
            return;
        }
        ui.add_space(4.0);
        ui.label(RichText::new("THIS SESSION").small().color(self.secondary));
        self.value(ui, RowId::SESSION_CPU, &self.model.session_cpu);
        self.value(ui, RowId::SESSION_MEMORY, &self.model.session_memory);
        if !system_toggle {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new("SYSTEM").small().color(self.secondary));
        if let Some(system) = &self.model.system {
            self.system_meters(ui, system);
        } else {
            self.value(ui, RowId::SYSTEM_CPU, DASH);
            self.value(ui, RowId::SYSTEM_MEMORY, DASH);
            self.value(ui, RowId::PRESSURE, DASH);
            self.value(ui, RowId::LOAD, DASH);
        }
    }

    fn system_meters(&self, ui: &mut egui::Ui, system: &SystemRows) {
        let meter = MeterColors {
            bar: self.live,
            value: self.text,
            label: self.secondary,
        };
        // One shared value column: without it each bar hugs its
        // own value and the bars scatter across the panel.
        let value_width = meter_value_width(ui, system);
        meter_row(MeterRowInput {
            ui,
            label: "cpu",
            value: &system.cpu,
            fraction: system.cpu_fraction,
            colors: meter,
            target: SYSTEM_CPU,
            value_width,
        });
        meter_row(MeterRowInput {
            ui,
            label: "memory",
            value: &system.memory,
            fraction: system.memory_fraction,
            colors: meter,
            target: SYSTEM_MEMORY,
            value_width,
        });
        let pressure_color = pressure_color(system.pressure_level, self.theme, self.text);
        meter_row(MeterRowInput {
            ui,
            label: "pressure",
            value: &system.pressure,
            fraction: system.pressure_fraction,
            colors: MeterColors {
                bar: pressure_color,
                value: pressure_color,
                label: self.secondary,
            },
            target: PRESSURE,
            value_width,
        });
        meter_row(MeterRowInput {
            ui,
            label: "load",
            value: &system.load,
            fraction: system.load_fraction,
            colors: meter,
            target: LOAD,
            value_width,
        });
    }

    /// A plain label/value row in the default colors.
    fn value(&self, ui: &mut egui::Ui, row: RowId, value: &str) -> egui::Response {
        let RowId { label, target } = row;
        value_row(ValueRowInput {
            ui,
            label,
            value,
            value_color: self.text,
            label_color: self.secondary,
            target,
        })
    }
}

fn pressure_color(
    level: Option<PressureLevel>,
    theme: &AppearanceConfig,
    text: Color32,
) -> Color32 {
    match level {
        Some(PressureLevel::Normal) => appearance::color(&theme.status_running),
        Some(PressureLevel::Warning) => appearance::color(&theme.status_waiting),
        Some(PressureLevel::Urgent | PressureLevel::Critical) => {
            appearance::color(&theme.status_failed)
        }
        None => text,
    }
}

struct SectionHeaderInput<'a> {
    ui: &'a mut egui::Ui,
    title: &'a str,
    open: &'a mut bool,
    system: Option<&'a mut bool>,
    text: Color32,
    secondary: Color32,
}

fn section_header(input: SectionHeaderInput<'_>) -> bool {
    let SectionHeaderInput {
        ui,
        title,
        open,
        system,
        text,
        secondary,
    } = input;
    let mut toggle = false;
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        let title = ui.add(
            egui::Label::new(RichText::new(title).color(text).size(13.0))
                .sense(egui::Sense::click()),
        );
        if title.clicked() {
            toggle = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if chevron(ui, *open, secondary).clicked() {
                toggle = true;
            }
            if let Some(show_system) = system {
                let response = ui.checkbox(
                    show_system,
                    RichText::new("SYSTEM").small().color(secondary),
                );
                mark(ui, SYSTEM_TOGGLE, &response);
            }
        });
    });
    if toggle {
        *open = !*open;
    }
    *open
}

fn chevron(ui: &mut egui::Ui, open: bool, tint: Color32) -> egui::Response {
    let icon = if open { "ChevronDown" } else { "ChevronRight" };
    ui.add_sized(
        [16.0, 16.0],
        egui::Button::image(
            egui::Image::new(icons::source(icon))
                .tint(tint)
                .fit_to_exact_size(egui::vec2(14.0, 14.0)),
        )
        .frame(false),
    )
}

struct ValueRowInput<'a> {
    ui: &'a mut egui::Ui,
    label: &'a str,
    value: &'a str,
    value_color: Color32,
    label_color: Color32,
    target: &'a str,
}

fn value_row(input: ValueRowInput<'_>) -> egui::Response {
    let ValueRowInput {
        ui,
        label,
        value,
        value_color,
        label_color,
        target,
    } = input;
    let mut value_response = None;
    ui.horizontal(|ui| {
        ui.set_min_height(18.0);
        ui.label(RichText::new(label).color(label_color).size(12.0));
        let response = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(value).color(value_color).size(12.0)).truncate(),
                )
            })
            .inner;
        mark(ui, target, &response);
        value_response = Some(response);
    });
    value_response.unwrap_or_else(|| ui.label(""))
}

#[derive(Clone, Copy)]
struct MeterColors {
    bar: Color32,
    value: Color32,
    label: Color32,
}

/// Widest system value, so every meter row reserves the same value cell and
/// the bars form one column. Clamped to leave room for the bar and label.
fn meter_value_width(ui: &egui::Ui, system: &SystemRows) -> f32 {
    let measured = [
        system.cpu.as_str(),
        system.memory.as_str(),
        system.pressure.as_str(),
        system.load.as_str(),
    ]
    .iter()
    .map(|value| {
        ui.painter()
            .layout_no_wrap(
                (*value).into(),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            )
            .size()
            .x
    })
    .fold(0.0, f32::max);
    let max_value = (ui.available_width() - 56.0 - 12.0 - 64.0).max(48.0);
    measured.min(max_value).max(28.0)
}

struct MeterRowInput<'a> {
    ui: &'a mut egui::Ui,
    label: &'a str,
    value: &'a str,
    fraction: f32,
    colors: MeterColors,
    target: &'a str,
    value_width: f32,
}

fn meter_row(input: MeterRowInput<'_>) {
    let MeterRowInput {
        ui,
        label,
        value,
        fraction,
        colors,
        target,
        value_width,
    } = input;
    ui.horizontal(|ui| {
        ui.set_min_height(18.0);
        ui.label(RichText::new(label).color(colors.label).size(12.0));
        let response = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let response = ui
                    .allocate_ui_with_layout(
                        egui::vec2(value_width, 18.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let response = ui.add(
                                egui::Label::new(
                                    RichText::new(value).color(colors.value).size(12.0),
                                )
                                .truncate(),
                            );
                            // The scope only advances by used space, so claim
                            // the rest of the cell to hold the column width.
                            let rest = (value_width - response.rect.width()).max(0.0);
                            ui.allocate_space(egui::vec2(rest, 18.0));
                            response
                        },
                    )
                    .inner;
                let bar = meter_bar(ui, fraction, colors.bar);
                mark_bar(ui, target, bar);
                response
            })
            .inner;
        mark(ui, target, &response);
    });
}

fn meter_bar(ui: &mut egui::Ui, fraction: f32, color: Color32) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 14.0), egui::Sense::hover());
    let bar = egui::Rect::from_center_size(rect.center(), egui::vec2(56.0, 4.0));
    ui.painter()
        .rect_filled(bar, 2.0, Color32::from_white_alpha(28));
    let fill_width = bar.width() * fraction.clamp(0.0, 1.0);
    if fill_width > 0.5 {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(bar.min, egui::vec2(fill_width, bar.height())),
            2.0,
            color,
        );
    }
    rect
}

fn mark(ui: &egui::Ui, target: &str, response: &egui::Response) {
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), target, response.rect);
    #[cfg(not(feature = "test-support"))]
    {
        let _ = (ui, target, response);
    }
}

fn mark_bar(ui: &egui::Ui, target: &str, rect: egui::Rect) {
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), &format!("{target}-bar"), rect);
    #[cfg(not(feature = "test-support"))]
    {
        let _ = (ui, target, rect);
    }
}
