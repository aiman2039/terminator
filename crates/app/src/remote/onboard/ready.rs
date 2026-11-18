//! Step 3: connected. What the host has, Mosh (only with mosh-server), the
//! project folder, and the OSC 7 lines for the Explorer.
use super::{HostRun, osc7};
use crate::{App, appearance, vnc_ui::form};
use eframe::egui::{self, RichText};
use terminator_core::remote::host_check::HostTools;

#[derive(Default)]
struct Actions {
    add: bool,
    open: bool,
    skip: bool,
}

impl App {
    pub(super) fn onboard_ready(&mut self, ui: &mut egui::Ui) {
        let done = appearance::color(&self.theme.git_added);
        let mosh_supported = self
            .state
            .capabilities
            .iter()
            .any(|c| c == terminator_core::remote::MOSH_CAPABILITY);
        let mut actions = Actions::default();
        let Some(run) = self
            .remote_ui
            .onboard
            .as_mut()
            .and_then(super::Onboard::run_mut)
        else {
            return;
        };
        ui.horizontal(|ui| {
            let rect = ui
                .allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover())
                .0;
            appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
                ui,
                rect,
                icon: "CircleCheck",
                size: appearance::TOOLBAR_ICON,
                tint: done,
            });
            ui.label(
                RichText::new(format!("Connected to {}", run.project.name))
                    .size(appearance::FONT_HEADING)
                    .strong(),
            );
        });
        ui.add_space(6.0);
        if let Some(tools) = &run.tools {
            tool_chips(ui, tools, done);
        }
        mosh_choice(ui, run, mosh_supported);
        form::section(ui, "Project folder");
        ui.add(
            appearance::singleline(&mut run.folder)
                .font(egui::TextStyle::Monospace)
                .hint_text(run.login_root.clone())
                .desired_width(f32::INFINITY),
        );
        form::hint(ui, "Blank: the login directory.", "");
        if let Some(error) = &run.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        form::section(ui, "Explorer follows your terminal (optional)");
        form::hint(
            ui,
            "The remote shell must report its folder (OSC 7). Without it the Explorer stays at the project folder.",
            "",
        );
        let found = run.tools.as_ref().map(|t| t.shell.clone());
        osc7::card(ui, &mut run.osc7, found.as_deref());
        ui.add_space(8.0);
        ui.separator();
        let busy = run.busy;
        appearance::footer_row(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                actions.open = appearance::primary_button(ui, "Add and open terminal").clicked();
                actions.add = ui.button("Add host").clicked();
                actions.skip = ui.button("Skip this host").clicked();
            });
            if busy {
                ui.spinner();
            }
        });
        if actions.skip {
            self.skip_onboard_host();
        } else if actions.open || actions.add {
            self.add_onboard_host(actions.open);
        }
    }
}

/// One chip per tool: found ones in the done color, missing ones dim with
/// what they are for.
fn tool_chips(ui: &mut egui::Ui, tools: &HostTools, done: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        if !tools.shell.is_empty() {
            chip(ui, &tools.shell, Some(done), "Login shell");
        }
        for tool in &tools.tools {
            let purpose = match tool.name.as_str() {
                "git" => "The Git sidebar needs git on the host.",
                "nvim" => "Editors run Neovim on the host.",
                _ => "Mosh terminals need mosh-server on the host.",
            };
            if tool.found {
                let text = short_version(&tool.name, &tool.version);
                chip(ui, &text, Some(done), purpose);
            } else {
                chip(ui, &format!("{} missing", tool.name), None, purpose);
            }
        }
    });
}

fn chip(ui: &mut egui::Ui, text: &str, tint: Option<egui::Color32>, tip: &str) {
    let weak = ui.visuals().weak_text_color();
    let color = tint.unwrap_or(weak);
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(egui::CornerRadius::same(appearance::RADIUS_LARGE))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(appearance::FONT_SMALL)
                    .color(color),
            );
        })
        .response
        .on_hover_text(tip);
}

/// `git 2.43.0` from `git version 2.43.0`: the name, then the first word
/// that starts with a digit.
fn short_version(name: &str, version: &str) -> String {
    let number = version
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == 'v')
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()));
    number.map_or_else(|| name.to_owned(), |n| format!("{name} {n}"))
}

/// Mosh shows only when the host has mosh-server.
fn mosh_choice(ui: &mut egui::Ui, run: &mut HostRun, supported: bool) {
    let found = run.tools.as_ref().is_some_and(|t| t.has("mosh-server"));
    if !found {
        run.project.mosh = false;
        return;
    }
    ui.add_enabled(
        supported,
        egui::Checkbox::new(&mut run.project.mosh, "Use Mosh for terminals"),
    )
    .on_hover_text("Needs mosh on this computer and UDP ports 60000–61000 open. Files, Git and transfers still use SSH.")
    .on_disabled_hover_text("Requires an updated session service.");
}

#[cfg(test)]
mod tests {
    use super::short_version;

    #[test]
    fn chips_show_the_version_number_only() {
        assert_eq!(short_version("git", "git version 2.43.0"), "git 2.43.0");
        assert_eq!(short_version("nvim", "NVIM v0.10.2"), "nvim 0.10.2");
        assert_eq!(
            short_version("mosh-server", "mosh-server (mosh 1.4.0) [build mosh 1.4.0]"),
            "mosh-server 1.4.0"
        );
        assert_eq!(short_version("git", ""), "git");
    }
}
