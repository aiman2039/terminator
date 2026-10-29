//! Shared look for main-pane forms (VNC connect and Properties, SSH host
//! Properties and import): small section captions, labels above fields,
//! and one accent button.
use super::*;

/// Fixed width of the port and display fields beside the host.
pub(crate) const NUMBER_WIDTH: f32 = 68.0;
const LABEL_SIZE: f32 = 11.0;
/// Below this host width, port and display move under the host.
const MIN_INLINE_HOST: f32 = 180.0;

/// Small uppercase caption that starts a group of fields.
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(LABEL_SIZE)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(2.0);
}

/// Field label, drawn above its control.
pub(crate) fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(LABEL_SIZE)
            .color(ui.visuals().weak_text_color()),
    );
}

/// Full-width single-line field with its label above.
pub(crate) fn text(
    ui: &mut egui::Ui,
    label_text: &str,
    value: &mut String,
    hint: &str,
) -> egui::Response {
    label(ui, label_text);
    ui.add(
        appearance::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    )
}

/// One-line hint under a group; `detail` shows on hover.
pub(crate) fn hint(ui: &mut egui::Ui, text: &str, detail: &str) {
    let response = ui.label(
        RichText::new(text)
            .size(LABEL_SIZE)
            .color(ui.visuals().weak_text_color()),
    );
    if !detail.is_empty() {
        response.on_hover_text(detail);
    }
}

/// Mutually exclusive choices drawn as one row of selectable chips.
pub(crate) fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    choices: &[(T, &str)],
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (choice, text) in choices {
            ui.selectable_value(value, *choice, *text);
        }
    });
}

/// The form's main action, filled with the theme accent.
pub(crate) fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let fill = ui.visuals().selection.bg_fill;
    let color = ui.visuals().selection.stroke.color;
    ui.add(egui::Button::new(RichText::new(text).strong().color(color)).fill(fill))
}

/// Password field with an eye toggle. `saved`: a password is already in
/// the keychain, so the empty field shows a mask instead of a prompt and
/// typing replaces it. Returns the field response.
pub(super) fn password(
    ui: &mut egui::Ui,
    value: &mut String,
    show: &mut bool,
    saved: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - appearance::TOOLBAR_BUTTON - 4.0).max(60.0);
        let hint = if saved { SAVED_MASK } else { "VNC password" };
        let response = ui.add(
            appearance::singleline(value)
                .password(!*show)
                .hint_text(hint)
                .desired_width(width),
        );
        let eye = if *show { "EyeOff" } else { "Eye" };
        if appearance::sidebar_action(ui, eye, "Show or hide the password").clicked() {
            *show = !*show;
        }
        response
    })
    .inner
}

/// Placeholder for a password stored in the keychain.
pub(super) const SAVED_MASK: &str = "••••••••";

/// Host, port and display, labels above: one row when the form is wide,
/// otherwise the host takes its own row. Returns whether a field lost
/// focus, so Enter can submit.
pub(super) fn endpoint(
    ui: &mut egui::Ui,
    host: &mut String,
    port: &mut String,
    display: &mut String,
) -> bool {
    let spacing = ui.spacing().item_spacing.x;
    let host_width = ui.available_width() - 2.0 * (NUMBER_WIDTH + spacing);
    if host_width < MIN_INLINE_HOST {
        let host_lost = text(ui, "Host", host, "Host name or IP address").lost_focus();
        let numbers_lost = ui
            .horizontal(|ui| port_display_fields(ui, port, display, true))
            .inner;
        return host_lost || numbers_lost;
    }
    ui.horizontal(|ui| {
        let host_lost = ui
            .vertical(|ui| {
                ui.set_width(host_width);
                label(ui, "Host");
                ui.add(
                    appearance::singleline(host)
                        .hint_text("Host name or IP address")
                        .desired_width(host_width),
                )
                .lost_focus()
            })
            .inner;
        let numbers_lost = port_display_fields(ui, port, display, true);
        host_lost || numbers_lost
    })
    .inner
}

/// Widest a form card grows in a large pane.
const CARD_WIDTH: f32 = 560.0;

/// A centered, framed card for a form in the main pane.
pub(crate) fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.vertical_centered(|ui| {
        ui.set_max_width(CARD_WIDTH.min(ui.available_width()));
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.add_space(12.0);
            egui::Frame::group(ui.style())
                .inner_margin(16.0)
                .corner_radius(8.0)
                .show(ui, add)
                .inner
        })
        .inner
    })
    .inner
}
