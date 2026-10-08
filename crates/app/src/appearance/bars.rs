use super::rows::{paint_status_icon, u8_from_f32};
use super::theme::{ICON_COLOR, color};
use eframe::egui::{self, Color32, FontId, RichText};
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
    let channel = |value: u8, divisor: u8| value.checked_div(divisor).unwrap_or(0);
    let fill = Color32::from_rgb(
        channel(warn.r(), 2).saturating_add(24),
        channel(warn.g(), 6),
        channel(warn.b(), 6),
    );
    let response = egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_enabled_ui(enabled, |ui| {
                ui.add(egui::Label::new(RichText::new(message).color(ICON_COLOR).strong()).wrap());
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
        0,
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
    pub close: egui::Response,
}

pub struct TerminalBarSpec<'a> {
    pub title: &'a str,
    pub active: bool,
    pub branch: Option<&'a str>,
    pub status: Option<Color32>,
    pub git_tip: &'a str,
    /// Stack-new-tab affordance for lone panes (whose leaf tab bar, with
    /// its own `+`, is hidden). None hides the button, e.g. when the pane
    /// has no leaf to stack into.
    pub stack_tip: Option<&'a str>,
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
}

/// Width of one leading caption icon slot; the glyph itself is 13pt.
pub(crate) const TERMINAL_LEADING_SLOT: f32 = 17.0;

const TERMINAL_BAR_HEIGHT: f32 = 26.0;
pub(crate) const TERMINAL_BUTTON: f32 = 22.0;
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

pub fn terminal_bar(ui: &mut egui::Ui, spec: TerminalBarSpec<'_>) -> TerminalBar {
    let TerminalBarSpec {
        title,
        active,
        branch,
        status,
        git_tip,
        stack_tip,
        vertical_tip,
        horizontal_tip,
        brand,
        status_icon,
        status_tint,
        spin,
        kind,
    } = spec;
    let (rect, bar) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TERMINAL_BAR_HEIGHT),
        egui::Sense::click_and_drag(),
    );
    let branch_width = branch
        .map(|text| {
            ui.painter()
                .layout_no_wrap(text.into(), FontId::proportional(12.0), ICON_COLOR)
                .size()
                .x
                + 6.0
        })
        .unwrap_or(0.0)
        .min(TERMINAL_BRANCH_MAX);
    let git_width = TERMINAL_BUTTON + branch_width;
    let mut cursor = rect.right() - 2.0;
    let slot = |cursor: &mut f32, width: f32| {
        let slot = egui::Rect::from_min_max(
            egui::pos2(*cursor - width, rect.center().y - TERMINAL_BUTTON / 2.0),
            egui::pos2(*cursor, rect.center().y + TERMINAL_BUTTON / 2.0),
        );
        *cursor -= width + 2.0;
        slot
    };
    let close_rect = slot(&mut cursor, TERMINAL_BUTTON);
    let horizontal_rect = slot(&mut cursor, TERMINAL_BUTTON);
    let vertical_rect = slot(&mut cursor, TERMINAL_BUTTON);
    let stack_rect = stack_tip
        .is_some()
        .then(|| slot(&mut cursor, TERMINAL_BUTTON));
    let git_rect = slot(&mut cursor, git_width);
    let tint = if active {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().weak_text_color()
    };
    // Leading identity icons mirror the workspace strip: stable brand plus
    // hook lifecycle status, or the session-kind glyph for plain shells.
    let mut lead = rect.left() + 8.0;
    let mut paint_lead = |ui: &mut egui::Ui, icon: &str, tint: Color32, spin: bool| {
        paint_status_icon(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(lead + 6.5, rect.center().y),
                egui::vec2(13.0, 13.0),
            ),
            icon,
            tint,
            spin,
        );
        lead += TERMINAL_LEADING_SLOT;
    };
    if let Some(brand) = brand {
        paint_lead(ui, brand, ICON_COLOR, false);
    }
    if let Some(icon) = status_icon {
        paint_lead(ui, icon, status_tint.unwrap_or(ICON_COLOR), spin);
    } else if let Some(kind) = kind {
        paint_lead(ui, kind, ICON_COLOR, false);
    }
    let dot_gap = if status.is_some() { 14.0 } else { 0.0 };
    let title_width = (git_rect.left() - dot_gap - lead - 4.0).max(0.0);
    let mut job =
        egui::text::LayoutJob::simple(title.into(), FontId::proportional(12.0), tint, title_width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let position = egui::pos2(lead, rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(egui::Rect::from_min_max(
            rect.min,
            egui::pos2(git_rect.left() - dot_gap - 4.0, rect.bottom()),
        ))
        .galley(position, galley, tint);
    if let Some(color) = status {
        ui.painter().circle_filled(
            egui::pos2(git_rect.left() - 8.0, rect.center().y),
            3.5,
            color,
        );
    }
    bar.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            active,
            title,
        )
    });
    let bar_id = bar.id;
    let button =
        |rect: egui::Rect, id: &str, icon: &str, tip: &str, hover_tint: Option<Color32>| {
            let response = ui
                .interact(rect, bar_id.with(id), egui::Sense::click())
                .on_hover_text(tip)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip)
            });
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
            }
            let tint = hover_tint
                .filter(|_| response.hovered())
                .unwrap_or(ICON_COLOR);
            egui::Image::new(crate::icons::source(icon))
                .tint(tint)
                .paint_at(
                    ui,
                    egui::Rect::from_center_size(
                        egui::pos2(rect.left() + TERMINAL_BUTTON / 2.0, rect.center().y),
                        egui::vec2(14.0, 14.0),
                    ),
                );
            response
        };
    let git = button(git_rect, "git", "GitBranch", git_tip, None);
    if let Some(branch) = branch.filter(|branch| !branch.is_empty()) {
        let mut job = egui::text::LayoutJob::simple(
            branch.into(),
            FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
            (git_rect.width() - TERMINAL_BUTTON).max(0.0),
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.painter().with_clip_rect(git_rect).galley(
            egui::pos2(
                git_rect.left() + TERMINAL_BUTTON,
                git_rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            ui.visuals().weak_text_color(),
        );
    }
    let stack = stack_rect
        .zip(stack_tip)
        .map(|(rect, tip)| button(rect, "stack-tab", "Plus", tip, None));
    let split_vertical = button(
        vertical_rect,
        "split-vertical",
        "Columns2",
        vertical_tip,
        None,
    );
    let split_horizontal = button(
        horizontal_rect,
        "split-horizontal",
        "Rows2",
        horizontal_tip,
        None,
    );
    let close = button(
        close_rect,
        "close-pane",
        "X",
        "Close pane",
        Some(ui.visuals().error_fg_color),
    );
    let bar = bar.on_hover_cursor(egui::CursorIcon::Grab);
    TerminalBar {
        bar: if title.is_empty() {
            bar
        } else {
            bar.on_hover_text(title)
        },
        git,
        stack,
        split_vertical,
        split_horizontal,
        close,
    }
}

/// Brief focus emphasis, fading to a thin, translucent steady-state outline.
pub fn focus_stroke(accent: Color32, elapsed: std::time::Duration) -> egui::Stroke {
    let progress = ((elapsed.as_secs_f32() - 0.2) / 1.0).clamp(0.0, 1.0);
    let strength = (1.0 - progress).powi(2);
    let alpha = u8_from_f32((70.0 + 185.0 * strength).round());
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
