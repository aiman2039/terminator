use eframe::egui::{self, Color32, FontFamily, FontId, RichText, TextStyle};
use terminator_core::appearance::{AppearanceConfig, rgb};

/// Keep small navigation/action glyphs legible independently of secondary text.
pub const ICON_COLOR: Color32 = Color32::from_rgb(242, 244, 248);

/// Orca dark context menu fill: `--background` `#0a0a0a`, used as the solid
/// stand-in for `dark:bg-[rgba(0,0,0,0.12)]` + `backdrop-blur-2xl`.
pub const MENU_FILL: Color32 = Color32::from_rgb(10, 10, 10);
/// Orca `--popover-foreground` `#fafafa`.
pub const MENU_TEXT: Color32 = Color32::from_rgb(250, 250, 250);
/// Orca `--muted-foreground` `#a1a1a1` (shortcuts use this at 85% in CSS).
const MENU_MUTED: Color32 = Color32::from_rgb(161, 161, 161);
/// Orca `dark:border-white/14` and `dark:focus:bg-white/14`.
const MENU_LINE: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);

pub fn color(value: &str) -> Color32 {
    let [r, g, b] = rgb(value).unwrap_or([209, 211, 217]);
    Color32::from_rgb(r, g, b)
}
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes, family) in [
        (
            "Inter",
            include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice(),
            FontFamily::Proportional,
        ),
        (
            "Inter Semibold",
            include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice(),
            FontFamily::Name("Semibold".into()),
        ),
        (
            "JetBrains Mono",
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
            FontFamily::Monospace,
        ),
        (
            "JetBrains Mono Bold",
            include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf").as_slice(),
            FontFamily::Name("Terminal Bold".into()),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, name.into());
    }
    fonts.font_data.insert(
        "Noto Sans Symbols 2 Braille".into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/NotoSansSymbols2-Braille.ttf"
        ))
        .into(),
    );
    for family in [
        FontFamily::Monospace,
        FontFamily::Name("Terminal Bold".into()),
    ] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("Noto Sans Symbols 2 Braille".into());
    }
    fonts.font_data.insert(
        "Noto Sans Hebrew".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/NotoSansHebrew-Regular.ttf"))
            .into(),
    );
    for family in [
        FontFamily::Monospace,
        FontFamily::Name("Terminal Bold".into()),
        FontFamily::Proportional,
        FontFamily::Name("Semibold".into()),
    ] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("Noto Sans Hebrew".into());
    }
    ctx.set_fonts(fonts);
    egui_extras::install_image_loaders(ctx);
    apply(ctx, &AppearanceConfig::default());
}
pub fn apply(ctx: &egui::Context, theme: &AppearanceConfig) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    for role in [TextStyle::Body, TextStyle::Button] {
        style.text_styles.insert(role, FontId::proportional(13.0));
    }
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(16.0, FontFamily::Name("Semibold".into())),
    );
    style.text_styles.insert(
        TextStyle::Name("Section".into()),
        FontId::new(13.0, FontFamily::Name("Semibold".into())),
    );
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));
    style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style.visuals.panel_fill = color(&theme.surface);
    style.visuals.window_fill = color(&theme.window);
    style.visuals.extreme_bg_color = color(&theme.surface);
    style.visuals.faint_bg_color = color(&theme.hover);
    style.visuals.override_text_color = Some(color(&theme.text));
    style.visuals.error_fg_color = color(&theme.status_failed);
    style.visuals.weak_text_color = Some(color(&theme.secondary));
    style.visuals.selection.bg_fill = color(&theme.selection);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, color(&theme.accent));
    style.visuals.window_stroke = egui::Stroke::new(theme.border_width, color(&theme.border));
    style.visuals.window_corner_radius = egui::CornerRadius::same(3);
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.bg_fill = color(&theme.surface);
        widget.weak_bg_fill = color(&theme.surface);
        widget.bg_stroke = egui::Stroke::new(theme.border_width, color(&theme.border));
        widget.fg_stroke = egui::Stroke::new(1.0, color(&theme.text));
        widget.corner_radius = egui::CornerRadius::same(3);
    }
    style.visuals.widgets.hovered.bg_fill = color(&theme.hover);
    style.visuals.widgets.hovered.weak_bg_fill = color(&theme.hover);
    style.visuals.widgets.active.bg_stroke.color = color(&theme.accent);
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
    style.visuals.indent_has_left_vline = false;
    style.visuals.menu_corner_radius = egui::CornerRadius::same(4);
    // Shared by every app-owned ScrollArea: overlay handles stay hidden at rest.
    style.spacing.scroll = egui::style::ScrollStyle {
        floating: true,
        bar_width: 8.0,
        floating_width: 4.0,
        floating_allocated_width: 0.0,
        handle_min_length: 28.0,
        bar_inner_margin: 3.0,
        bar_outer_margin: 2.0,
        foreground_color: true,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.0,
        interact_handle_opacity: 0.0,
        ..Default::default()
    };
    let compact = theme.compact();
    style.spacing.button_padding = if compact {
        egui::vec2(6.0, 4.0)
    } else {
        egui::vec2(8.0, 5.0)
    };
    style.spacing.text_edit_width = 220.0;
    style.spacing.indent = 14.0;
    style.spacing.item_spacing = if compact {
        egui::vec2(6.0, 6.0)
    } else {
        egui::vec2(8.0, 8.0)
    };
    style.spacing.interact_size.y = if compact { 24.0 } else { 28.0 };
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}

