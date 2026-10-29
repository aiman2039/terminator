//! The main VNC page while no session is live: an address bar that finds or
//! adds a host, and saved hosts as cards. All host editing happens in
//! Properties on this page; the sidebar only lists hosts.
use super::*;
use terminator_core::vnc::{DEFAULT_PORT, VncSessionStatus};

const CARD_WIDTH: f32 = 184.0;
const THUMB_HEIGHT: f32 = 112.0;
const CARD_HEIGHT: f32 = THUMB_HEIGHT + 48.0;
const CARD_RADIUS: f32 = 8.0;

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
        let mut actions = Vec::new();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
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
                "{count} host{} · Double-click to connect · Right-click for more",
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
        let texture = session
            .as_ref()
            .and_then(|s| self.vnc_views.get(&s.id))
            .and_then(|view| view.texture.clone());
        let status = session.as_ref().map(|s| self.vnc_status_color(s.status));
        paint_card(PaintCardInput {
            ui,
            rect,
            hovered: response.hovered(),
            profile,
            texture: texture.as_ref(),
            status,
        });
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

struct PaintCardInput<'a> {
    ui: &'a egui::Ui,
    rect: egui::Rect,
    hovered: bool,
    profile: &'a VncProfile,
    texture: Option<&'a TextureHandle>,
    status: Option<egui::Color32>,
}

/// Card body: thumbnail (live screen or a monitor glyph), status dot,
/// name and endpoint.
fn paint_card(input: PaintCardInput<'_>) {
    let PaintCardInput {
        ui,
        rect,
        hovered,
        profile,
        texture,
        status,
    } = input;
    let visuals = ui.visuals();
    let painter = ui.painter();
    let fill = if hovered {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        visuals.faint_bg_color
    };
    painter.rect_filled(rect, CARD_RADIUS, fill);
    let thumb = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 8.0, rect.top() + 8.0),
        egui::vec2(rect.width() - 16.0, THUMB_HEIGHT - 8.0),
    );
    painter.rect_filled(thumb, 4.0, visuals.extreme_bg_color);
    match texture {
        Some(texture) => {
            let fitted = fit_rect(thumb, texture.size_vec2());
            egui::Image::new((texture.id(), fitted.size())).paint_at(ui, fitted);
        }
        None => appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect: thumb,
            icon: "Monitor",
            size: 32.0,
            tint: visuals.weak_text_color(),
        }),
    }
    if let Some(color) = status {
        painter.circle_filled(
            egui::pos2(thumb.left() + 10.0, thumb.top() + 10.0),
            5.0,
            color,
        );
    }
    let text_left = rect.left() + 10.0;
    let width = rect.width() - 20.0;
    let name = painter.layout(
        profile.display_name(),
        egui::FontId::proportional(13.0),
        visuals.strong_text_color(),
        width,
    );
    painter.galley(
        egui::pos2(text_left, thumb.bottom() + 8.0),
        truncate(ui, name, width),
        visuals.strong_text_color(),
    );
    let endpoint = painter.layout(
        profile.endpoint(),
        egui::FontId::proportional(11.0),
        visuals.weak_text_color(),
        width,
    );
    painter.galley(
        egui::pos2(text_left, thumb.bottom() + 26.0),
        truncate(ui, endpoint, width),
        visuals.weak_text_color(),
    );
}

/// Single-line galley cut with an ellipsis to `width`.
fn truncate(
    ui: &egui::Ui,
    galley: std::sync::Arc<egui::Galley>,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    if galley.rows.len() <= 1 {
        return galley;
    }
    let mut job = egui::text::LayoutJob::single_section(
        galley.text().to_owned(),
        galley
            .job
            .sections
            .first()
            .map(|section| section.format.clone())
            .unwrap_or_default(),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// Largest rectangle with `size`'s aspect ratio centered in `outer`.
fn fit_rect(outer: egui::Rect, size: egui::Vec2) -> egui::Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return outer;
    }
    let scale = (outer.width() / size.x).min(outer.height() / size.y);
    egui::Rect::from_center_size(outer.center(), egui::vec2(size.x * scale, size.y * scale))
}

fn new_host_card(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT), egui::Sense::click());
    let visuals = ui.visuals();
    let stroke = egui::Stroke::new(1.0, visuals.weak_text_color());
    let fill = if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter()
        .rect(rect, CARD_RADIUS, fill, stroke, egui::StrokeKind::Inside);
    appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
        ui,
        rect: egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.center().y - 10.0),
            egui::vec2(32.0, 32.0),
        ),
        icon: "Plus",
        size: 24.0,
        tint: visuals.weak_text_color(),
    });
    ui.painter().text(
        egui::pos2(rect.center().x, rect.center().y + 18.0),
        egui::Align2::CENTER_CENTER,
        "New host",
        egui::FontId::proportional(12.0),
        visuals.weak_text_color(),
    );
    response.on_hover_text("Add a VNC host")
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
