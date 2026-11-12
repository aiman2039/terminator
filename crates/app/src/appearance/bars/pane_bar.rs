//! Pieces of the terminal pane bar: layout, icons, title, and buttons.
use super::*;
use crate::appearance::{paint_focus, paint_unread_dot};

/// One button slot plus its gap.
const SLOT: f32 = TERMINAL_BUTTON + 2.0;

/// Where the caption's right-hand parts sit.
#[derive(Debug)]
pub(super) struct BarLayout {
    /// Split right, split down, zoom and close; `None` when they fold.
    pub(super) buttons: Option<ButtonSlots>,
    /// The one menu button that replaces the four when the pane is narrow.
    pub(super) more: Option<egui::Rect>,
    pub(super) branch: Option<egui::Rect>,
    pub(super) resume: Option<egui::Rect>,
    /// Where the title (and the status dot) must stop.
    pub(super) title_right: f32,
}

#[derive(Debug)]
pub(super) struct ButtonSlots {
    pub(super) close: egui::Rect,
    pub(super) zoom: Option<egui::Rect>,
    pub(super) horizontal: egui::Rect,
    pub(super) vertical: egui::Rect,
}

pub(super) struct LayoutInput {
    pub(super) rect: egui::Rect,
    /// Where the title starts, after the leading glyphs.
    pub(super) lead: f32,
    pub(super) branch_width: f32,
    pub(super) resume_width: Option<f32>,
    pub(super) zoom: bool,
    /// The buttons show: the pane has focus or the pointer is on it.
    pub(super) show: bool,
    pub(super) dot: bool,
}

impl BarLayout {
    /// The buttons keep fixed slots (also while hidden, so a pointer that
    /// reaches them finds them); the branch and the Resume chip move next
    /// to whatever shows.
    pub(super) fn new(input: LayoutInput) -> Self {
        let LayoutInput {
            rect,
            lead,
            branch_width,
            resume_width,
            zoom,
            show,
            dot,
        } = input;
        let count = if zoom { 4.0 } else { 3.0 };
        let resume = resume_width.map_or(0.0, |width| width + 4.0);
        let dot = if dot { 14.0 } else { 0.0 };
        let right = rect.right() - 2.0;
        // Room for buttons and branch once the title has its minimum.
        let room = right - lead - TERMINAL_TITLE_MIN - resume - dot;
        let folded = room < SLOT * count;
        let cluster = if folded { SLOT } else { SLOT * count };
        let mut cursor = right;
        let mut slot = || {
            let slot = egui::Rect::from_min_max(
                egui::pos2(
                    cursor - TERMINAL_BUTTON,
                    rect.center().y - TERMINAL_BUTTON / 2.0,
                ),
                egui::pos2(cursor, rect.center().y + TERMINAL_BUTTON / 2.0),
            );
            cursor -= SLOT;
            slot
        };
        let (buttons, more) = if folded {
            (None, Some(slot()))
        } else {
            let close = slot();
            let zoom = zoom.then(&mut slot);
            let horizontal = slot();
            let vertical = slot();
            (
                Some(ButtonSlots {
                    close,
                    zoom,
                    horizontal,
                    vertical,
                }),
                None,
            )
        };
        let edge = if show {
            right - cluster
        } else {
            rect.right() - 6.0
        };
        let branch_room = if show { room - cluster } else { room + 4.0 };
        let branch = (branch_width > 0.0 && branch_width <= branch_room).then(|| {
            egui::Rect::from_min_max(
                egui::pos2(edge - branch_width, rect.center().y - TERMINAL_BUTTON / 2.0),
                egui::pos2(edge, rect.center().y + TERMINAL_BUTTON / 2.0),
            )
        });
        let edge = branch.map_or(edge, |branch| branch.left() - 2.0);
        let resume = resume_width.map(|width| {
            egui::Rect::from_min_max(
                egui::pos2(edge - 4.0 - width, rect.center().y - TERMINAL_BUTTON / 2.0),
                egui::pos2(edge - 4.0, rect.center().y + TERMINAL_BUTTON / 2.0),
            )
        });
        let edge = resume.map_or(edge, |chip| chip.left() - 2.0);
        Self {
            buttons,
            more,
            branch,
            resume,
            title_right: edge - dot,
        }
    }
}

#[derive(Default)]
pub(super) struct BarActions {
    pub(super) branch: Option<egui::Response>,
    pub(super) split_vertical: Option<egui::Response>,
    pub(super) split_horizontal: Option<egui::Response>,
    pub(super) zoom: Option<egui::Response>,
    pub(super) close: Option<egui::Response>,
    pub(super) more: Option<egui::Response>,
}

pub(super) struct BarActionsInput<'a> {
    pub(super) ui: &'a egui::Ui,
    pub(super) bar_id: egui::Id,
    pub(super) layout: &'a BarLayout,
    pub(super) spec: &'a TerminalBarSpec<'a>,
    pub(super) show: bool,
}

