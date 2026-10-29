//! The main VNC page while no session is live: an address bar that finds or
//! adds a host, and saved hosts as cards. All host editing happens in
//! Properties on this page; the sidebar only lists hosts.
use super::*;
use card::{CARD_HEIGHT, CARD_WIDTH, PaintCardInput, new_host_card, paint_card};
use terminator_core::vnc::{DEFAULT_PORT, VncSessionStatus};

/// What the user did on one card this frame.
enum CardAction {
    Connect,
    Properties,
    Duplicate,
    Remove,
}

impl App {
    /// The page body: Properties while editing, else the address book.
    pub(super) fn vnc_address_book(&mut self, ui: &mut egui::Ui, state_key: &str) {
        if self.vnc_sidebar.form_open {
            egui::ScrollArea::vertical()
                .id_salt("vnc-properties")
                .auto_shrink([false, false])
                .show(ui, |ui| self.vnc_properties(ui));
            return;
        }
        self.vnc_address_bar(ui, state_key);
        self.vnc_confirm_remove_bar(ui);
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("vnc-address-book")
            .auto_shrink([false, false])
            .show(ui, |ui| self.vnc_host_grid(ui, state_key));
    }

    /// Typing filters the cards; Enter or Connect opens the matching host,
    /// or Properties for a new one with the address filled in.
    fn vnc_address_bar(&mut self, ui: &mut egui::Ui, state_key: &str) {
        let view = self.vnc_views.entry(state_key.to_owned()).or_default();
        let (submit, address) = ui
            .horizontal(|ui| {
                let width = (ui.available_width() - 90.0).max(120.0);
                let field = ui.add(
                    appearance::singleline(&mut view.address)
                        .hint_text("Find or add a host: host:display or host::port")
                        .desired_width(width),
                );
                if field.changed() {
                    view.error = None;
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    // The cards below must not also see this Enter.
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                }
                let clicked = form::primary_button(ui, "Connect").clicked();
                (enter || clicked, view.address.trim().to_owned())
            })
            .inner;
        // A tab shows the error in its status line; the page has none.
        if state_key == page::PAGE_KEY
            && let Some(error) = self.vnc_views.get(state_key).and_then(|v| v.error.clone())
        {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
        }
        if !submit || address.is_empty() {
            return;
        }
        match parse_address(&address) {
            Ok((host, port)) => self.vnc_open_address(&host, port),
            Err(error) => {
                self.vnc_views
                    .entry(state_key.to_owned())
                    .or_default()
                    .error = Some(error)
            }
        }
    }

    fn vnc_open_address(&mut self, host: &str, port: u16) {
        let existing = self
            .state
            .vnc_profiles
            .iter()
            .find(|p| {
                p.port == port
                    && (p.host.eq_ignore_ascii_case(host)
                        || p.options.ssh.host.eq_ignore_ascii_case(host))
            })
            .cloned();
        if let Some(profile) = existing {
            self.connect_vnc_profile(&profile, None);
            return;
        }
        let editor = &mut self.vnc_sidebar;
        editor.begin_new();
        host.clone_into(&mut editor.host);
        editor.port = port.to_string();
        editor.display = display_for_port(&editor.port);
    }