pub fn sidebar_action(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    ui.add_sized(
        [22.0, 22.0],
        egui::Button::image(
            egui::Image::new(crate::icons::source(icon))
                .tint(ICON_COLOR)
                .fit_to_exact_size(egui::vec2(14.0, 14.0)),
        )
        .frame(false),
    )
    .on_hover_text(tip)
}

/// Sidebar lists stay wheel/trackpad-scrollable without a visible bar.
pub fn sidebar_scroll(salt: &'static str) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
}

pub fn tool_button(
    ui: &mut egui::Ui,
    tool: crate::preferences::SidebarTool,
    label: &str,
    active: bool,
) -> egui::Response {
    use crate::preferences::SidebarTool;
    let response = ui
        .add_sized([36.0, 32.0], egui::Button::new("").frame(false))
        .on_hover_text(label);
    if response.hovered() {
        ui.painter()
            .rect_filled(response.rect, 2, ui.visuals().widgets.hovered.bg_fill);
    }
    let tint = ICON_COLOR;
    let icon = match tool {
        SidebarTool::Explorer => "Files",
        SidebarTool::Agents => "PanelsTopLeft",
        SidebarTool::Git => "GitBranch",
        SidebarTool::History => "History",
    };
    egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .paint_at(
            ui,
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(16.0, 16.0)),
        );
    if active {
        ui.painter().line_segment(
            [
                response.rect.left_bottom() + egui::vec2(8.0, -1.0),
                response.rect.right_bottom() + egui::vec2(-8.0, -1.0),
            ],
            egui::Stroke::new(2.0, tint),
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, label)
    });
    response
}
/// Orca dark menu recipe from `ui/context-menu.tsx` and `.dark` tokens in `main.css`.
pub fn menu_style(style: &mut egui::Style) {
    egui::menu::menu_style(style);
    for role in [TextStyle::Body, TextStyle::Button] {
        style.text_styles.insert(role, FontId::proportional(12.0));
    }
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style.spacing.menu_margin = egui::Margin::same(4);
    style.visuals.window_fill = MENU_FILL;
    style.visuals.override_text_color = Some(MENU_TEXT);
    style.visuals.weak_text_color = Some(MENU_MUTED);
    style.visuals.window_stroke = egui::Stroke::new(1.0, MENU_LINE);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(11);
    style.visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 16],
        blur: 34,
        spread: 0,
        color: Color32::from_black_alpha(102),
    };
    style.visuals.widgets.hovered.bg_fill = MENU_LINE;
    style.visuals.widgets.hovered.weak_bg_fill = MENU_LINE;
    style.visuals.widgets.open.bg_fill = MENU_LINE;
    style.visuals.widgets.open.weak_bg_fill = MENU_LINE;
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.fg_stroke = egui::Stroke::new(1.0, MENU_TEXT);
    }
}

pub fn context_menu(
    response: &egui::Response,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> Option<egui::InnerResponse<()>> {
    egui::Popup::context_menu(response)
        .style(menu_style)
        .show(add_contents)
}

pub fn menu_button<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let config = egui::menu::MenuConfig::new().style(menu_style);
    let (response, inner) = if egui::menu::is_in_menu(ui) {
        egui::menu::SubMenuButton::new(title).ui(ui, add_contents)
    } else {
        egui::menu::MenuButton::new(title)
            .config(config)
            .ui(ui, add_contents)
    };
    egui::InnerResponse::new(inner.map(|shown| shown.inner), response)
}

