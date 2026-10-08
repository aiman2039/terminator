use super::theme::ICON_COLOR;
use eframe::egui::{self, Color32, FontId, TextStyle};
use std::time::Duration;

pub fn target_header(ui: &mut egui::Ui, label: &str, tooltip: &str) {
    wrapping_path_tooltip(
        ui,
        label,
        tooltip,
        FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );
    ui.separator();
}

/// One row when the path fits; wrap to two rows only when it does not.
pub fn wrapping_path(
    ui: &mut egui::Ui,
    path: &str,
    font: FontId,
    color: Color32,
) -> egui::Response {
    wrapping_path_tooltip(ui, path, path, font, color)
}

pub(super) fn wrapping_path_tooltip(
    ui: &mut egui::Ui,
    path: &str,
    tooltip: &str,
    font: FontId,
    color: Color32,
) -> egui::Response {
    let natural = ui
        .painter()
        .layout_no_wrap(path.to_owned(), font.clone(), color)
        .size()
        .x;
    let avail = ui.available_width().max(0.0);
    let rows = usize::from(natural > avail).saturating_add(1);
    let mut job = egui::text::LayoutJob::simple(path.to_owned(), font, color, avail);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter()
        .with_clip_rect(rect)
        .galley(rect.min, galley, color);
    response.on_hover_text(tooltip)
}

pub struct WrappingPathRow<R> {
    #[allow(dead_code, reason = "tests inspect path and action rects")]
    pub path: egui::Response,
    #[allow(dead_code, reason = "tests inspect path and action rects")]
    pub actions: R,
}

/// Path plus actions on one wrapping row: one line when they fit, wrap when they do not.
pub fn wrapping_path_row<R>(
    ui: &mut egui::Ui,
    path: &str,
    add_actions: impl FnOnce(&mut egui::Ui) -> R,
) -> WrappingPathRow<R> {
    let mut path_response = None;
    let actions = ui
        .horizontal_wrapped(|ui| {
            path_response = Some(wrapping_path(
                ui,
                path,
                FontId::proportional(12.0),
                ui.visuals().text_color(),
            ));
            add_actions(ui)
        })
        .inner;
    WrappingPathRow {
        path: path_response
            .unwrap_or_else(|| ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover())),
        actions,
    }
}
/// Git tint applies to the filename and badge; the icon stays bright.
pub fn file_row(
    ui: &mut egui::Ui,
    label: &str,
    icon: &str,
    selected: bool,
    height: f32,
    trailing: &str,
    tint: Color32,
) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().override_text_color = Some(tint);
        row(ui, label, icon, selected, height, trailing, tint)
    })
    .inner
}
pub fn project_row(
    ui: &mut egui::Ui,
    label: &str,
    icon: &str,
    selected: bool,
    height: f32,
    trailing: &str,
    tint: Color32,
) -> egui::Response {
    ui.scope(|ui| {
        ui.style_mut()
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        row(ui, label, icon, selected, height, trailing, tint)
    })
    .inner
}
/// Session row with an optional status-tinted (and spinning) icon and a
/// muted second line. The subtitle gets its own line; it never overlaps the label.
/// `brand` paints the stable agent glyph beside the status icon.
pub struct SessionRowSpec<'a> {
    pub label: &'a str,
    pub icon: &'a str,
    pub selected: bool,
    pub trailing: &'a str,
    pub tint: Color32,
    pub icon_tint: Option<Color32>,
    pub spin: bool,
    pub subtitle: Option<&'a str>,
    pub brand: Option<&'a str>,
}

pub const SESSION_ROW_HEIGHT: f32 = 24.0;
pub const SESSION_ROW_DETAIL_HEIGHT: f32 = 38.0;

