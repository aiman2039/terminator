//! Right-sidebar Info tool: the focused session and host resources.

#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{appearance, icons, resource_sample::SystemStats};
use eframe::egui::{self, Color32, RichText};
use terminator_core::appearance::AppearanceConfig;
use terminator_sys::PressureLevel;

pub const SESSION: &str = "info-session";
pub const CWD: &str = "info-cwd";
pub const BRANCH: &str = "info-branch";
pub const STARTED: &str = "info-started";
pub const SESSION_CPU: &str = "info-session-cpu";
pub const SESSION_MEMORY: &str = "info-session-memory";
pub const SYSTEM_CPU: &str = "info-system-cpu";
pub const SYSTEM_MEMORY: &str = "info-system-memory";
pub const PRESSURE: &str = "info-pressure";
pub const LOAD: &str = "info-load";
pub const SYSTEM_TOGGLE: &str = "info-system-toggle";

const DASH: &str = "—";

#[derive(Clone, Debug)]
pub struct Input {
    pub label: Option<String>,
    pub cwd: Option<String>,
    pub cwd_full: Option<String>,
    pub branch: Option<String>,
    pub started: String,
    pub unconfirmed: bool,
    pub session_cpu: Option<f32>,
    pub session_memory: Option<u64>,
    pub system: Option<SystemStats>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub session: String,
    pub cwd: String,
    pub cwd_full: Option<String>,
    pub branch: Option<String>,
    pub started: String,
    pub unconfirmed: bool,
    pub session_cpu: String,
    pub session_memory: String,
    pub system: Option<SystemRows>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemRows {
    pub cpu: String,
    pub cpu_fraction: f32,
    pub memory: String,
    pub memory_fraction: f32,
    pub pressure: String,
    pub pressure_fraction: f32,
    pub pressure_level: Option<PressureLevel>,
    pub load: String,
    pub load_fraction: f32,
}

pub struct Toggles {
    pub process_open: bool,
    pub resources_open: bool,
    pub show_system: bool,
}

#[must_use]
pub fn started_label(created: u64, now: u64, live: bool) -> String {
    if !live || created == 0 {
        return DASH.into();
    }
    let age = now.saturating_sub(created);
    if age < 90 {
        format!("{age}s ago")
    } else if age < 5_400 {
        format!("{}m ago", age / 60)
    } else if age < 86_400 * 2 {
        format!("{}h ago", age / 3_600)
    } else {
        format!("{}d ago", age / 86_400)
    }
}

#[must_use]
pub fn format_percent(value: f32) -> String {
    if !value.is_finite() || value < 0.0 {
        return DASH.into();
    }
    if value >= 10.0 || value.fract() < 0.05 {
        format!("{value:.0}%")
    } else {
        format!("{value:.1}%")
    }
}

#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bytes == 0 {
        return "0 MB".into();
    }
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.0} MB", value / MB)
    } else if value >= KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

#[must_use]
pub fn format_load(one: f64, five: f64, fifteen: f64) -> String {
    format!("{one:.2} {five:.2} {fifteen:.2}")
}

