mod list;
mod markdown_header;

use super::theme::icon_color;
use eframe::egui::{self, Color32, FontId, TextStyle};
pub(crate) use list::RowInput;
pub use list::row;
use list::{RowSpec, row_ext};
#[cfg(test)]
pub use markdown_header::MarkdownHeader;
pub(crate) use markdown_header::MarkdownHeaderInput;
pub use markdown_header::markdown_header;
use std::time::Duration;

pub fn target_header(ui: &mut egui::Ui, label: &str, tooltip: &str) {
    wrapping_path_tooltip(WrappingPathTooltipInput {
        color: ui.visuals().weak_text_color(),
        ui,
        path: label,
        tooltip,
        font: FontId::proportional(11.0),
    });
    ui.separator();
}

/// One row when the path fits; wrap to two rows only when it does not.
pub fn wrapping_path(
    ui: &mut egui::Ui,
    path: &str,
    font: FontId,
    color: Color32,
) -> egui::Response {
    wrapping_path_tooltip(WrappingPathTooltipInput {
        ui,
        path,
        tooltip: path,
        font,
        color,
    })
}

pub(crate) struct WrappingPathTooltipInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) path: &'a str,
    pub(crate) tooltip: &'a str,
    pub(crate) font: FontId,
    pub(crate) color: Color32,
}

pub(super) fn wrapping_path_tooltip(input: WrappingPathTooltipInput<'_>) -> egui::Response {
    let WrappingPathTooltipInput {
        ui,
        path,
        tooltip,
        font,
        color,
    } = input;
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests inspect path and action rects")
    )]
    pub path: egui::Response,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tests inspect path and action rects")
    )]
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
pub(crate) struct FileRowInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub label: &'a str,
    pub icon: &'a str,
    pub selected: bool,
    pub height: f32,
    pub trailing: &'a str,
    pub tint: Color32,
}

pub fn file_row(input: FileRowInput<'_>) -> egui::Response {
    let FileRowInput {
        ui,
        label,
        icon,
        selected,
        height,
        trailing,
        tint,
    } = input;
    ui.scope(|ui| {
        ui.visuals_mut().override_text_color = Some(tint);
        row(RowInput {
            ui,
            label,
            icon,
            selected,
            height,
            trailing,
            tint,
        })
    })
    .inner
}
pub(crate) struct ProjectRowInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub label: &'a str,
    pub icon: &'a str,
    pub selected: bool,
    pub height: f32,
    pub trailing: &'a str,
    pub tint: Color32,
}

pub fn project_row(input: ProjectRowInput<'_>) -> egui::Response {
    let ProjectRowInput {
        ui,
        label,
        icon,
        selected,
        height,
        trailing,
        tint,
    } = input;
    ui.scope(|ui| {
        ui.style_mut()
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        row(RowInput {
            ui,
            label,
            icon,
            selected,
            height,
            trailing,
            tint,
        })
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
pub(crate) struct PaintStatusIconInput<'a> {
    pub ui: &'a egui::Ui,
    pub rect: egui::Rect,
    pub icon: &'a str,
    pub tint: Color32,
    pub spin: bool,
}

pub fn paint_status_icon(input: PaintStatusIconInput<'_>) {
    let PaintStatusIconInput {
        ui,
        rect,
        icon,
        tint,
        spin,
    } = input;
    if spin && !ui.is_rect_visible(rect) {
        return;
    }
    let mut image = egui::Image::new(crate::icons::source(icon));
    // Language icons carry their own palette; tinting them would flatten it.
    if !crate::icons::is_language(icon) {
        image = image.tint(tint);
    }
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
