//! Step 1: one host field, the `~/.ssh/config` aliases as cards to tick,
//! and the less common fields folded under Advanced.
use super::{Candidate, Onboard};
use crate::{App, appearance, vnc_ui::form};
use eframe::egui::{self, RichText};

/// Width of one alias card.
const CARD_WIDTH: f32 = 196.0;

impl App {
    pub(super) fn onboard_choose(&mut self, ui: &mut egui::Ui) {
        let groups = self.group_names();
        let default_timeout = self.state.settings.ssh_command_timeout_seconds;
        let has = |capability: &str| self.state.capabilities.iter().any(|c| c == capability);
        let timeout = has(terminator_core::remote::HOST_TIMEOUT_CAPABILITY);
        let commands = has(terminator_core::remote::COMMANDS_CAPABILITY);
        let managed = self.remote_supported();
        let Some(onboard) = self.remote_ui.onboard.as_mut() else {
            return;
        };
        ui.label(
            RichText::new("Add SSH hosts")
                .size(appearance::FONT_HEADING)
                .strong(),
        );
        ui.weak("Terminator uses your OpenSSH setup and keys. Nothing is installed on the host.");
        ui.add_space(10.0);
        let submit = address_field(ui, onboard);
        ui.add_space(10.0);
        config_cards(ui, onboard);
        ui.add_space(6.0);
        egui::CollapsingHeader::new("Advanced · user, port, key, group, timeout, terminal command")
            .id_salt("ssh-add-advanced")
            .show(ui, |ui| {
                let single = onboard.host_count() <= 1;
                advanced(ui, onboard, single);
                crate::remote_ui::shared_fields(crate::remote_ui::SharedFields {
                    ui,
                    draft: &mut onboard.template,
                    groups: &groups,
                    timeout: (default_timeout, timeout),
                    commands,
                });
            });
        if let Some(error) = &onboard.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(8.0);
        let count = onboard.host_count();
        let mut connect = submit;
        let mut cancel = false;
        appearance::footer_row(ui, |ui| {
            ui.add_enabled_ui(managed && count > 0, |ui| {
                let label = if count > 1 {
                    format!("Connect {count} hosts")
                } else {
                    "Connect".to_owned()
                };
                connect |= appearance::primary_button(ui, &label).clicked();
            });
            cancel = ui.button("Cancel").clicked();
        });
        if !managed {
            form::hint(ui, "Restart the session service to add SSH hosts.", "");
        }
        if cancel {
            self.close_onboard();
        } else if connect && managed && count > 0 {
            self.start_onboard();
        }
    }
}

/// The typed host. Returns whether Enter was pressed in it.
fn address_field(ui: &mut egui::Ui, onboard: &mut Onboard) -> bool {
    form::label(ui, "Host");
    let field = ui.add(
        appearance::singleline(&mut onboard.address)
            .hint_text("Alias from ~/.ssh/config, or user@host:port")
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY),
    );
    if field.changed() {
        onboard.error = None;
    }
    field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Alias cards with a tick each, plus Select all / none.
fn config_cards(ui: &mut egui::Ui, onboard: &mut Onboard) {
    let Some(candidates) = onboard.candidates.as_mut() else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak("Reading ~/.ssh/config…");
        });
        return;
    };
    if candidates.is_empty() {
        form::hint(ui, "No unsaved hosts in ~/.ssh/config.", "");
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("From ~/.ssh/config").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (label, value) in [("Select none", false), ("Select all", true)] {
                if ui.small_button(label).clicked() {
                    for candidate in candidates.iter_mut() {
                        candidate.selected = value;
                    }
                }
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        for candidate in candidates.iter_mut() {
            alias_card(ui, candidate);
        }
    });
    let selected = candidates.iter().filter(|c| c.selected).count();
    if selected > 1 {
        form::hint(
            ui,
            &format!(
                "{selected} selected. Hosts connect one at a time, so a password or code prompt can show for each one."
            ),
            "",
        );
    }
}

fn alias_card(ui: &mut egui::Ui, candidate: &mut Candidate) {
    let accent = ui.visuals().selection.stroke.color;
    let stroke = if candidate.selected {
        egui::Stroke::new(1.0, accent)
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    egui::Frame::new()
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(appearance::RADIUS_MEDIUM))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            ui.horizontal(|ui| {
                ui.checkbox(&mut candidate.selected, "");
                ui.vertical(|ui| {
                    ui.label(RichText::new(&candidate.host.alias).strong());
                    ui.add(
                        egui::Label::new(
                            RichText::new(crate::remote_ui::config_host_summary(&candidate.host))
                                .monospace()
                                .size(appearance::FONT_CAPTION)
                                .weak(),
                        )
                        .truncate(),
                    );
                });
            });
        });
}

/// Name, user, port and key: only for a single host.
fn advanced(ui: &mut egui::Ui, onboard: &mut Onboard, single: bool) {
    ui.add_enabled_ui(single, |ui| {
        let template = &mut onboard.template;
        form::text(ui, "Name", &mut template.name, "The host name");
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                form::label(ui, "User");
                ui.add(
                    appearance::singleline(&mut template.ssh.username)
                        .hint_text("From SSH config")
                        .desired_width(180.0),
                );
            });
            ui.vertical(|ui| {
                form::label(ui, "Port");
                appearance::optional_port(ui, &mut template.ssh.port, "22");
            });
        });
        crate::remote_ui::sign_in_fields(ui, &mut template.ssh);
    })
    .response
    .on_disabled_hover_text("Name, user, port and key apply when you add one host.");
}
