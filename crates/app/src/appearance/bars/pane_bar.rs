//! Pieces of the terminal pane bar: slots, icons, title, and buttons.
use super::*;
use crate::appearance::{paint_focus, paint_unread_dot};

/// Branch label and button tooltips.
pub(super) struct BarTips<'a> {
    pub(super) branch: Option<&'a str>,
    pub(super) git: &'a str,
    pub(super) stack: Option<&'a str>,
    pub(super) zoom: Option<&'a str>,
    pub(super) vertical: &'a str,
    pub(super) horizontal: &'a str,
}

pub(super) struct BarActions {
    pub(super) git: egui::Response,
    pub(super) stack: Option<egui::Response>,
    pub(super) split_vertical: egui::Response,
    pub(super) split_horizontal: egui::Response,
    pub(super) zoom: Option<egui::Response>,
    pub(super) close: egui::Response,
}

pub(super) struct BarActionsInput<'a> {
    pub(super) ui: &'a egui::Ui,
    pub(super) bar_id: egui::Id,
    pub(super) slots: &'a BarSlots,
    pub(super) tips: BarTips<'a>,
}

/// The Git (with branch), stack, split and close buttons, left to right.
pub(super) fn bar_actions(input: BarActionsInput<'_>) -> BarActions {
    let BarActionsInput {
        ui,
        bar_id,
        slots,
        tips,
    } = input;
    let button = |rect, id, icon, tip| {
        bar_button(
            ui,
            BarButton {
                bar_id,
                rect,
                id,
                icon,
                tip,
                hover_tint: None,
            },
        )
    };
    let git = button(slots.git, "git", "GitBranch", tips.git);
    if let Some(branch) = tips.branch.filter(|branch| !branch.is_empty()) {
        paint_branch(ui, slots.git, branch);
    }
    let stack = slots
        .stack
        .zip(tips.stack)
        .map(|(rect, tip)| button(rect, "stack-tab", "Plus", tip));
    let split_vertical = button(slots.vertical, "split-vertical", "Columns2", tips.vertical);
    let split_horizontal = button(
        slots.horizontal,
        "split-horizontal",
        "Rows2",
        tips.horizontal,
    );
    let zoom = slots
        .zoom
        .zip(tips.zoom)
        .map(|(rect, tip)| button(rect, "zoom-pane", "Maximize2", tip));
    let close = bar_button(
        ui,
        BarButton {
            bar_id,
            rect: slots.close,
            id: "close-pane",
            icon: "X",
            tip: "Close pane",
            hover_tint: Some(ui.visuals().error_fg_color),
        },
    );
    BarActions {
        git,
        stack,
        split_vertical,
        split_horizontal,
        zoom,
        close,
    }
}

/// Branch label width beside the Git icon, capped.
pub(super) fn branch_width(ui: &egui::Ui, branch: Option<&str>) -> f32 {
    branch
        .map(|text| {
            ui.painter()
                .layout_no_wrap(text.into(), FontId::proportional(FONT_SMALL), icon_color())
                .size()
                .x
                + 6.0
        })
        .unwrap_or(0.0)
        .min(TERMINAL_BRANCH_MAX)
}

/// Right-aligned button slots, laid out right to left.
pub(super) struct BarSlots {
    pub(super) close: egui::Rect,
    pub(super) zoom: Option<egui::Rect>,
    pub(super) horizontal: egui::Rect,
    pub(super) vertical: egui::Rect,
    pub(super) stack: Option<egui::Rect>,
    pub(super) git: egui::Rect,
}

impl BarSlots {
    /// `(stack, zoom)`: whether those optional buttons show.
    pub(super) fn layout(rect: egui::Rect, git_width: f32, (stack, zoom): (bool, bool)) -> Self {
        let mut cursor = rect.right() - 2.0;
        let mut slot = |width: f32| {
            let slot = egui::Rect::from_min_max(
                egui::pos2(cursor - width, rect.center().y - TERMINAL_BUTTON / 2.0),
                egui::pos2(cursor, rect.center().y + TERMINAL_BUTTON / 2.0),
            );
            cursor -= width + 2.0;
            slot
        };
        let close = slot(TERMINAL_BUTTON);
        let zoom = zoom.then(|| slot(TERMINAL_BUTTON));
        let horizontal = slot(TERMINAL_BUTTON);
        let vertical = slot(TERMINAL_BUTTON);
        let stack = stack.then(|| slot(TERMINAL_BUTTON));
        let git = slot(git_width);
        Self {
            close,
            zoom,
            horizontal,
            vertical,
            stack,
            git,
        }
    }
}

pub(super) struct LeadingIcons<'a> {
    pub(super) brand: Option<&'a str>,
    pub(super) status_icon: Option<&'a str>,
    pub(super) status_tint: Option<Color32>,
    pub(super) spin: bool,
    pub(super) kind: Option<&'a str>,
    pub(super) title_tint: Option<Color32>,
    pub(super) unread: bool,
}

