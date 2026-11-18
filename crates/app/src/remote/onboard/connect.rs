//! Step 2: one checklist per host. Read config, host key, sign in, check
//! the host. The host-key trust and the sign-in prompts show in their row.
use super::{HostRun, Phase};
use crate::{App, appearance, vnc_ui::form};
use eframe::egui::{self, RichText};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Done,
    Active,
    Waiting,
    Failed,
}

/// What the user did this frame.
#[derive(Default)]
struct Actions {
    trust: bool,
    answer: bool,
    cancel_prompt: bool,
    retry: bool,
    skip: bool,
    back: bool,
}

impl App {
    pub(super) fn onboard_connect(&mut self, ui: &mut egui::Ui) {
        let colors = Colors {
            done: appearance::color(&self.theme.git_added),
            failed: appearance::color(&self.theme.status_failed),
        };
        let mut actions = Actions::default();
        let Some(onboard) = self.remote_ui.onboard.as_mut() else {
            return;
        };
        let total = onboard.runs.len();
        let index = onboard.current;
        let Some(run) = onboard.run_mut() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Connecting to {}", run.project.name))
                    .size(appearance::FONT_HEADING)
                    .strong(),
            );
            if total > 1 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("Host {} of {total}", index.saturating_add(1)));
                });
            }
        });
        ui.add_space(8.0);
        checklist(ui, run, colors, &mut actions);
        ui.add_space(8.0);
        ui.separator();
        appearance::footer_row(ui, |ui| {
            if run.phase == Phase::Failed {
                actions.retry |= appearance::primary_button(ui, "Retry").clicked();
            }
            actions.skip |= ui.button("Skip this host").clicked();
            actions.back |= ui.button("Back").clicked();
        });
        self.apply_connect_actions(actions);
    }

    fn apply_connect_actions(&mut self, actions: Actions) {
        if actions.trust {
            self.trust_onboard_key();
        }
        if actions.answer || actions.cancel_prompt {
            self.answer_onboard_prompt(actions.cancel_prompt);
        }
        if actions.retry {
            self.retry_onboard_host();
        }
        if actions.skip {
            self.skip_onboard_host();
        } else if actions.back {
            self.answer_onboard_prompt(true);
            if let Some(onboard) = self.remote_ui.onboard.as_mut() {
                onboard.stage = super::Stage::Choose;
                onboard.runs.clear();
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Colors {
    done: egui::Color32,
    failed: egui::Color32,
}

fn checklist(ui: &mut egui::Ui, run: &mut HostRun, colors: Colors, actions: &mut Actions) {
    let probing = run.phase == Phase::Probing;
    row(ui, (1, Row::Done, colors), "Read SSH config", |ui| {
        ui.label(
            RichText::new(&run.summary)
                .monospace()
                .size(appearance::FONT_SMALL)
                .weak(),
        );
    });
    let key_state = match run.phase {
        _ if run.key_ok => Row::Done,
        Phase::ChangedKey => Row::Failed,
        Phase::HostKey | Phase::Probing => Row::Active,
        Phase::Failed => Row::Failed,
        _ => Row::Waiting,
    };
    row(ui, (2, key_state, colors), "Host key", |ui| {
        host_key(ui, run, actions)
    });
    let auth_state = match run.phase {
        _ if run.auth_ok => Row::Done,
        Phase::SigningIn => Row::Active,
        Phase::Probing if run.key_ok => Row::Active,
        Phase::Failed if run.key_ok => Row::Failed,
        _ => Row::Waiting,
    };
    row(ui, (3, auth_state, colors), "Sign in", |ui| {
        sign_in(ui, run, actions)
    });
    let check_state = if probing && run.key_ok && run.auth_ok {
        Row::Active
    } else {
        Row::Waiting
    };
    row(ui, (4, check_state, colors), "Check the host", |ui| {
        ui.weak("shell, git, nvim, mosh-server");
    });
    row(ui, (5, Row::Waiting, colors), "Pick a folder", |ui| {
        ui.weak("and, if you want, let the Explorer follow your terminal");
    });
    if run.phase == Phase::Failed
        && let Some(error) = &run.error
    {
        ui.colored_label(colors.failed, error);
    }
}

fn row(
    ui: &mut egui::Ui,
    (number, state, colors): (usize, Row, Colors),
    title: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal_top(|ui| {
        let size = egui::vec2(22.0, 22.0);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let center = rect.center();
        let painter = ui.painter();
        let weak = ui.visuals().weak_text_color();
        let mark = |icon, tint: egui::Color32| {
            painter.circle_filled(center, 10.0, tint.gamma_multiply(0.25));
            appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
                ui,
                rect,
                icon,
                size: appearance::TOOLBAR_ICON,
                tint,
            });
        };
        match state {
            Row::Done => mark("Check", colors.done),
            Row::Failed => mark("X", colors.failed),
            Row::Active => {
                egui::Spinner::new().paint_at(ui, rect.shrink(3.0));
            }
            Row::Waiting => {
                painter.circle_stroke(center, 10.0, egui::Stroke::new(1.0, weak));
                painter.text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    number.to_string(),
                    egui::FontId::proportional(appearance::FONT_CAPTION),
                    weak,
                );
            }
        }
        ui.vertical(|ui| {
            let text = RichText::new(title).strong();
            ui.label(if state == Row::Waiting {
                text.weak()
            } else {
                text
            });
            body(ui);
        });
    });
    ui.add_space(6.0);
}