/// Flat full-width action row with a fixed icon column and optional shortcut.
pub fn menu_item(ui: &mut egui::Ui, label: &str, icon: &str, shortcut: &str) -> egui::Response {
    ui.set_min_width(220.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    let destructive =
        label.starts_with("Close ") || label.starts_with("Remove ") || label.starts_with("Clear ");
    let previous = ui.visuals().override_text_color;
    let tint = if destructive {
        ui.visuals().error_fg_color
    } else {
        ui.visuals().weak_text_color()
    };
    if destructive {
        ui.visuals_mut().override_text_color = Some(tint);
    }
    let response = row(ui, label, icon, false, 26.0, shortcut, tint);
    ui.visuals_mut().override_text_color = previous;
    #[cfg(feature = "test-support")]
    crate::diagnostics::record(ui.ctx(), label, response.rect);
    response
}
pub fn target_header(ui: &mut egui::Ui, target: &str) {
    wrapping_path(
        ui,
        target,
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
    let natural = ui
        .painter()
        .layout_no_wrap(path.to_owned(), font.clone(), color)
        .size()
        .x;
    let avail = ui.available_width().max(0.0);
    let rows = usize::from(natural > avail) + 1;
    let mut job = egui::text::LayoutJob::simple(path.to_owned(), font, color, avail);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter()
        .with_clip_rect(rect)
        .galley(rect.min, galley, color);
    response.on_hover_text(path)
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
pub fn session_row(
    ui: &mut egui::Ui,
    label: &str,
    icon: &str,
    selected: bool,
    height: f32,
    trailing: &str,
    tint: Color32,
) -> egui::Response {
    ui.scope(|ui| {
        if !selected {
            ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
        }
        row(ui, label, icon, selected, height, trailing, tint)
    })
    .inner
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
    let response = ui.add_sized(
        [ui.available_width(), height],
        egui::Button::new("").frame(false),
    );
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
    if trailing == "●" {
        ui.painter().circle_filled(
            egui::pos2(response.rect.right() - 10.0, response.rect.center().y),
            3.5,
            tint,
        );
    }
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(response.rect.left() + 12.0, response.rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    egui::Image::new(crate::icons::source(icon))
        .tint(ICON_COLOR)
        .paint_at(ui, icon_rect);
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
        egui::pos2(response.rect.left() + 26.0, response.rect.top()),
        egui::pos2(
            right.max(response.rect.left() + 26.0),
            response.rect.bottom(),
        ),
    );
    // Paint the label rather than overlaying a selectable Label widget: the row
    // must own clicks on its text as well as its icon and empty space.
    let mut job = egui::text::LayoutJob::simple(
        label.to_owned(),
        TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
        rect.width(),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let position = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(rect)
        .galley(position, galley, ui.visuals().text_color());
    if trailing != "●" {
        ui.painter().text(
            egui::pos2(response.rect.right() - 5.0, response.rect.center().y),
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
        .interact(title_rect, row.id.with("title"), egui::Sense::click())
        .on_hover_text(title)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
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
    let fill = Color32::from_rgb(warn.r() / 2 + 24, warn.g() / 6, warn.b() / 6);
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

/// A compact caption inside a pane border, without tab or split controls.
pub fn pane_caption(
    ui: &mut egui::Ui,
    title: &str,
    active: bool,
    closeable: bool,
) -> (egui::Response, Option<egui::Response>) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
    let tint = if active {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().weak_text_color()
    };
    let mut job = egui::text::LayoutJob::simple(
        title.into(),
        FontId::proportional(12.0),
        tint,
        (rect.width() - if closeable { 40.0 } else { 16.0 }).max(0.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let position = egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(rect)
        .galley(position, galley, tint);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            active,
            title,
        )
    });
    let close = closeable.then(|| {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 10.0, rect.center().y),
            egui::vec2(18.0, 18.0),
        );
        let response = ui
            .interact(
                close_rect,
                response.id.with("close-pane"),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Close pane");
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Close pane")
        });
        if response.hovered() {
            ui.painter()
                .rect_filled(close_rect, 2, ui.visuals().widgets.hovered.bg_fill);
        }
        egui::Image::new(crate::icons::source("X"))
            .tint(if response.hovered() {
                ui.visuals().error_fg_color
            } else {
                ICON_COLOR
            })
            .paint_at(
                ui,
                egui::Rect::from_center_size(close_rect.center(), egui::vec2(12.0, 12.0)),
            );
        response
    });
    (
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(title),
        close,
    )
}

/// Brief focus emphasis, fading to a thin, translucent steady-state outline.
pub fn focus_stroke(accent: Color32, elapsed: std::time::Duration) -> egui::Stroke {
    let progress = ((elapsed.as_secs_f32() - 0.2) / 1.0).clamp(0.0, 1.0);
    let strength = (1.0 - progress).powi(2);
    let alpha = (70.0 + 185.0 * strength).round() as u8;
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

#[cfg(test)]
mod row_tests {
    use super::*;

    #[test]
    fn app_and_terminal_fonts_cover_hebrew_letters() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx.fonts_mut(|fonts| {
            for family in [
                FontFamily::Monospace,
                FontFamily::Name("Terminal Bold".into()),
                FontFamily::Proportional,
                FontFamily::Name("Semibold".into()),
            ] {
                let font_id = FontId::new(13.0, family);
                for c in '\u{05D0}'..='\u{05EA}' {
                    assert!(fonts.has_glyph(&font_id, c), "{font_id:?} missing {c}");
                }
            }
        });
    }

    #[test]
    fn terminal_fonts_cover_braille_block() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx.fonts_mut(|fonts| {
            for family in [
                FontFamily::Monospace,
                FontFamily::Name("Terminal Bold".into()),
            ] {
                let font_id = FontId::new(13.0, family);
                for c in '\u{2800}'..='\u{28FF}' {
                    assert!(
                        fonts.has_glyph(&font_id, c),
                        "missing U+{:04X}",
                        u32::from(c)
                    );
                }
            }
        });
    }

    fn draw_markdown_header(
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> MarkdownHeader {
        let mut header = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 100.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                header = Some(markdown_header(
                    ui,
                    "a-long-markdown-file-name.md",
                    true,
                    false,
                    crate::markdown::Mode::Preview,
                ))
            },
        );
        output.textures_delta.clear();
        header.unwrap()
    }

    #[test]
    fn markdown_filename_tabs_and_icons_share_one_row_without_overlapping() {
        for width in [250.0, 390.0, 900.0] {
            let ctx = egui::Context::default();
            install(&ctx);
            let header = draw_markdown_header(&ctx, width, vec![]);
            let mut rects = vec![header.title.rect];
            rects.extend(header.modes.iter().map(|(_, response)| response.rect));
            rects.extend([header.refresh.rect, header.close.rect]);
            assert!(header.title.rect.width() >= 30.0);
            for pair in rects.windows(2) {
                assert!((pair[0].center().y - pair[1].center().y).abs() < 0.1);
                assert!(
                    pair[0].right() <= pair[1].left() + 0.1,
                    "width={width}: {pair:?}"
                );
            }
            assert!(header.close.rect.right() <= width);
        }
    }

    #[test]
    fn markdown_tab_refresh_and_close_clicks_do_not_hit_the_filename() {
        for index in 0..5 {
            let ctx = egui::Context::default();
            install(&ctx);
            let header = draw_markdown_header(&ctx, 390.0, vec![]);
            let rect = match index {
                0..=2 => header.modes[index].1.rect,
                3 => header.refresh.rect,
                _ => header.close.rect,
            };
            let pos = rect.center();
            draw_markdown_header(&ctx, 390.0, vec![egui::Event::PointerMoved(pos)]);
            draw_markdown_header(
                &ctx,
                390.0,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            let header = draw_markdown_header(
                &ctx,
                390.0,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            assert!(!header.title.clicked());
            assert_eq!(header.close.clicked(), index == 4);
            assert_eq!(header.refresh.clicked(), index == 3);
            for (i, (_, response)) in header.modes.iter().enumerate() {
                assert_eq!(response.clicked(), index == i);
            }
        }
    }
    fn draw(ctx: &egui::Context, events: Vec<egui::Event>) -> egui::Response {
        let mut response = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(260.0, 80.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            response = Some(row(
                ui,
                "Terminal 1",
                "Terminal",
                false,
                24.0,
                "",
                Color32::GRAY,
            ));
        });
        output.textures_delta.clear();
        response.unwrap()
    }
    #[test]
    fn navigation_row_and_close_icons_render_bright_without_hover() {
        for surface in ["tool", "row", "close"] {
            let ctx = egui::Context::default();
            install(&ctx);
            let draw = || {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| match surface {
                    "tool" => {
                        tool_button(ui, crate::preferences::SidebarTool::Git, "Git", false);
                    }
                    "row" => {
                        row(ui, "File", "FileCode", false, 24.0, "", Color32::GRAY);
                    }
                    _ => {
                        pane_caption(ui, "Terminal", false, true);
                    }
                });
                output.textures_delta.clear();
                output
            };
            // Allow the image loader to finish before inspecting painted images.
            draw();
            let output = draw();
            let icons = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.brush.is_some() => Some(rect),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(!icons.is_empty(), "Missing rendered {surface} icon");
            assert!(
                icons.iter().all(|icon| icon.fill == ICON_COLOR),
                "The {surface} icon must remain bright even when its text is muted"
            );
        }
    }

    #[test]
    fn focus_border_fades_and_stays_soft() {
        let accent = Color32::from_rgb(56, 113, 225);
        let strong = focus_stroke(accent, std::time::Duration::ZERO);
        let fading = focus_stroke(accent, std::time::Duration::from_millis(700));
        let steady = focus_stroke(accent, std::time::Duration::from_secs(2));
        assert_eq!(strong.width, 2.0);
        assert_eq!(strong.color.a(), 255);
        assert!(fading.width < strong.width && fading.width > steady.width);
        assert!(fading.color.a() < strong.color.a() && fading.color.a() > steady.color.a());
        assert_eq!(steady.width, 1.0);
        assert_eq!(steady.color.a(), 70);
        assert_eq!(
            steady,
            focus_stroke(accent, std::time::Duration::from_secs(60))
        );
    }
    #[test]
    fn row_click_works_on_icon_label_and_empty_space() {
        for x in [12.0, 55.0, 230.0] {
            let ctx = egui::Context::default();
            install(&ctx);
            let rect = draw(&ctx, vec![]).rect;
            let pos = egui::pos2(rect.left() + x, rect.center().y);
            draw(&ctx, vec![egui::Event::PointerMoved(pos)]);
            draw(
                &ctx,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            assert!(
                draw(
                    &ctx,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default()
                    }]
                )
                .clicked(),
                "Row should receive the click at x={x}"
            );
        }
    }

    #[test]
    fn unsaved_close_actions_fit_narrow_panes_and_long_errors() {
        for width in [170.0, 220.0, 320.0, 640.0] {
            for message in [
                "Unsaved changes",
                "Editor did not close. Check for unsaved buffers or running editor jobs.",
            ] {
                let ctx = egui::Context::default();
                install(&ctx);
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 500.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let bounds = ui.max_rect();
                        let bar = unsaved_close_bar(
                            ui,
                            UnsavedCloseBar {
                                theme: &AppearanceConfig::default(),
                                message,
                                enabled: true,
                            },
                        );
                        for response in [&bar.save, &bar.discard, &bar.cancel] {
                            assert!(
                                bounds.contains_rect(response.rect),
                                "width {width}: {:?} outside {bounds:?}",
                                response.rect
                            );
                        }
                        assert!(!bar.save.rect.intersects(bar.discard.rect));
                        assert!(!bar.discard.rect.intersects(bar.cancel.rect));
                    },
                );
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn unsaved_close_bar_keeps_actions_on_one_row() {
        let ctx = egui::Context::default();
        install(&ctx);
        let theme = AppearanceConfig::default();
        let mut bar = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 80.0),
                )),
                ..Default::default()
            },
            |ui| {
                bar = Some(unsaved_close_bar(
                    ui,
                    UnsavedCloseBar {
                        theme: &theme,
                        message: "Unsaved changes",
                        enabled: true,
                    },
                ));
            },
        );
        output.textures_delta.clear();
        let bar = bar.expect("bar");
        assert!(bar.save.rect.width() > 8.0);
        assert!(bar.discard.rect.width() > 8.0);
        assert!(bar.cancel.rect.width() > 8.0);
        assert!(!bar.save.rect.intersects(bar.discard.rect));
        assert_eq!(bar.choice(), None);
    }

    const PASTE_PATH: &str =
        "/var/folders/0r/9s1qpwv16qd5km4kkkjr11w80000gn/T/terminator-paste-TrXSrw.png";

    fn run_at_width<T>(width: f32, mut add: impl FnMut(&mut egui::Ui) -> T) -> T {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut value = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 200.0),
                )),
                ..Default::default()
            },
            |ui| {
                value = Some(add(ui));
            },
        );
        output.textures_delta.clear();
        value.expect("ui ran")
    }

    fn line_height(ui: &egui::Ui, font: FontId) -> f32 {
        ui.painter()
            .layout_no_wrap("X".into(), font, ui.visuals().text_color())
            .size()
            .y
    }

    fn wrapping_path_lines(width: f32) -> i32 {
        run_at_width(width, |ui| {
            let font = FontId::proportional(12.0);
            let response = wrapping_path(ui, PASTE_PATH, font.clone(), ui.visuals().text_color());
            (response.rect.height() / line_height(ui, font)).round() as i32
        })
    }

    fn image_toolbar(width: f32) -> (egui::Rect, Vec<egui::Rect>) {
        run_at_width(width, |ui| {
            let row = wrapping_path_row(ui, PASTE_PATH, |ui| {
                [
                    ui.button("Reload"),
                    ui.button("Open as text"),
                    ui.button("Open externally"),
                ]
                .map(|response| response.rect)
            });
            (row.path.rect, row.actions.to_vec())
        })
    }

    #[test]
    fn wrapping_path_stays_one_row_when_it_fits() {
        assert_eq!(wrapping_path_lines(720.0), 1);
    }

    #[test]
    fn wrapping_path_uses_two_rows_when_narrow() {
        assert_eq!(wrapping_path_lines(140.0), 2);
    }

    #[test]
    fn image_toolbar_keeps_path_and_actions_on_one_row_when_wide() {
        for width in [1000.0, 1200.0] {
            let (path, buttons) = image_toolbar(width);
            assert_eq!(buttons.len(), 3, "width={width}");
            let mut rects = vec![path];
            rects.extend(buttons);
            for pair in rects.windows(2) {
                assert!(
                    (pair[0].center().y - pair[1].center().y).abs() < 0.1,
                    "width={width}: {pair:?}"
                );
                assert!(
                    pair[0].right() <= pair[1].left() + 0.1,
                    "width={width}: {pair:?}"
                );
            }
        }
    }

    #[test]
    fn image_toolbar_wraps_actions_when_narrow() {
        let (path, buttons) = image_toolbar(180.0);
        assert!(
            buttons
                .iter()
                .any(|button| button.center().y - path.center().y > path.height() * 0.4),
            "narrow toolbar should wrap actions below the path: path={path:?} buttons={buttons:?}"
        );
    }

    #[test]
    fn hover_path_does_not_exceed_two_rows() {
        let lines = run_at_width(440.0, |ui| {
            let font = FontId::proportional(11.0);
            let response =
                wrapping_path(ui, PASTE_PATH, font.clone(), ui.visuals().weak_text_color());
            (response.rect.height() / line_height(ui, font)).round() as i32
        });
        assert!(
            (1..=2).contains(&lines),
            "hover path should be one or two rows, lines={lines}"
        );
    }

    #[test]
    fn menu_style_matches_orca_dark_tokens() {
        let mut style = egui::Style {
            visuals: egui::Visuals::dark(),
            ..Default::default()
        };
        menu_style(&mut style);
        assert_eq!(style.visuals.window_fill, MENU_FILL);
        assert_eq!(style.visuals.override_text_color, Some(MENU_TEXT));
        assert_eq!(style.visuals.weak_text_color, Some(MENU_MUTED));
        assert_eq!(style.visuals.window_stroke.color, MENU_LINE);
        assert_eq!(
            style.visuals.menu_corner_radius,
            egui::CornerRadius::same(11)
        );
        assert_eq!(style.visuals.widgets.hovered.bg_fill, MENU_LINE);
        assert_eq!(
            style.text_styles.get(&TextStyle::Body),
            Some(&FontId::proportional(12.0))
        );
    }

    #[test]
    fn context_menu_paints_orca_fill_and_text() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut fill = None;
        let mut text = None;
        let pos = egui::pos2(40.0, 20.0);
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(pos)],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: Default::default(),
            }],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: Default::default(),
            }],
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = ui.button("Terminal");
                    context_menu(&response, |ui| {
                        fill = Some(ui.visuals().window_fill);
                        text = ui.visuals().override_text_color;
                        menu_item(ui, "Copy", "Copy", "⌘C");
                    });
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(fill, Some(MENU_FILL));
        assert_eq!(text, Some(MENU_TEXT));
    }
}
