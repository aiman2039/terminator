mod pane_bar;

use super::rows::{PaintStatusIconInput, paint_status_icon, u8_from_f32};
use super::scale::{FONT_SMALL, WIDGET_RADIUS};
use super::theme::{color, icon_color};
use eframe::egui::{self, Color32, FontId, RichText};
use pane_bar::*;
use terminator_core::appearance::AppearanceConfig;

pub struct UnsavedCloseBar<'a> {
    pub theme: &'a AppearanceConfig,
    pub message: &'a str,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedCloseChoice {
    Save,
    Discard,
    Cancel,
}

pub struct UnsavedCloseBarResponse {
    pub save: egui::Response,
    pub discard: egui::Response,
    pub cancel: egui::Response,
}

impl UnsavedCloseBarResponse {
    pub fn choice(&self) -> Option<UnsavedCloseChoice> {
        if self.save.clicked() {
            Some(UnsavedCloseChoice::Save)
        } else if self.discard.clicked() {
            Some(UnsavedCloseChoice::Discard)
        } else if self.cancel.clicked() {
            Some(UnsavedCloseChoice::Cancel)
        } else {
            None
        }
    }
}

/// High-visibility confirm strip inside a file pane, not a floating window.
pub fn unsaved_close_bar(ui: &mut egui::Ui, input: UnsavedCloseBar<'_>) -> UnsavedCloseBarResponse {
    let UnsavedCloseBar {
        theme,
        message,
        enabled,
    } = input;
    let warn = color(&theme.status_failed);
    // A warning wash over the panel surface reads in light and dark themes.
    let fill = color(&theme.surface).lerp_to_gamma(warn, 0.28);
    let response = egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(enabled, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(message).color(color(&theme.text)).strong())
                        .wrap(),
                );
                ui.horizontal_wrapped(|ui| {
                    let save = ui.button("Save and close");
                    let discard = ui.button("Discard changes");
                    let cancel = ui.button("Cancel");
                    UnsavedCloseBarResponse {
                        save,
                        discard,
                        cancel,
                    }
                })
                .inner
            })
            .inner
        });
    let rect = response.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
        egui::CornerRadius::ZERO,
        warn,
    );
    response.inner
}

/// Terminal pane top bar: title, Git, vertical split, horizontal split, close.
pub struct TerminalBar {
    pub bar: egui::Response,
    pub git: egui::Response,
    pub stack: Option<egui::Response>,
    pub split_vertical: egui::Response,
    pub split_horizontal: egui::Response,
    pub zoom: Option<egui::Response>,
    pub close: egui::Response,
    /// The Resume chip, when `TerminalBarSpec::resume_tip` asked for it.
    pub resume: Option<egui::Response>,
}

pub struct TerminalBarSpec<'a> {
    pub title: &'a str,
    pub title_tint: Option<Color32>,
    pub active: bool,
    pub branch: Option<&'a str>,
    pub status: Option<Color32>,
    pub git_tip: &'a str,
    /// Stack-new-tab affordance for lone panes (whose leaf tab bar, with
    /// its own `+`, is hidden). None hides the button, e.g. when the pane
    /// has no leaf to stack into.
    pub stack_tip: Option<&'a str>,
    /// Zoom (or Restore) button tip; None hides it (panes that cannot zoom).
    pub zoom_tip: Option<&'a str>,
    pub vertical_tip: &'a str,
    pub horizontal_tip: &'a str,
    /// Stable agent brand glyph, painted bright at the caption's left edge.
    pub brand: Option<&'a str>,
    /// Hook lifecycle status glyph and its tint. Spins while running.
    pub status_icon: Option<&'a str>,
    pub status_tint: Option<Color32>,
    pub spin: bool,
    /// Session-kind glyph shown when no lifecycle status is known, so every
    /// terminal caption carries an icon like the workspace strip does.
    pub kind: Option<&'a str>,
    /// Unread notices from a completed agent: a dot on the status icon.
    pub unread: bool,
    /// Weak text after the title where room allows (a window title).
    pub subtitle: Option<&'a str>,
    /// Tip of the Resume chip before the Git button; None hides the chip.
    pub resume_tip: Option<&'a str>,
}

/// Width of one leading caption icon slot; the glyph itself is 13pt.
pub(crate) const TERMINAL_LEADING_SLOT: f32 = 17.0;

const TERMINAL_BAR_HEIGHT: f32 = 26.0;
/// Pane bar buttons use the shared compact toolbar square.
pub(crate) const TERMINAL_BUTTON: f32 = super::TOOLBAR_BUTTON;
pub(crate) const TERMINAL_BRANCH_MAX: f32 = 96.0;