pub fn session_row_spec(ui: &mut egui::Ui, spec: SessionRowSpec<'_>) -> egui::Response {
    ui.scope(|ui| {
        if !spec.selected {
            ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
        }
        row_ext(
            ui,
            RowSpec {
                label: spec.label,
                icon: spec.icon,
                selected: spec.selected,
                height: if spec.subtitle.is_some() {
                    SESSION_ROW_DETAIL_HEIGHT
                } else {
                    SESSION_ROW_HEIGHT
                },
                trailing: spec.trailing,
                tint: spec.tint,
                icon_tint: spec.icon_tint,
                spin: spec.spin,
                subtitle: spec.subtitle,
                brand: spec.brand,
                reserve_brand: true,
            },
        )
    })
    .inner
}

/// Working icons rotate slowly: one turn per `STATUS_SPIN_PERIOD`, sampled at
/// `STATUS_SPIN_INTERVAL`. The wake rate dominates the cost — each wake paints
/// the visible UI — while rotating the paint itself is a cheap transform, so
/// the slower cadence keeps CPU low however the icon moves. The phase comes
/// from the wall clock, so several icons stay in sync, and egui keeps the
/// soonest request, so they share one schedule of at most two wakes per
/// second, usually coalesced with the app's one-second heartbeat. A clipped
/// icon, a hidden sidebar, a minimized window, or an unfocused window
/// (`focused == Some(false)`) does not request another frame. Unknown focus
/// still schedules. Keyboard, pointer, and terminal updates are separate.
pub(super) const STATUS_SPIN_INTERVAL: Duration = Duration::from_millis(500);
/// One full turn takes two seconds, so consecutive 500 ms frames step 90°:
/// visibly rotating, not flashing.
const STATUS_SPIN_PERIOD: f64 = 2.0;

