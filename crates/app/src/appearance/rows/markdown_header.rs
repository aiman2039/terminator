//! The single-row Markdown pane header: title, view tabs, refresh, close.
use super::icon_color;
use eframe::egui::{self, Color32, FontId};

/// Responses for the single-row Markdown header.
pub struct MarkdownHeader {
    pub title: egui::Response,
    pub modes: [(crate::markdown::Mode, egui::Response); 3],
    pub refresh: egui::Response,
    pub close: egui::Response,
}

/// One flat row: file title, view tabs, refresh, and the existing pane close.
pub(crate) struct MarkdownHeaderInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub title: &'a str,
    pub active: bool,
    pub editing: bool,
    pub mode: crate::markdown::Mode,
}

pub fn markdown_header(input: MarkdownHeaderInput<'_>) -> MarkdownHeader {
    let MarkdownHeaderInput {
        ui,
        title,
        active,
        editing,
        mode,
    } = input;
    let (rect, row) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0, ui.visuals().panel_fill);
    let fixed = 28.0 + 24.0;
    let scale = ((rect.width() - fixed - 32.0) / 162.0).clamp(0.0, 1.0);
    let title_width = (ui
        .painter()
        .layout_no_wrap(title.into(), FontId::proportional(12.0), icon_color())
        .size()
        .x
        + 16.0)
        .min((rect.width() - fixed - 162.0 * scale).max(0.0));
    let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(title_width, rect.height()));
    let title_response = header_title(
        ui,
        HeaderTitle {
            rect: title_rect,
            id: row.id.with("title"),
            title,
            active,
            editing,
        },
    );
    let (modes, left) = mode_tabs(
        ui,
        ModeTabs {
            rect,
            row: row.id,
            left: title_rect.right(),
            scale,
            mode,
        },
    );
    let refresh_rect = egui::Rect::from_min_size(
        egui::pos2(left, rect.top()),
        egui::vec2(28.0_f32.min(rect.width()), rect.height()),
    );
    let close_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - 24.0_f32.min(rect.width()), rect.top()),
        rect.max,
    );
    MarkdownHeader {
        title: title_response,
        modes,
        refresh: header_icon(HeaderIconInput {
            ui,
            rect: refresh_rect,
            id: row.id.with("refresh"),
            icon: "RefreshCw",
            label: "Refresh preview",
        }),
        close: header_icon(HeaderIconInput {
            ui,
            rect: close_rect,
            id: row.id.with("close-pane"),
            icon: "X",
            label: "Close pane",
        }),
    }
}

struct HeaderTitle<'a> {
    rect: egui::Rect,
    id: egui::Id,
    title: &'a str,
    active: bool,
    /// The title is hidden while it is being renamed in place.
    editing: bool,
}

/// The draggable file title; also the pane's drag handle.
fn header_title(ui: &egui::Ui, input: HeaderTitle<'_>) -> egui::Response {
    let HeaderTitle {
        rect,
        id,
        title,
        active,
        editing,
    } = input;
    let response = ui
        .interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_text(title)
        .on_hover_cursor(egui::CursorIcon::Grab);
    if !editing {
        header_text(HeaderTextInput {
            ui,
            rect: rect.shrink2(egui::vec2(8.0, 0.0)),
            text: title,
            font: FontId::proportional(12.0),
            tint: if active {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().weak_text_color()
            },
        });
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), title));
    response
}

struct ModeTabs {
    rect: egui::Rect,
    row: egui::Id,
    /// Where the first tab starts.
    left: f32,
    scale: f32,
    mode: crate::markdown::Mode,
}

/// Edit, Preview and Split tabs after the title. Returns their responses
/// and where the last tab ends.
fn mode_tabs(ui: &egui::Ui, tabs: ModeTabs) -> ([(crate::markdown::Mode, egui::Response); 3], f32) {
    use crate::markdown::Mode;
    let ModeTabs {
        rect,
        row,
        left,
        scale,
        mode,
    } = tabs;
    let mut left = left;
    let modes = [
        (Mode::Edit, 44.0),
        (Mode::Preview, 70.0),
        (Mode::Split, 48.0),
    ]
    .map(|(option, width)| {
        let tab_rect = egui::Rect::from_min_size(
            egui::pos2(left, rect.top()),
            egui::vec2(width * scale, rect.height()),
        );
        left = tab_rect.right();
        let response = mode_tab(
            ui,
            ModeTab {
                rect: tab_rect,
                id: row.with(option.label()),
                label: option.label(),
                selected: option == mode,
                scale,
            },
        );
        (option, response)
    });
    (modes, left)
}

struct ModeTab<'a> {
    rect: egui::Rect,
    id: egui::Id,
    label: &'a str,
    selected: bool,
    /// Shrinks padding with the tabs on narrow panes.
    scale: f32,
}

fn mode_tab(ui: &egui::Ui, tab: ModeTab<'_>) -> egui::Response {
    let ModeTab {
        rect,
        id,
        label,
        selected,
        scale,
    } = tab;
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            0,
            if selected {
                ui.visuals().window_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    if selected {
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            egui::Stroke::new(2.0, ui.visuals().weak_text_color()),
        );
    }
    header_text(HeaderTextInput {
        ui,
        rect: rect.shrink2(egui::vec2(8.0 * scale, 0.0)),
        text: label,
        font: FontId::proportional(13.0),
        tint: if selected {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        },
    });
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label)
    });
    response
}

struct HeaderTextInput<'a> {
    ui: &'a egui::Ui,
    rect: egui::Rect,
    text: &'a str,
    font: FontId,
    tint: Color32,
}

fn header_text(input: HeaderTextInput<'_>) {
    let HeaderTextInput {
        ui,
        rect,
        text,
        font,
        tint,
    } = input;
    if rect.width() <= 0.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple(text.into(), font, tint, rect.width());
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let position = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(rect)
        .galley(position, galley, tint);
}

pub(crate) struct HeaderIconInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) rect: egui::Rect,
    pub(crate) id: egui::Id,
    pub(crate) icon: &'a str,
    pub(crate) label: &'a str,
}

fn header_icon(input: HeaderIconInput<'_>) -> egui::Response {
    let HeaderIconInput {
        ui,
        rect,
        id,
        icon,
        label,
    } = input;
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0, ui.visuals().widgets.hovered.bg_fill);
    }
    egui::Image::new(crate::icons::source(icon))
        .tint(icon_color())
        .paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0)),
        );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    response
}
