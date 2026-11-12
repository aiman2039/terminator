mod pane_bar;
mod resume;

use super::rows::{PaintStatusIconInput, paint_status_icon, u8_from_f32};
use super::scale::{FONT_SMALL, WIDGET_RADIUS};
use super::theme::{color, icon_color};
use eframe::egui::{self, Color32, FontId, RichText};
use pane_bar::*;
use resume::{resume_chip, resume_chip_width};
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

/// Terminal pane top bar: glyphs, title, branch, then split right, split
/// down, zoom and close. Those four show only while the pane has focus or
/// the pointer is on the caption, and fold into one menu button (`more`)
/// when the pane is narrow.
pub struct TerminalBar {
    pub bar: egui::Response,
    /// The branch text; a click opens Git.
    pub branch: Option<egui::Response>,
    pub split_vertical: Option<egui::Response>,
    pub split_horizontal: Option<egui::Response>,
    pub zoom: Option<egui::Response>,
    pub close: Option<egui::Response>,
    /// The folded menu button: the caller opens the pane menu from it.
    pub more: Option<egui::Response>,
    /// The Resume chip, when `TerminalBarSpec::resume_tip` asked for it.
    pub resume: Option<egui::Response>,
    /// Where the title area ends (an inline rename field stops here).
    pub controls_left: f32,
}

pub struct TerminalBarSpec<'a> {
    pub title: &'a str,
    pub title_tint: Option<Color32>,
    /// The pane has focus: text-colored title and visible buttons.
    pub active: bool,
    /// Steady focus mark behind the caption; None for no fill (no focus, or
    /// a lone pane).
    pub focus_fill: Option<Color32>,
    pub branch: Option<&'a str>,
    /// Status dot; callers pass it only when no status glyph shows.
    pub status: Option<Color32>,
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
    /// The Resume chip before the branch; None hides the chip. It shows
    /// with or without focus.
    pub resume_tip: Option<ResumeChip<'a>>,
}

/// The pane caption's Resume chip.
#[derive(Clone, Copy)]
pub struct ResumeChip<'a> {
    pub tip: &'a str,
    /// No verified presence: the chip shows a `?` marker.
    pub unsure: bool,
}

/// Width of one leading caption icon slot; the glyph itself is 13pt.
pub(crate) const TERMINAL_LEADING_SLOT: f32 = 17.0;

const TERMINAL_BAR_HEIGHT: f32 = 26.0;
/// Pane bar buttons use the shared compact toolbar square.
pub(crate) const TERMINAL_BUTTON: f32 = super::TOOLBAR_BUTTON;
pub(crate) const TERMINAL_BRANCH_MAX: f32 = 96.0;
/// The narrowest title a caption keeps: the branch text yields first,
/// then the buttons fold into one menu button.
pub(crate) const TERMINAL_TITLE_MIN: f32 = 96.0;

/// Fixed width of the branch and split cluster on a tab bar, where the
/// branch slot cannot be measured per leaf.
pub fn strip_terminal_actions_width() -> f32 {
    14.0 + TERMINAL_BRANCH_MAX + 4.0 + TERMINAL_BUTTON + 4.0 + TERMINAL_BUTTON
}

/// Title color when the caller gives no tint: text while active, weak otherwise.
fn default_title_tint(ui: &egui::Ui, active: bool) -> egui::Color32 {
    if active {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    }
}

/// The focus fill behind a caption, out to the pane edges: the caption
/// sits inside the dock body margin.
fn paint_focus_fill(ui: &egui::Ui, rect: egui::Rect, fill: Color32) {
    let margin = ui.style().spacing.window_margin;
    let area = egui::Rect::from_min_max(
        egui::pos2(
            rect.left() - f32::from(margin.left),
            rect.top() - f32::from(margin.top),
        ),
        egui::pos2(rect.right() + f32::from(margin.right), rect.bottom()),
    );
    egui::Painter::new(ui.ctx().clone(), ui.layer_id(), area).rect_filled(
        area,
        egui::CornerRadius::ZERO,
        fill,
    );
}

/// The steady focus mark of a pane: a 2 px accent rule on its top edge.
pub fn paint_focus_rule(painter: &egui::Painter, pane: egui::Rect, accent: Color32) -> egui::Rect {
    let rule = egui::Rect::from_min_size(pane.min, egui::vec2(pane.width(), 2.0));
    painter.rect_filled(rule, egui::CornerRadius::ZERO, accent);
    rule
}

pub fn terminal_bar(ui: &mut egui::Ui, spec: TerminalBarSpec<'_>) -> TerminalBar {
    let (rect, bar) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TERMINAL_BAR_HEIGHT),
        egui::Sense::click_and_drag(),
    );
    if let Some(fill) = spec.focus_fill {
        paint_focus_fill(ui, rect, fill);
    }
    let show = spec.active || ui.rect_contains_pointer(rect);
    let tint = spec
        .title_tint
        .unwrap_or_else(|| default_title_tint(ui, spec.active));
    let lead = paint_leading_icons(
        ui,
        rect,
        LeadingIcons {
            brand: spec.brand,
            status_icon: spec.status_icon,
            status_tint: spec.status_tint,
            spin: spec.spin,
            kind: spec.kind,
            title_tint: Some(tint),
            unread: spec.unread,
        },
    );
    let layout = BarLayout::new(LayoutInput {
        rect,
        lead,
        branch_width: branch_width(ui, spec.branch),
        resume_width: spec
            .resume_tip
            .map(|chip| resume_chip_width(ui, chip.unsure)),
        zoom: spec.zoom_tip.is_some(),
        show,
        dot: spec.status.is_some(),
    });
    paint_title(
        ui,
        BarTitle {
            rect,
            right: layout.title_right,
            lead,
            title: spec.title,
            subtitle: spec.subtitle.filter(|_| !spec.title.is_empty()),
            tint,
            status: spec.status,
        },
    );
    let resume = layout
        .resume
        .zip(spec.resume_tip)
        .map(|(rect, chip)| resume_chip(ui, bar.id, rect, chip));
    bar.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), spec.active, spec.title)
    });
    let actions = bar_actions(BarActionsInput {
        ui,
        bar_id: bar.id,
        layout: &layout,
        spec: &spec,
        show,
    });
    let bar = bar_hover(
        bar.on_hover_cursor(egui::CursorIcon::Grab),
        spec.title,
        spec.subtitle,
    );
    TerminalBar::new(bar, actions, resume, layout.title_right)
}

impl TerminalBar {
    fn new(
        bar: egui::Response,
        actions: BarActions,
        resume: Option<egui::Response>,
        controls_left: f32,
    ) -> Self {
        Self {
            bar,
            branch: actions.branch,
            split_vertical: actions.split_vertical,
            split_horizontal: actions.split_horizontal,
            zoom: actions.zoom,
            close: actions.close,
            more: actions.more,
            resume,
            controls_left,
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

/// Brief focus emphasis that fades out; the steady mark is
/// [`paint_focus_rule`] plus the caption fill and text-colored title.
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