/// The branch text, then split right, split down, zoom and close (or the
/// one folded menu button). Hidden buttons still take clicks: the pointer
/// on them shows them.
pub(super) fn bar_actions(input: BarActionsInput<'_>) -> BarActions {
    let BarActionsInput {
        ui,
        bar_id,
        layout,
        spec,
        show,
    } = input;
    // On the focus fill, a hovered button uses a darker fill.
    let on_fill = spec.focus_fill.is_some();
    let branch_tip = spec
        .branch
        .map_or_else(|| "Open Git".to_owned(), |b| format!("{b}\nOpen Git"));
    let button = |rect, id, icon, tip, hover_tint| {
        bar_button(
            ui,
            BarButton {
                bar_id,
                rect,
                id,
                icon,
                tip,
                hover_tint,
                show,
                on_fill,
            },
        )
    };
    let mut actions = BarActions {
        branch: layout
            .branch
            .zip(spec.branch)
            .map(|(rect, branch)| branch_text(ui, bar_id, rect, (branch, &branch_tip))),
        ..BarActions::default()
    };
    if let Some(slots) = &layout.buttons {
        actions.split_vertical = Some(button(
            slots.vertical,
            "split-vertical",
            "Columns2",
            spec.vertical_tip,
            None,
        ));
        actions.split_horizontal = Some(button(
            slots.horizontal,
            "split-horizontal",
            "Rows2",
            spec.horizontal_tip,
            None,
        ));
        actions.zoom = slots
            .zoom
            .zip(spec.zoom_tip)
            .map(|(rect, tip)| button(rect, "zoom-pane", "Maximize2", tip, None));
        actions.close = Some(button(
            slots.close,
            "close-pane",
            "X",
            "Close pane",
            Some(ui.visuals().error_fg_color),
        ));
    }
    actions.more = layout
        .more
        .map(|rect| button(rect, "pane-more", "EllipsisVertical", "Pane actions", None));
    actions
}

/// Branch label width, capped.
pub(super) fn branch_width(ui: &egui::Ui, branch: Option<&str>) -> f32 {
    branch
        .filter(|text| !text.is_empty())
        .map(|text| {
            ui.painter()
                .layout_no_wrap(text.into(), FontId::proportional(FONT_SMALL), icon_color())
                .size()
                .x
                + 4.0
        })
        .unwrap_or(0.0)
        .min(TERMINAL_BRANCH_MAX)
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
    /// Where the title stops: the first right-hand part.
    pub(super) right: f32,
    pub(super) lead: f32,
    pub(super) title: &'a str,
    /// Weak text after the title, cut first when room runs out.
    pub(super) subtitle: Option<&'a str>,
    pub(super) tint: Color32,
    pub(super) status: Option<Color32>,
}

/// One-line title clipped before the right-hand parts, then the status dot.
pub(super) fn paint_title(ui: &egui::Ui, input: BarTitle<'_>) {
    let BarTitle {
        rect,
        right,
        lead,
        title,
        subtitle,
        tint,
        status,
    } = input;
    let title_width = (right - lead - 4.0).max(0.0);
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
            egui::pos2(right - 4.0, rect.bottom()),
        ))
        .galley(position, galley, tint);
    if let Some(color) = status {
        ui.painter()
            .circle_filled(egui::pos2(right + 6.0, rect.center().y), 3.5, color);
    }
}

pub(super) struct BarButton<'a> {
    pub(super) bar_id: egui::Id,
    pub(super) rect: egui::Rect,
    pub(super) id: &'a str,
    pub(super) icon: &'a str,
    pub(super) tip: &'a str,
    pub(super) hover_tint: Option<Color32>,
    /// Paint the button; a hidden one still takes a click.
    pub(super) show: bool,
    pub(super) on_fill: bool,
}

pub(super) fn bar_button(ui: &egui::Ui, input: BarButton<'_>) -> egui::Response {
    let BarButton {
        bar_id,
        rect,
        id,
        icon,
        tip,
        hover_tint,
        show,
        on_fill,
    } = input;
    let response = ui
        .interact(rect, bar_id.with(id), egui::Sense::click())
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
    // Keyboard focus on a hidden button shows it.
    if !show && !response.has_focus() {
        return response;
    }
    if response.hovered() {
        let widgets = &ui.visuals().widgets;
        let fill = if on_fill {
            widgets.active.bg_fill
        } else {
            widgets.hovered.bg_fill
        };
        ui.painter().rect_filled(rect, WIDGET_RADIUS, fill);
    }
    let tint = hover_tint
        .filter(|_| response.hovered())
        .unwrap_or(icon_color());
    egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .paint_at(
            ui,
            egui::Rect::from_center_size(
                rect.center(),
                egui::Vec2::splat(super::super::TOOLBAR_ICON),
            ),
        );
    paint_focus(ui, &response);
    response
}

/// The branch as plain weak text; a click opens Git.
fn branch_text(
    ui: &egui::Ui,
    bar_id: egui::Id,
    rect: egui::Rect,
    (branch, tip): (&str, &str),
) -> egui::Response {
    let response = ui
        .interact(rect, bar_id.with("git"), egui::Sense::click())
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
    let mut job = egui::text::LayoutJob::simple(
        branch.into(),
        FontId::proportional(FONT_SMALL),
        ui.visuals().weak_text_color(),
        rect.width(),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    ui.painter().with_clip_rect(rect).galley(
        egui::pos2(
            rect.right() - galley.size().x,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        ui.visuals().weak_text_color(),
    );
    paint_focus(ui, &response);
    response
}