    fn vnc_confirm_remove_bar(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.vnc_sidebar.confirm_remove.clone() else {
            return;
        };
        let Some(profile) = self.state.vnc_profiles.iter().find(|p| p.id == id).cloned() else {
            self.vnc_sidebar.confirm_remove = None;
            return;
        };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(format!("Remove {}?", profile.display_name()));
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

    fn vnc_host_grid(&mut self, ui: &mut egui::Ui, state_key: &str) {
        let filter = self
            .vnc_views
            .get(state_key)
            .map(|view| view.address.trim().to_lowercase())
            .unwrap_or_default();
        let mut profiles: Vec<VncProfile> = self
            .state
            .vnc_profiles
            .iter()
            .filter(|p| filter.is_empty() || profile_matches(p, &filter))
            .cloned()
            .collect();
        profiles.sort_by_key(|p| std::cmp::Reverse(p.last_connected));
        let mut actions: Vec<(VncProfile, CardAction)> =
            grid_key_action(ui, &profiles, &mut self.vnc_sidebar.selected_host)
                .into_iter()
                .collect();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(CARD_GAP, CARD_GAP);
            for profile in &profiles {
                if let Some(action) = self.vnc_host_card(ui, profile) {
                    actions.push((profile.clone(), action));
                }
            }
            if new_host_card(ui).clicked() {
                self.vnc_sidebar.begin_new();
            }
        });
        ui.add_space(10.0);
        let count = profiles.len();
        form::hint(
            ui,
            &format!(
                "{count} host{} · Double-click or Enter to connect · Right-click for more",
                if count == 1 { "" } else { "s" }
            ),
            "",
        );
        for (profile, action) in actions {
            self.vnc_card_action(&profile, action);
        }
    }

    fn vnc_card_action(&mut self, profile: &VncProfile, action: CardAction) {
        match action {
            CardAction::Connect => self.connect_vnc_profile(profile, None),
            CardAction::Properties => self.vnc_sidebar.begin_edit(profile),
            CardAction::Duplicate => {
                self.vnc_sidebar.begin_edit(profile);
                self.vnc_sidebar.editing_id = None;
                self.vnc_sidebar.credential_saved = false;
                self.vnc_sidebar.name = format!("{} copy", profile.display_name());
            }
            CardAction::Remove => self.vnc_sidebar.confirm_remove = Some(profile.id.clone()),
        }
    }

    fn vnc_host_card(&mut self, ui: &mut egui::Ui, profile: &VncProfile) -> Option<CardAction> {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT), egui::Sense::click());
        let session = self.vnc_profile_session(&profile.id);
        // The live screen while connected, else the last saved one.
        let texture = session
            .as_ref()
            .and_then(|s| self.vnc_views.get(&s.id))
            .and_then(|view| view.texture.clone())
            .or_else(|| {
                let path = self.paths.vnc_thumbnail(&profile.id);
                self.vnc_sidebar.thumbnails.get(ui.ctx(), &profile.id, path)
            });
        let status = session.as_ref().map(|s| self.vnc_status_color(s.status));
        paint_card(PaintCardInput {
            ui,
            rect,
            hovered: response.hovered(),
            selected: self.vnc_sidebar.selected_host.as_deref() == Some(profile.id.as_str()),
            profile,
            texture: texture.as_ref(),
            status,
        });
        if response.clicked() || response.secondary_clicked() {
            self.vnc_sidebar.selected_host = Some(profile.id.clone());
        }
        let response = response.on_hover_text(profile_details(profile));
        let mut action = response.double_clicked().then_some(CardAction::Connect);
        appearance::context_menu(&response, |ui| {
            let endpoint = profile.endpoint();
            appearance::target_header(ui, &endpoint, &endpoint);
            for (label, icon, item) in [
                ("Connect", "Play", CardAction::Connect),
                ("Properties", "Settings2", CardAction::Properties),
                ("Duplicate", "Copy", CardAction::Duplicate),
            ] {
                if menu_item(ui, label, icon) {
                    action = Some(item);
                }
            }
            ui.separator();
            if menu_item(ui, "Remove host…", "Trash") {
                action = Some(CardAction::Remove);
            }
        });
        action
    }

    fn vnc_profile_session(&self, profile_id: &str) -> Option<VncSession> {
        self.state
            .vnc_sessions
            .iter()
            .find(|s| {
                s.profile_id.as_deref() == Some(profile_id)
                    && s.status != VncSessionStatus::Disconnected
            })
            .cloned()
    }
}

/// Space between cards, both directions.
const CARD_GAP: f32 = 12.0;

/// Keyboard control of the cards while no text field has focus: arrows
/// move the selection, Enter connects, Delete asks to remove, and
/// Command/Ctrl+I opens Properties.
fn grid_key_action(
    ui: &egui::Ui,
    profiles: &[VncProfile],
    selected: &mut Option<String>,
) -> Option<(VncProfile, CardAction)> {
    if profiles.is_empty() || ui.ctx().memory(|m| m.focused().is_some()) {
        return None;
    }
    let columns = card_columns(ui.available_width());
    let (step, action) = ui.input(|i| {
        let step = if i.key_pressed(egui::Key::ArrowRight) {
            Step::Forward(1)
        } else if i.key_pressed(egui::Key::ArrowLeft) {
            Step::Back(1)
        } else if i.key_pressed(egui::Key::ArrowDown) {
            Step::Forward(columns)
        } else if i.key_pressed(egui::Key::ArrowUp) {
            Step::Back(columns)
        } else {
            Step::Forward(0)
        };
        let action = if i.key_pressed(egui::Key::Enter) {
            Some(CardAction::Connect)
        } else if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
            Some(CardAction::Remove)
        } else if i.modifiers.command && i.key_pressed(egui::Key::I) {
            Some(CardAction::Properties)
        } else {
            None
        };
        (step, action)
    });
    let current = selected
        .as_ref()
        .and_then(|id| profiles.iter().position(|p| &p.id == id));
    let last = profiles.len().saturating_sub(1);
    let index = match (current, step) {
        (None, Step::Forward(0)) => return None,
        (None, _) => 0,
        (Some(at), Step::Forward(n)) => at.saturating_add(n).min(last),
        (Some(at), Step::Back(n)) => at.saturating_sub(n),
    };
    let profile = profiles.get(index)?;
    *selected = Some(profile.id.clone());
    action.map(|action| (profile.clone(), action))
}

enum Step {
    Forward(usize),
    Back(usize),
}