#[must_use]
pub fn model(input: &Input) -> Model {
    let system = input.system.as_ref().map(|system| {
        let pressure_fraction = system
            .pressure
            .map(|pressure| (pressure.percent / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let pressure = system
            .pressure
            .map(|pressure| {
                format!(
                    "{} · {}",
                    format_percent(pressure.percent),
                    pressure.level.label()
                )
            })
            .unwrap_or_else(|| DASH.into());
        let cores = system.cpus.max(1) as f64;
        SystemRows {
            cpu: format_percent(system.cpu),
            cpu_fraction: (system.cpu / 100.0).clamp(0.0, 1.0),
            memory: format!(
                "{} / {}",
                format_bytes(system.memory_used),
                format_bytes(system.memory_total)
            ),
            memory_fraction: if system.memory_total == 0 {
                0.0
            } else {
                (system.memory_used as f32 / system.memory_total as f32).clamp(0.0, 1.0)
            },
            pressure,
            pressure_fraction,
            pressure_level: system.pressure.map(|pressure| pressure.level),
            load: format_load(system.load_one, system.load_five, system.load_fifteen),
            load_fraction: (system.load_one / cores).clamp(0.0, 1.0) as f32,
        }
    });
    Model {
        session: input.label.clone().unwrap_or_else(|| DASH.into()),
        cwd: input.cwd.clone().unwrap_or_else(|| DASH.into()),
        cwd_full: input.cwd_full.clone(),
        branch: input.branch.clone().filter(|branch| !branch.is_empty()),
        started: input.started.clone(),
        unconfirmed: input.unconfirmed,
        session_cpu: input
            .session_cpu
            .map(format_percent)
            .unwrap_or_else(|| DASH.into()),
        session_memory: input
            .session_memory
            .map(format_bytes)
            .unwrap_or_else(|| DASH.into()),
        system,
    }
}

pub fn show(ui: &mut egui::Ui, model: &Model, toggles: &mut Toggles, theme: &AppearanceConfig) {
    ui.spacing_mut().item_spacing.y = 3.0;
    appearance::sidebar_scroll("session-info").show(ui, |ui| {
        ui.set_width(ui.available_width());
        let text = appearance::color(&theme.text);
        let secondary = appearance::color(&theme.secondary);
        let live = appearance::color(&theme.status_running);
        if section_header(
            ui,
            "Process",
            &mut toggles.process_open,
            None,
            text,
            secondary,
        ) {
            value_row(ui, "session", &model.session, text, secondary, SESSION);
            let cwd = value_row(ui, "cwd", &model.cwd, text, secondary, CWD);
            if let Some(full) = &model.cwd_full {
                cwd.on_hover_text(full);
            }
            if model.unconfirmed {
                ui.label(
                    RichText::new("Last known directory")
                        .small()
                        .color(secondary),
                );
            }
            let branch = model.branch.as_deref().unwrap_or(DASH);
            let branch_color = if model.branch.is_some() { live } else { text };
            value_row(ui, "branch", branch, branch_color, secondary, BRANCH);
            value_row(ui, "started", &model.started, text, secondary, STARTED);
        }
        ui.add_space(8.0);
        let system_toggle = toggles.show_system;
        if section_header(
            ui,
            "Resources",
            &mut toggles.resources_open,
            Some(&mut toggles.show_system),
            text,
            secondary,
        ) {
            ui.add_space(4.0);
            ui.label(RichText::new("THIS SESSION").small().color(secondary));
            value_row(ui, "cpu", &model.session_cpu, text, secondary, SESSION_CPU);
            value_row(
                ui,
                "memory",
                &model.session_memory,
                text,
                secondary,
                SESSION_MEMORY,
            );
            if system_toggle {
                ui.add_space(6.0);
                ui.label(RichText::new("SYSTEM").small().color(secondary));
                if let Some(system) = &model.system {
                    let meter = MeterColors {
                        bar: live,
                        value: text,
                        label: secondary,
                    };
                    meter_row(
                        ui,
                        "cpu",
                        &system.cpu,
                        system.cpu_fraction,
                        meter,
                        SYSTEM_CPU,
                    );
                    meter_row(
                        ui,
                        "memory",
                        &system.memory,
                        system.memory_fraction,
                        meter,
                        SYSTEM_MEMORY,
                    );
                    let pressure_color = pressure_color(system.pressure_level, theme, text);
                    meter_row(
                        ui,
                        "pressure",
                        &system.pressure,
                        system.pressure_fraction,
                        MeterColors {
                            bar: pressure_color,
                            value: pressure_color,
                            label: secondary,
                        },
                        PRESSURE,
                    );
                    meter_row(ui, "load", &system.load, system.load_fraction, meter, LOAD);
                } else {
                    value_row(ui, "cpu", DASH, text, secondary, SYSTEM_CPU);
                    value_row(ui, "memory", DASH, text, secondary, SYSTEM_MEMORY);
                    value_row(ui, "pressure", DASH, text, secondary, PRESSURE);
                    value_row(ui, "load", DASH, text, secondary, LOAD);
                }
            }
        }
    });
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

fn section_header(
    ui: &mut egui::Ui,
    title: &str,
    open: &mut bool,
    system: Option<&mut bool>,
    text: Color32,
    secondary: Color32,
) -> bool {
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

fn value_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    value_color: Color32,
    label_color: Color32,
    target: &str,
) -> egui::Response {
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

fn meter_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    fraction: f32,
    colors: MeterColors,
    target: &str,
) {
    ui.horizontal(|ui| {
        ui.set_min_height(18.0);
        ui.label(RichText::new(label).color(colors.label).size(12.0));
        let response = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let response = ui.add(
                    egui::Label::new(RichText::new(value).color(colors.value).size(12.0))
                        .truncate(),
                );
                meter_bar(ui, fraction, colors.bar);
                response
            })
            .inner;
        mark(ui, target, &response);
    });
}

fn meter_bar(ui: &mut egui::Ui, fraction: f32, color: Color32) {
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
}

fn mark(ui: &egui::Ui, target: &str, response: &egui::Response) {
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), target, response.rect);
    #[cfg(not(feature = "test-support"))]
    {
        let _ = (ui, target, response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_sample::SystemStats;
    use terminator_sys::{MemoryPressure, PressureLevel};

    fn system() -> SystemStats {
        SystemStats {
            cpu: 29.0,
            memory_used: (43.9_f64 * 1024.0 * 1024.0 * 1024.0) as u64,
            memory_total: 128 * 1024 * 1024 * 1024,
            pressure: Some(MemoryPressure {
                percent: 10.0,
                level: PressureLevel::Normal,
            }),
            load_one: 6.70,
            load_five: 5.43,
            load_fifteen: 6.15,
            cpus: 10,
        }
    }

    #[test]
    fn info_model_formats_process_and_system_rows() {
        let model = model(&Input {
            label: Some("Terminal 3".into()),
            cwd: Some("~/RustroverProjects/terminator".into()),
            cwd_full: Some("/Users/ohaddahan/RustroverProjects/terminator".into()),
            branch: Some("master".into()),
            started: "2m ago".into(),
            unconfirmed: false,
            session_cpu: None,
            session_memory: None,
            system: Some(system()),
        });
        assert_eq!(model.session, "Terminal 3");
        assert_eq!(model.cwd, "~/RustroverProjects/terminator");
        assert_eq!(model.branch.as_deref(), Some("master"));
        assert_eq!(model.started, "2m ago");
        assert_eq!(model.session_cpu, DASH);
        assert_eq!(model.session_memory, DASH);
        let system = model.system.unwrap();
        assert_eq!(system.cpu, "29%");
        assert!((system.cpu_fraction - 0.29).abs() < 0.001);
        assert!(system.memory.starts_with("43."));
        assert!(system.memory.contains("128.0 GB"));
        assert!((system.memory_fraction - 43.9 / 128.0).abs() < 0.02);
        assert_eq!(system.pressure, "10% · normal");
        assert!((system.pressure_fraction - 0.10).abs() < 0.001);
        assert_eq!(system.load, "6.70 5.43 6.15");
        assert!((system.load_fraction - 0.67).abs() < 0.001);
    }

    #[test]
    fn info_started_label_is_blank_until_the_process_is_live() {
        assert_eq!(started_label(1_000, 1_120, false), DASH);
        assert_eq!(started_label(0, 1_120, true), DASH);
        assert_eq!(started_label(1_000, 1_120, true), "2m ago");
        assert_eq!(started_label(1_000, 1_000 + 3_600 * 3, true), "3h ago");
        assert_eq!(started_label(1_000, 1_000 + 86_400 * 3, true), "3d ago");
    }

    #[test]
    fn info_bytes_use_one_decimal_for_gigabytes() {
        assert_eq!(format_bytes(0), "0 MB");
        assert_eq!(format_bytes(44 * 1024 * 1024 * 1024), "44.0 GB");
        assert_eq!(format_bytes(12 * 1024 * 1024), "12 MB");
    }
}
