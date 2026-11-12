//! One confirmation dialog: a body line, a weak line, optional extra
//! content, and a right-aligned footer `[Cancel] [destructive] [default]`.
//! The default action has keyboard focus and runs on Enter; Esc and the
//! title X cancel; the destructive action is never the default.
//!
//! While a confirmation shows, [`begin_confirm_frame`] takes Enter and Esc
//! out of the input at the start of the frame, before any terminal or
//! editor reads it, and the app stops terminal input
//! (`Popups::confirm_open`). A key that answers a dialog never reaches a PTY.
use super::{danger_button, primary_button};
use eframe::egui;

/// "1 session", "6 sessions": a count with its noun in dialog copy.
pub fn count_noun(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Which button answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmAnswer {
    /// Cancel, Esc, or the title X.
    Cancel,
    /// The destructive action (never the default).
    Destructive,
    /// The default action (Enter).
    Default,
}

/// One footer button. `id` names it for native fixtures; `disabled` is the
/// reason it cannot run now.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConfirmButton<'a> {
    pub label: &'a str,
    pub id: Option<&'a str>,
    pub disabled: Option<&'a str>,
}

impl<'a> ConfirmButton<'a> {
    pub const fn new(label: &'a str) -> Self {
        Self {
            label,
            id: None,
            disabled: None,
        }
    }

    #[must_use]
    pub const fn id(mut self, id: &'a str) -> Self {
        self.id = Some(id);
        self
    }

    #[must_use]
    pub const fn disabled(mut self, why: Option<&'a str>) -> Self {
        self.disabled = why;
        self
    }
}

/// The dialog's text and buttons. Without a `default`, Cancel is the
/// focused button and Enter cancels.
pub struct Confirm<'a> {
    pub body: String,
    pub detail: String,
    pub cancel: ConfirmButton<'a>,
    pub destructive: Option<ConfirmButton<'a>>,
    pub default: Option<ConfirmButton<'a>>,
}

impl<'a> Confirm<'a> {
    pub fn new(body: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            detail: detail.into(),
            cancel: ConfirmButton::new("Cancel"),
            destructive: None,
            default: None,
        }
    }

    #[must_use]
    pub const fn cancel(mut self, button: ConfirmButton<'a>) -> Self {
        self.cancel = button;
        self
    }

    #[must_use]
    pub const fn destructive(mut self, button: ConfirmButton<'a>) -> Self {
        self.destructive = Some(button);
        self
    }

    #[must_use]
    pub const fn default_action(mut self, button: ConfirmButton<'a>) -> Self {
        self.default = Some(button);
        self
    }
}

/// Keys taken for the open confirmation at the start of this frame.
#[derive(Clone, Copy, Debug, Default)]
struct ConfirmKeys {
    enter: bool,
    escape: bool,
    /// No confirmation showed last frame: focus the default button.
    first: bool,
}

fn open_id() -> egui::Id {
    egui::Id::unique("appearance-confirm-open")
}

fn keys_id() -> egui::Id {
    egui::Id::unique("appearance-confirm-keys")
}

/// Call once at the start of each main-window frame, before any widget
/// reads keys. Returns whether a confirmation showed last frame; then its
/// Enter and Esc are taken out of the input for [`confirm`].
pub fn begin_confirm_frame(ctx: &egui::Context) -> bool {
    let open = ctx
        .data_mut(|data| data.remove_temp::<bool>(open_id()))
        .unwrap_or(false);
    let keys = if open {
        ctx.input_mut(|input| ConfirmKeys {
            enter: input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            escape: input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            first: false,
        })
    } else {
        ConfirmKeys {
            first: true,
            ..ConfirmKeys::default()
        }
    };
    ctx.data_mut(|data| data.insert_temp(keys_id(), keys));
    open
}

/// Show `window` as a confirmation. `extra` adds content under the two
/// text lines. Returns the answer in the frame it is given.
pub fn confirm(
    ctx: &egui::Context,
    window: egui::Window<'_>,
    dialog: Confirm<'_>,
    extra: impl FnOnce(&mut egui::Ui),
) -> Option<ConfirmAnswer> {
    ctx.data_mut(|data| data.insert_temp(open_id(), true));
    // Only the first confirmation of a frame takes the keys.
    let keys = ctx
        .data_mut(|data| data.remove_temp::<ConfirmKeys>(keys_id()))
        .unwrap_or_default();
    let mut open = true;
    let mut answer = None;
    window
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.set_max_width(420.0);
            if !dialog.body.is_empty() {
                ui.label(&dialog.body);
            }
            if !dialog.detail.is_empty() {
                ui.weak(&dialog.detail);
            }
            extra(ui);
            ui.add_space(12.0);
            answer = footer(ui, &dialog, keys.first);
        });
    if answer.is_some() {
        return answer;
    }
    if !open || keys.escape {
        return Some(ConfirmAnswer::Cancel);
    }
    if !keys.enter {
        return None;
    }
    match dialog.default {
        None => Some(ConfirmAnswer::Cancel),
        Some(button) if button.disabled.is_none() => Some(ConfirmAnswer::Default),
        // A default that cannot run yet ignores Enter.
        Some(_) => None,
    }
}

/// The footer, right to left: default, destructive, cancel.
fn footer(ui: &mut egui::Ui, dialog: &Confirm<'_>, first: bool) -> Option<ConfirmAnswer> {
    super::super::footer_row(ui, |ui| {
        let mut answer = None;
        let mut focus = None;
        if let Some(button) = dialog.default {
            let response = add(ui, button, primary_button);
            if button.disabled.is_none() {
                focus = Some(response.id);
            }
            if response.clicked() {
                answer = Some(ConfirmAnswer::Default);
            }
        }
        if let Some(button) = dialog.destructive
            && add(ui, button, danger_button).clicked()
        {
            answer = Some(ConfirmAnswer::Destructive);
        }
        let cancel = add(ui, dialog.cancel, |ui, label| ui.button(label));
        if cancel.clicked() {
            answer = Some(ConfirmAnswer::Cancel);
        }
        if first {
            ui.memory_mut(|memory| memory.request_focus(focus.unwrap_or(cancel.id)));
        }
        answer
    })
}

fn add(
    ui: &mut egui::Ui,
    button: ConfirmButton<'_>,
    paint: impl FnOnce(&mut egui::Ui, &str) -> egui::Response,
) -> egui::Response {
    let response = super::enabled_unless(ui, button.disabled, |ui| paint(ui, button.label));
    // Only once clickable, so a fixture click waits for it.
    #[cfg(feature = "test-support")]
    if let Some(id) = button.id
        && button.disabled.is_none()
    {
        crate::diagnostics::record_ui(ui, id, response.rect);
    }
    response
}

#[cfg(test)]
#[path = "confirm_tests.rs"]
mod tests;
