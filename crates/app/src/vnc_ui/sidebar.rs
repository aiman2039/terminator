use super::*;
impl App {
    pub(crate) fn vnc_sidebar(&mut self, ui: &mut egui::Ui) {
        self.vnc_sidebar_header(ui);
        ui.add_space(4.0);

        if !self.vnc_supported() {
            ui.colored_label(
                appearance::color(&self.theme.status_waiting),
                "Restart the session service to enable daemon VNC sessions.",
            );
            self.restart_session_button(ui, true);
            return;
        }

        self.vnc_sidebar_filter(ui);
        ui.add_space(6.0);

        let profiles = self.filtered_vnc_profiles();
        let live_sessions: Vec<VncSession> = self.state.vnc_sessions.clone();

        appearance::sidebar_scroll("vnc-sidebar").show(ui, |ui| {
            if !live_sessions.is_empty() {
                form::section(ui, "Active sessions");
                for session in &live_sessions {
                    ui.horizontal(|ui| self.vnc_session_row(ui, session));
                }
            }

            form::section(ui, "Saved hosts");
            if profiles.is_empty() {
                form::hint(ui, "No saved hosts. Click + to add one.", "");
            }
            for profile in &profiles {
                self.vnc_profile_card(ui, profile);
            }
        });
    }

    fn vnc_sidebar_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("VNC Connections");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "Plus", "New VNC host").clicked() {
                    self.vnc_sidebar.begin_new();
                    self.show_vnc_page();
                }
                if appearance::sidebar_action(ui, "Monitor", "Show all hosts").clicked() {
                    self.show_vnc_page();
                }
            });
        });
    }

    fn vnc_sidebar_filter(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let response = ui.add(
                appearance::singleline(&mut self.vnc_sidebar.filter)
                    .hint_text("Filter connections…")
                    .desired_width((ui.available_width() - 28.0).max(60.0)),
            );
            let _ = response;
            if !self.vnc_sidebar.filter.is_empty()
                && appearance::sidebar_action(ui, "X", "Clear filter").clicked()
            {
                self.vnc_sidebar.filter.clear();
            }
        });
    }

    fn filtered_vnc_profiles(&self) -> Vec<VncProfile> {
        let filter = self.vnc_sidebar.filter.trim().to_lowercase();
        self.state
            .vnc_profiles
            .iter()
            .filter(|p| {
                filter.is_empty()
                    || p.name.to_lowercase().contains(&filter)
                    || p.host.to_lowercase().contains(&filter)
            })
            .cloned()
            .collect()
    }

    /// A live session. Closing its last tab detaches it (it stays
    /// connected); clicking the row reattaches it in a tab.
    fn vnc_session_row(&mut self, ui: &mut egui::Ui, session: &VncSession) {
        let detached = !self.vnc_session_has_tab(&session.id);
        ui.colored_label(self.vnc_status_color(session.status), "●");
        let actions_width = appearance::TOOLBAR_BUTTON + if detached { 64.0 } else { 0.0 };
        let label = ui
            .add_sized(
                [
                    (ui.available_width() - actions_width).max(40.0),
                    appearance::TOOLBAR_BUTTON,
                ],
                egui::Label::new(RichText::new(&session.label).strong())
                    .halign(egui::Align::LEFT)
                    .truncate()
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(format!(
                "{}:{} ({}). {}",
                session.host,
                session.port,
                session.status.label(),
                if detached {
                    "Click to reattach."
                } else {
                    "Click to show."
                }
            ));
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(
            ui.ctx(),
            &format!("vnc-session:{}", session.label),
            label.rect,
        );
        if label.clicked() {
            self.focus_or_open_vnc_session(session);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let disconnect = appearance::sidebar_action(ui, "CircleStop", "Disconnect");
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(
                ui.ctx(),
                &format!("vnc-disconnect:{}", session.label),
                disconnect.rect,
            );
            if disconnect.clicked() {
                self.send(Request::VncDisconnect {
                    session: session.id.clone(),
                });
            }
            if detached {
                let weak = ui.visuals().weak_text_color();
                let _hint = ui
                    .label(RichText::new("Detached").size(11.0).color(weak))
                    .on_hover_text("Connected with no open tab. Click the name to reattach.");
                #[cfg(feature = "test-support")]
                crate::diagnostics::record(
                    ui.ctx(),
                    &format!("vnc-detached:{}", session.label),
                    _hint.rect,
                );
            }
        });
    }

    /// One saved host: name and endpoint on the left, actions on the right.
    /// The card highlights on hover; clicking the name connects.
    fn vnc_profile_card(&mut self, ui: &mut egui::Ui, profile: &VncProfile) {
        let background = ui.painter().add(egui::Shape::Noop);
        let card = egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| self.vnc_profile_actions(ui, profile));
                self.vnc_profile_confirm_remove(ui, profile);
            })
            .response;
        if ui.rect_contains_pointer(card.rect) {
            ui.painter().set(
                background,
                egui::Shape::rect_filled(card.rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill),
            );
        }
    }

    fn vnc_profile_confirm_remove(&mut self, ui: &mut egui::Ui, profile: &VncProfile) {
        if self.vnc_sidebar.confirm_remove.as_deref() != Some(profile.id.as_str()) {
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Remove this host?");
            if ui.button("Remove").clicked() {
                self.vnc_sidebar.confirm_remove = None;
                self.send(Request::VncProfileRemove {
                    id: profile.id.clone(),
                });
            }
            if ui.button("Cancel").clicked() {
                self.vnc_sidebar.confirm_remove = None;
            }
        });
    }

    fn vnc_profile_actions(&mut self, ui: &mut egui::Ui, profile: &VncProfile) {
        let actions_width = 3.0 * (appearance::TOOLBAR_BUTTON + ui.spacing().item_spacing.x);
        let text_width = (ui.available_width() - actions_width).max(40.0);
        let details = address_book::profile_details(profile);
        let connect_click = ui
            .vertical(|ui| {
                ui.set_width(text_width);
                let name = ui.add(
                    egui::Label::new(RichText::new(profile.display_name()).strong())
                        .truncate()
                        .sense(egui::Sense::click()),
                );
                let endpoint = ui.add(
                    egui::Label::new(
                        RichText::new(details)
                            .size(11.0)
                            .color(ui.visuals().weak_text_color()),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                );
                name.union(endpoint)
            })
            .inner
            .on_hover_text("Click to connect");
        if connect_click.clicked() {
            self.connect_vnc_profile(profile, None);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if appearance::sidebar_action(ui, "Trash", "Remove host").clicked() {
                self.vnc_sidebar.confirm_remove = Some(profile.id.clone());
            }
            if appearance::sidebar_action(ui, "Pencil", "Properties").clicked() {
                self.vnc_sidebar.begin_edit(profile);
                self.show_vnc_page();
            }
            if appearance::sidebar_action(ui, "Play", "Connect").clicked() {
                self.connect_vnc_profile(profile, None);
            }
        });
    }
}