/// Cards per row in `width`, at least one.
fn card_columns(width: f32) -> usize {
    let mut columns = 1usize;
    let mut used = CARD_WIDTH;
    while used + CARD_GAP + CARD_WIDTH <= width {
        columns = columns.saturating_add(1);
        used += CARD_GAP + CARD_WIDTH;
    }
    columns
}

/// One context-menu row; closes the menu when clicked.
fn menu_item(ui: &mut egui::Ui, label: &str, icon: &str) -> bool {
    let clicked = appearance::menu_item(appearance::MenuItemInput {
        ui,
        label,
        icon,
        shortcut: "",
    })
    .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

fn profile_matches(profile: &VncProfile, filter: &str) -> bool {
    [
        profile.name.as_str(),
        profile.host.as_str(),
        profile.options.ssh.host.as_str(),
    ]
    .iter()
    .any(|text| text.to_lowercase().contains(filter))
}

/// Endpoint plus short tags: transport, view only, saved password.
pub(super) fn profile_details(profile: &VncProfile) -> String {
    use terminator_core::vnc::Transport;
    let mut details = profile.endpoint();
    match profile.options.transport {
        Transport::Ssh => details.push_str(" · SSH"),
        Transport::Tls => details.push_str(" · TLS"),
        Transport::Direct => {}
    }
    if profile.view_only {
        details.push_str(" · View only");
    }
    if profile.credential_saved || !profile.password.is_empty() {
        details.push_str(" · Password saved");
    }
    details
}

/// `host:display` (TigerVNC style), `host::port`, or `host` (display 0).
/// IPv6 hosts go in brackets: `[::1]:1`.
pub(super) fn parse_address(input: &str) -> Result<(String, u16), String> {
    let text = input.trim();
    let text = text.strip_prefix("vnc://").unwrap_or(text);
    // `rest` is what follows the host's colon: "N" (display) or ":N" (port).
    let (host, rest) = match text.strip_prefix('[') {
        Some(inner) => {
            let (host, tail) = inner
                .split_once(']')
                .ok_or("Missing ] after the IPv6 address")?;
            if !tail.is_empty() && !tail.starts_with(':') {
                return Err(format!("Invalid address {input}"));
            }
            (host, tail.strip_prefix(':'))
        }
        None => match text.split_once(':') {
            Some((host, rest)) => (host, Some(rest)),
            None => (text, None),
        },
    };
    let host = host.trim();
    if host.is_empty() {
        return Err("Enter a host name or IP address".into());
    }
    let number = |digits: &str| {
        digits
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("Invalid number in {input}"))
    };
    let port = match rest {
        None => u32::from(DEFAULT_PORT),
        Some(rest) => match rest.strip_prefix(':') {
            Some(port) => number(port)?,
            None => u32::from(DEFAULT_PORT).saturating_add(number(rest)?),
        },
    };
    let port = u16::try_from(port)
        .ok()
        .filter(|p| *p > 0)
        .ok_or("Port must be between 1 and 65535")?;
    Ok((host.to_owned(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles(count: usize) -> Vec<VncProfile> {
        (0..count)
            .map(|n| VncProfile {
                id: format!("p{n}"),
                name: String::new(),
                host: "localhost".into(),
                port: 5900,
                username: String::new(),
                password: String::new(),
                shared: true,
                view_only: false,
                last_connected: 0,
                options: Default::default(),
                credential_saved: false,
            })
            .collect()
    }

    fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(CARD_WIDTH * 2.0 + CARD_GAP, 400.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn arrows_move_the_selection_and_enter_connects_it() {
        let ctx = egui::Context::default();
        let hosts = profiles(5);
        let mut selected = None;
        let mut run = |key, modifiers| {
            let mut result = None;
            let mut output = ctx.run_ui(press(key, modifiers), |ui| {
                result = grid_key_action(ui, &hosts, &mut selected);
            });
            output.textures_delta.clear();
            (
                result.map(|(p, a)| (p.id, matches!(a, CardAction::Connect))),
                selected.clone(),
            )
        };
        assert_eq!(
            run(egui::Key::ArrowRight, egui::Modifiers::NONE)
                .1
                .as_deref(),
            Some("p0")
        );
        assert_eq!(
            run(egui::Key::ArrowDown, egui::Modifiers::NONE)
                .1
                .as_deref(),
            Some("p2")
        );
        assert_eq!(
            run(egui::Key::ArrowLeft, egui::Modifiers::NONE)
                .1
                .as_deref(),
            Some("p1")
        );
        let (action, _) = run(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(action, Some(("p1".into(), true)));
    }

    #[test]
    fn cards_per_row_fit_the_width() {
        assert_eq!(card_columns(10.0), 1);
        assert_eq!(card_columns(CARD_WIDTH * 3.0 + CARD_GAP * 2.0), 3);
    }
}