/// Fixed width of the Git and split cluster on a tab bar, where the branch
/// slot cannot be measured per leaf.
pub fn strip_terminal_actions_width() -> f32 {
    14.0 + TERMINAL_BUTTON
        + 4.0
        + TERMINAL_BRANCH_MAX
        + 4.0
        + TERMINAL_BUTTON
        + 4.0
        + TERMINAL_BUTTON
}

/// Title color when the caller gives no tint: accent while active, weak otherwise.
fn default_title_tint(ui: &egui::Ui, active: bool) -> egui::Color32 {
    if active {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().weak_text_color()
    }
}

pub fn terminal_bar(ui: &mut egui::Ui, spec: TerminalBarSpec<'_>) -> TerminalBar {
    let TerminalBarSpec {
        title,
        title_tint,
        active,
        branch,
        status,
        git_tip,
        stack_tip,
        zoom_tip,
        vertical_tip,
        horizontal_tip,
        brand,
        status_icon,
        status_tint,
        spin,
        kind,
        unread,
        subtitle,
        resume_tip,
    } = spec;
    let (rect, bar) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TERMINAL_BAR_HEIGHT),
        egui::Sense::click_and_drag(),
    );
    let git_width = TERMINAL_BUTTON + branch_width(ui, branch);
    let slots = BarSlots::layout(rect, git_width, (stack_tip.is_some(), zoom_tip.is_some()));
    let resume_rect = resume_tip.map(|_| resume_chip_rect(ui, slots.git));
    let tint = title_tint.unwrap_or_else(|| default_title_tint(ui, active));
    let lead = paint_leading_icons(
        ui,
        rect,
        LeadingIcons {
            brand,
            status_icon,
            status_tint,
            spin,
            kind,
            title_tint: Some(tint),
            unread,
        },
    );
    paint_title(
        ui,
        BarTitle {
            rect,
            git_left: resume_rect.map_or(slots.git.left(), |chip| chip.left() - 2.0),
            lead,
            title,
            subtitle: subtitle.filter(|_| !title.is_empty()),
            tint,
            status,
        },
    );
    let resume = resume_rect
        .zip(resume_tip)
        .map(|(chip, tip)| resume_chip(ui, bar.id, chip, tip));
    bar.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), active, title)
    });
    let actions = bar_actions(BarActionsInput {
        ui,
        bar_id: bar.id,
        slots: &slots,
        tips: BarTips {
            branch,
            git: git_tip,
            stack: stack_tip,
            zoom: zoom_tip,
            vertical: vertical_tip,
            horizontal: horizontal_tip,
        },
    });
    let bar = bar_hover(bar.on_hover_cursor(egui::CursorIcon::Grab), title, subtitle);
    TerminalBar::new(bar, actions, resume)
}

impl TerminalBar {
    fn new(bar: egui::Response, actions: BarActions, resume: Option<egui::Response>) -> Self {
        Self {
            bar,
            git: actions.git,
            stack: actions.stack,
            split_vertical: actions.split_vertical,
            split_horizontal: actions.split_horizontal,
            zoom: actions.zoom,
            close: actions.close,
            resume,
        }
    }
}

/// The bar tooltip: the title, then the subtitle on its own line.
fn bar_hover(bar: egui::Response, title: &str, subtitle: Option<&str>) -> egui::Response {
    let hover = [Some(title), subtitle]
        .into_iter()
        .flatten()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if hover.is_empty() {
        bar
    } else {
        bar.on_hover_text(hover)
    }
}

/// Brief focus emphasis that fades out; the pane title carries steady focus.
pub fn focus_stroke(accent: Color32, elapsed: std::time::Duration) -> egui::Stroke {
    let progress = ((elapsed.as_secs_f32() - 0.2) / 1.0).clamp(0.0, 1.0);
    let strength = (1.0 - progress).powi(2);
    let alpha = u8_from_f32((255.0 * strength).round());
    egui::Stroke::new(
        1.0 + strength,
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha),
    )
}

/// Cover custom click controls while preserving text, resize and drag cursors.
pub fn click_cursor(ctx: &egui::Context) {
    if ctx.output(|output| output.cursor_icon) != egui::CursorIcon::Default {
        return;
    }
    let hovered =
        ctx.interaction_snapshot(|snapshot| snapshot.hovered.iter().copied().collect::<Vec<_>>());
    if hovered
        .into_iter()
        .filter_map(|id| ctx.read_response(id))
        .any(|response| {
            response.enabled()
                && response.hovered()
                && response.sense.senses_click()
                && !response.sense.senses_drag()
        })
    {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}