fn host_key(ui: &mut egui::Ui, run: &HostRun, actions: &mut Actions) {
    match run.phase {
        Phase::HostKey => {
            ui.label("First connection. Compare these fingerprints with ones from the host's administrator.");
            match &run.keys {
                None if run.busy => {
                    ui.weak("Reading host keys…");
                }
                None => {}
                Some(scan) => {
                    for key in &scan.keys {
                        ui.label(
                            RichText::new(format!("{}  {}", key.key_type, key.fingerprint))
                                .monospace()
                                .size(appearance::FONT_SMALL),
                        );
                    }
                    ui.add_enabled_ui(!run.busy && !scan.keys.is_empty(), |ui| {
                        actions.trust |= ui.button("Trust and continue").clicked();
                    });
                }
            }
            if let Some(error) = &run.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        }
        Phase::ChangedKey => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "The host key does not match known_hosts. Someone may be intercepting the connection, or the host was reinstalled. Terminator will not replace the key.",
            );
            ui.label("If you trust the change, remove the old key in a terminal, then retry:");
            ui.label(
                RichText::new(format!(
                    "ssh-keygen -R {}",
                    terminator_core::quote(&run.project.ssh.host)
                ))
                .monospace(),
            );
        }
        _ if run.key_ok => {
            ui.weak("Trusted");
        }
        _ => {
            ui.weak("Connecting…");
        }
    }
}

fn sign_in(ui: &mut egui::Ui, run: &mut HostRun, actions: &mut Actions) {
    if run.auth_ok {
        ui.weak("Signed in");
        return;
    }
    if run.phase != Phase::SigningIn {
        if run.phase == Phase::Probing && run.key_ok {
            ui.weak("Trying your keys and SSH agent…");
        }
        return;
    }
    let Some(prompt) = run.prompt.clone() else {
        ui.weak("Waiting for OpenSSH…");
        return;
    };
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(egui::CornerRadius::same(appearance::RADIUS_MEDIUM))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.weak("OpenSSH asks:");
            ui.label(RichText::new(&prompt.text).monospace());
            ui.horizontal(|ui| {
                let field = ui.add(
                    appearance::singleline(&mut run.answer)
                        .password(true)
                        .desired_width(220.0),
                );
                if !field.has_focus() && run.answer.is_empty() {
                    field.request_focus();
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                actions.answer |= enter | appearance::primary_button(ui, "Continue").clicked();
                actions.cancel_prompt |= ui.button("Cancel").clicked();
            });
            form::hint(
                ui,
                "Sent only to ssh. Not saved. The shared connection keeps you signed in while it stays open.",
                "",
            );
        });
    if let Some(error) = &run.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
}