/// `as f32` for finite normals. Subnormals flush to zero; magnitudes past f32 saturate.
fn f32_from_f64(value: f64) -> f32 {
    let bits = value.to_bits();
    let sign = (bits >> 63) & 1;
    let exp_bits = (bits >> 52) & 0x7ff;
    let mant = bits & 0x000f_ffff_ffff_ffff;
    let Ok(exp_u) = u16::try_from(exp_bits) else {
        return 0.0;
    };
    if exp_u == 0x7ff {
        if mant != 0 {
            return f32::NAN;
        }
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    if exp_u == 0 {
        return if sign == 0 { 0.0 } else { -0.0 };
    }
    let Some(unbiased) = i32::from(exp_u).checked_sub(1023) else {
        return 0.0;
    };
    if unbiased > 127 {
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    if unbiased < -126 {
        return if sign == 0 { 0.0 } else { -0.0 };
    }
    let mut mantissa = mant >> 29;
    let remainder = mant & ((1_u64 << 29) - 1);
    if remainder > (1_u64 << 28) || (remainder == (1_u64 << 28) && mantissa & 1 == 1) {
        mantissa = mantissa.saturating_add(1);
    }
    let mut exp32 = unbiased.saturating_add(127);
    if mantissa >= (1_u64 << 23) {
        mantissa = 0;
        exp32 = exp32.saturating_add(1);
    }
    if exp32 >= 255 {
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    let Ok(exp32) = u32::try_from(exp32) else {
        return 0.0;
    };
    let Ok(mantissa) = u32::try_from(mantissa) else {
        return 0.0;
    };
    let sign = u32::try_from(sign).unwrap_or(0);
    f32::from_bits((sign << 31) | (exp32 << 23) | mantissa)
}

pub(super) fn u8_from_f32(value: f32) -> u8 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    let mut out = 0_u8;
    let mut cursor = 0.0_f32;
    while cursor + 1.0 <= value {
        cursor += 1.0;
        out = out.saturating_add(1);
    }
    out
}

/// Paint an icon, optionally rotating it to signal ongoing work.
pub fn paint_status_icon(ui: &egui::Ui, rect: egui::Rect, icon: &str, tint: Color32, spin: bool) {
    if spin && !ui.is_rect_visible(rect) {
        return;
    }
    let mut image = egui::Image::new(crate::icons::source(icon)).tint(tint);
    if spin {
        let (angle, animate) = ui.input(|input| {
            let viewport = input.viewport();
            let turns = (input.time % STATUS_SPIN_PERIOD) / STATUS_SPIN_PERIOD;
            let angle = f32_from_f64(turns) * std::f32::consts::TAU;
            let animate = viewport.focused != Some(false) && !viewport.minimized.unwrap_or(false);
            (angle, animate)
        });
        image = image.rotate(angle, egui::Vec2::splat(0.5));
        if animate {
            ui.ctx().request_repaint_after(STATUS_SPIN_INTERVAL);
        }
    }
    image.paint_at(ui, rect);
}

struct RowSpec<'a> {
    label: &'a str,
    icon: &'a str,
    selected: bool,
    height: f32,
    trailing: &'a str,
    tint: Color32,
    icon_tint: Option<Color32>,
    spin: bool,
    subtitle: Option<&'a str>,
    brand: Option<&'a str>,
    /// Reserve the brand slot even when `brand` is absent so the status icon
    /// stays in one column across rows. Session rows reserve it; plain rows
    /// (project folders, settings, headers) stay compact.
    reserve_brand: bool,
}

/// Consistent full-width native sidebar row with fixed icon and status columns.
pub fn row(
    ui: &mut egui::Ui,
    label: &str,
    icon: &str,
    selected: bool,
    height: f32,
    trailing: &str,
    tint: Color32,
) -> egui::Response {
    row_ext(
        ui,
        RowSpec {
            label,
            icon,
            selected,
            height,
            trailing,
            tint,
            icon_tint: None,
            spin: false,
            subtitle: None,
            brand: None,
            reserve_brand: false,
        },
    )
}

fn row_ext(ui: &mut egui::Ui, spec: RowSpec<'_>) -> egui::Response {
    let RowSpec {
        label,
        icon,
        selected,
        height,
        trailing,
        tint,
        icon_tint,
        spin,
        subtitle,
        brand,
        reserve_brand,
    } = spec;
    // A reserved brand slot keeps the status icon and the label in one column
    // whether or not this row paints a brand glyph.
    let reserved = reserve_brand || brand.is_some();
    let label_left = if reserved { 36.0 } else { 26.0 };
    let previous_height = ui.spacing().interact_size.y;
    ui.spacing_mut().interact_size.y = height;
    let response = ui.add_sized(
        [ui.available_width(), height],
        egui::Button::new("").frame(false),
    );
    ui.spacing_mut().interact_size.y = previous_height;
    // Keep geometry and interaction IDs stable while avoiding text layout and
    // image loading for rows outside the sidebar viewport.
    if !ui.is_rect_visible(response.rect) {
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
        });
        return response;
    }
    if response.hovered() || selected {
        ui.painter().rect_filled(
            response.rect,
            4,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    let trailing_width = if trailing.is_empty() {
        4.0
    } else {
        ui.painter()
            .layout_no_wrap(trailing.into(), FontId::proportional(12.0), tint)
            .size()
            .x
            + 12.0
    };
    let right = response.rect.right() - trailing_width;
    let rect = egui::Rect::from_min_max(
        egui::pos2(response.rect.left() + label_left, response.rect.top()),
        egui::pos2(
            right.max(response.rect.left() + label_left),
            response.rect.bottom(),
        ),
    );
    // Paint the label rather than overlaying a selectable Label widget: the row
    // must own clicks on its text as well as its icon and empty space.
    let single_line = |text: &str, font: FontId, color: Color32| {
        let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, rect.width());
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        ui.painter().layout_job(job)
    };
    let galley = single_line(
        label,
        TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
    let sub = subtitle.map(|text| {
        single_line(
            text,
            FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        )
    });
    const GAP: f32 = 1.0;
    let block = galley.size().y + sub.as_ref().map_or(0.0, |s| GAP + s.size().y);
    let top = response.rect.center().y - block * 0.5;
    let label_center = top + galley.size().y * 0.5;
    if let Some(brand) = brand {
        paint_status_icon(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(response.rect.left() + 9.0, label_center),
                egui::vec2(14.0, 14.0),
            ),
            brand,
            ICON_COLOR,
            false,
        );
    }
    let size = if brand.is_some() { 14.0 } else { 16.0 };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            response.rect.left() + if reserved { 25.0 } else { 12.0 },
            label_center,
        ),
        egui::vec2(size, size),
    );
    paint_status_icon(ui, icon_rect, icon, icon_tint.unwrap_or(ICON_COLOR), spin);
    let painter = ui.painter().with_clip_rect(rect);
    let sub_top = top + galley.size().y + GAP;
    painter.galley(
        egui::pos2(rect.left(), top),
        galley,
        ui.visuals().text_color(),
    );
    if let Some(sub) = sub {
        painter.galley(
            egui::pos2(rect.left(), sub_top),
            sub,
            ui.visuals().weak_text_color(),
        );
    }
    if trailing == "●" {
        ui.painter().circle_filled(
            egui::pos2(response.rect.right() - 10.0, label_center),
            3.5,
            tint,
        );
    } else {
        ui.painter().text(
            egui::pos2(response.rect.right() - 5.0, label_center),
            egui::Align2::RIGHT_CENTER,
            trailing,
            FontId::proportional(12.0),
            if trailing.contains(' ') {
                ui.visuals().weak_text_color()
            } else {
                tint
            },
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    response
}

/// Responses for the single-row Markdown header.
pub struct MarkdownHeader {
    pub title: egui::Response,
    pub modes: [(crate::markdown::Mode, egui::Response); 3],
    pub refresh: egui::Response,
    pub close: egui::Response,
}

/// One flat row: file title, view tabs, refresh, and the existing pane close.
pub fn markdown_header(
    ui: &mut egui::Ui,
    title: &str,
    active: bool,
    editing: bool,
    mode: crate::markdown::Mode,
) -> MarkdownHeader {
    use crate::markdown::Mode;
    let (rect, row) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0, ui.visuals().panel_fill);
    let fixed = 28.0 + 24.0;
    let scale = ((rect.width() - fixed - 32.0) / 162.0).clamp(0.0, 1.0);
    let title_width = (ui
        .painter()
        .layout_no_wrap(title.into(), FontId::proportional(12.0), ICON_COLOR)
        .size()
        .x
        + 16.0)
        .min((rect.width() - fixed - 162.0 * scale).max(0.0));
    let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(title_width, rect.height()));
    let title_response = ui
        .interact(
            title_rect,
            row.id.with("title"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_text(title)
        .on_hover_cursor(egui::CursorIcon::Grab);
    if !editing {
        header_text(
            ui,
            title_rect.shrink2(egui::vec2(8.0, 0.0)),
            title,
            FontId::proportional(12.0),
            if active {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().weak_text_color()
            },
        );
    }
    title_response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), title));
    let mut left = title_rect.right();
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
        let response = ui
            .interact(tab_rect, row.id.with(option.label()), egui::Sense::click())
            .on_hover_text(option.label())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let selected = option == mode;
        if selected || response.hovered() {
            ui.painter().rect_filled(
                tab_rect,
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
                tab_rect.x_range(),
                tab_rect.bottom() - 1.0,
                egui::Stroke::new(2.0, ui.visuals().weak_text_color()),
            );
        }
        header_text(
            ui,
            tab_rect.shrink2(egui::vec2(8.0 * scale, 0.0)),
            option.label(),
            FontId::proportional(13.0),
            if selected {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            },
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                selected,
                option.label(),
            )
        });
        (option, response)
    });
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
        refresh: header_icon(
            ui,
            refresh_rect,
            row.id.with("refresh"),
            "RefreshCw",
            "Refresh preview",
        ),
        close: header_icon(ui, close_rect, row.id.with("close-pane"), "X", "Close pane"),
    }
}

fn header_text(ui: &egui::Ui, rect: egui::Rect, text: &str, font: FontId, tint: Color32) {
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

fn header_icon(
    ui: &egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    icon: &str,
    label: &str,
) -> egui::Response {
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0, ui.visuals().widgets.hovered.bg_fill);
    }
    egui::Image::new(crate::icons::source(icon))
        .tint(ICON_COLOR)
        .paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0)),
        );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response
}