/// Leading identity icons mirror the workspace strip: stable brand plus
/// hook lifecycle status, or the session-kind glyph for plain shells.
/// Returns where the title starts.
pub(super) fn paint_leading_icons(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icons: LeadingIcons<'_>,
) -> f32 {
    let LeadingIcons {
        brand,
        status_icon,
        status_tint,
        spin,
        kind,
        title_tint,
        unread,
    } = icons;
    let mut lead = rect.left() + 8.0;
    let mut paint_lead = |ui: &mut egui::Ui, icon: &str, tint: Color32, spin: bool| {
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(lead + 6.5, rect.center().y),
            egui::vec2(13.0, 13.0),
        );
        paint_status_icon(PaintStatusIconInput {
            ui,
            rect: icon_rect,
            icon,
            tint,
            spin,
        });
        lead += TERMINAL_LEADING_SLOT;
        icon_rect
    };
    if let Some(brand) = brand {
        paint_lead(ui, brand, icon_color(), false);
    }
    if let Some(icon) = status_icon {
        let icon_rect = paint_lead(ui, icon, status_tint.unwrap_or(icon_color()), spin);
        if unread {
            paint_unread_dot(ui, icon_rect);
        }
    } else if let Some(kind) = kind {
        paint_lead(ui, kind, title_tint.unwrap_or(icon_color()), false);
    }
    lead
}

pub(super) struct BarTitle<'a> {
    pub(super) rect: egui::Rect,
    pub(super) git_left: f32,
    pub(super) lead: f32,
    pub(super) title: &'a str,
    /// Weak text after the title, cut first when room runs out.
    pub(super) subtitle: Option<&'a str>,
    pub(super) tint: Color32,
    pub(super) status: Option<Color32>,
}

/// One-line title clipped before the Git slot, then the status dot.
pub(super) fn paint_title(ui: &egui::Ui, input: BarTitle<'_>) {
    let BarTitle {
        rect,
        git_left,
        lead,
        title,
        subtitle,
        tint,
        status,
    } = input;
    let dot_gap = if status.is_some() { 14.0 } else { 0.0 };
    let title_width = (git_left - dot_gap - lead - 4.0).max(0.0);
    let mut job = egui::text::LayoutJob::simple(
        title.into(),
        FontId::proportional(FONT_SMALL),
        tint,
        title_width,
    );
    if let Some(subtitle) = subtitle {
        job.append(
            subtitle,
            8.0,
            egui::TextFormat::simple(
                FontId::proportional(FONT_SMALL),
                ui.visuals().weak_text_color(),
            ),
        );
    }
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let position = egui::pos2(lead, rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(egui::Rect::from_min_max(
            rect.min,
            egui::pos2(git_left - dot_gap - 4.0, rect.bottom()),
        ))
        .galley(position, galley, tint);
    if let Some(color) = status {
        ui.painter()
            .circle_filled(egui::pos2(git_left - 8.0, rect.center().y), 3.5, color);
    }
}

pub(super) struct BarButton<'a> {
    pub(super) bar_id: egui::Id,
    pub(super) rect: egui::Rect,
    pub(super) id: &'a str,
    pub(super) icon: &'a str,
    pub(super) tip: &'a str,
    pub(super) hover_tint: Option<Color32>,
}

pub(super) fn bar_button(ui: &egui::Ui, input: BarButton<'_>) -> egui::Response {
    let BarButton {
        bar_id,
        rect,
        id,
        icon,
        tip,
        hover_tint,
    } = input;
    let response = ui
        .interact(rect, bar_id.with(id), egui::Sense::click())
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, WIDGET_RADIUS, ui.visuals().widgets.hovered.bg_fill);
    }
    let tint = hover_tint
        .filter(|_| response.hovered())
        .unwrap_or(icon_color());
    egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .paint_at(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + TERMINAL_BUTTON / 2.0, rect.center().y),
                egui::Vec2::splat(super::super::TOOLBAR_ICON),
            ),
        );
    response
}

pub(super) fn paint_branch(ui: &egui::Ui, git_rect: egui::Rect, branch: &str) {
    let mut job = egui::text::LayoutJob::simple(
        branch.into(),
        FontId::proportional(FONT_SMALL),
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

const RESUME_LABEL: &str = "Resume";

/// The Resume chip sits just left of the Git button.
pub(super) fn resume_chip_rect(ui: &egui::Ui, git: egui::Rect) -> egui::Rect {
    let text = ui
        .painter()
        .layout_no_wrap(
            RESUME_LABEL.into(),
            FontId::proportional(FONT_SMALL),
            icon_color(),
        )
        .size()
        .x;
    let width = 6.0 + super::super::TOOLBAR_ICON + 4.0 + text + 8.0;
    egui::Rect::from_min_max(
        egui::pos2(git.left() - 4.0 - width, git.top()),
        egui::pos2(git.left() - 4.0, git.bottom()),
    )
}

/// An outlined `▷ Resume` chip. Only a click acts on it.
pub(super) fn resume_chip(
    ui: &egui::Ui,
    bar_id: egui::Id,
    rect: egui::Rect,
    tip: &str,
) -> egui::Response {
    let response = ui
        .interact(rect, bar_id.with("resume-chip"), egui::Sense::click())
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), RESUME_LABEL)
    });
    let visuals = ui.visuals();
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, WIDGET_RADIUS, visuals.widgets.hovered.bg_fill);
    }
    ui.painter().rect_stroke(
        rect,
        WIDGET_RADIUS,
        visuals.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let icon = egui::Rect::from_center_size(
        egui::pos2(
            rect.left() + 6.0 + super::super::TOOLBAR_ICON / 2.0,
            rect.center().y,
        ),
        egui::Vec2::splat(super::super::TOOLBAR_ICON),
    );
    egui::Image::new(crate::icons::source("Play"))
        .tint(icon_color())
        .paint_at(ui, icon);
    ui.painter().text(
        egui::pos2(icon.right() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        RESUME_LABEL,
        FontId::proportional(FONT_SMALL),
        visuals.text_color(),
    );
    paint_focus(ui, &response);
    response
}
