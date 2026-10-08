use super::bars::*;
use super::controls::*;
use super::rows::*;
use super::theme::*;
use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};
use terminator_core::appearance::AppearanceConfig;

#[cfg(test)]
#[cfg(test)]
mod row_tests {
    use super::*;

    fn i32_from_f32(value: f32) -> i32 {
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }
        if value >= 1_000_000.0 {
            return i32::MAX;
        }
        let mut out = 0_i32;
        let mut cursor = 0.0_f32;
        while cursor + 1.0 <= value {
            cursor += 1.0;
            out = out.saturating_add(1);
        }
        out
    }

    #[test]
    fn font_atlas_side_is_capped_to_4096() {
        let mut input = egui::RawInput {
            max_texture_side: Some(16_384),
            ..Default::default()
        };
        cap_max_texture_side(&mut input);
        assert_eq!(input.max_texture_side, Some(FONT_ATLAS_MAX_SIDE));
        let mut input = egui::RawInput {
            max_texture_side: Some(2048),
            ..Default::default()
        };
        cap_max_texture_side(&mut input);
        assert_eq!(input.max_texture_side, Some(2048));
        let mut missing = egui::RawInput::default();
        cap_max_texture_side(&mut missing);
        assert_eq!(missing.max_texture_side, Some(FONT_ATLAS_MAX_SIDE));
    }

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
                ));
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
                    modifiers: egui::Modifiers::default(),
                }],
            );
            let header = draw_markdown_header(
                &ctx,
                390.0,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
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
    fn clipped_session_row_preserves_height_without_painting_text() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_clip_rect(egui::Rect::NOTHING);
            let response = session_row_spec(
                ui,
                SessionRowSpec {
                    label: "Offscreen agent",
                    icon: "LoaderCircle",
                    selected: true,
                    trailing: "background",
                    tint: Color32::GRAY,
                    icon_tint: None,
                    spin: true,
                    subtitle: Some("Waiting for a response"),
                    brand: Some("Terminal"),
                },
            );
            assert_eq!(response.rect.height(), SESSION_ROW_DETAIL_HEIGHT);
        });
        output.textures_delta.clear();
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| !matches!(shape.shape, egui::Shape::Text(_)))
        );
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
                        terminal_bar(
                            ui,
                            TerminalBarSpec {
                                title: "Terminal",
                                active: false,
                                branch: None,
                                status: None,
                                git_tip: "Open Git",
                                stack_tip: None,
                                vertical_tip: "Split vertically",
                                horizontal_tip: "Split horizontally",
                                brand: None,
                                status_icon: None,
                                status_tint: None,
                                spin: false,
                                kind: None,
                            },
                        );
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
    fn terminal_bar_leading_icons_shift_the_title_right() {
        fn title_left(
            ctx: &egui::Context,
            brand: Option<&str>,
            status_icon: Option<&str>,
            kind: Option<&str>,
        ) -> (f32, usize) {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                terminal_bar(
                    ui,
                    TerminalBarSpec {
                        title: "Terminal",
                        active: false,
                        branch: None,
                        status: None,
                        git_tip: "Open Git",
                        stack_tip: None,
                        vertical_tip: "Split vertically",
                        horizontal_tip: "Split horizontally",
                        brand,
                        status_icon,
                        status_tint: Some(Color32::GREEN),
                        spin: false,
                        kind,
                    },
                );
            });
            output.textures_delta.clear();
            // Icons paint as textured meshes once loaded, or tinted rect
            // placeholders while the loader finishes.
            let meshes = output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape, egui::Shape::Mesh(_))
                        || matches!(
                            &shape.shape,
                            egui::Shape::Rect(rect) if rect.brush.is_some()
                        )
                })
                .count();
            let left = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Terminal" => Some(text.pos.x),
                    _ => None,
                })
                .expect("caption title");
            (left, meshes)
        }
        let ctx = egui::Context::default();
        install(&ctx);
        // Allow the image loader to finish before inspecting painted images.
        title_left(&ctx, None, None, None);
        let (plain, plain_meshes) = title_left(&ctx, None, None, None);
        let (branded, branded_meshes) =
            title_left(&ctx, Some("AgentCodex"), Some("LoaderCircle"), None);
        let (kinded, _) = title_left(&ctx, None, None, Some("Terminal"));
        assert!(
            branded > plain + TERMINAL_LEADING_SLOT,
            "brand + status must move the title right: {plain} vs {branded}"
        );
        assert!(
            branded_meshes > plain_meshes,
            "brand + status must paint extra icons"
        );
        assert!(
            kinded > plain,
            "the kind fallback must move the title right for plain shells"
        );
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
            focus_stroke(accent, std::time::Duration::from_mins(1))
        );
    }
    #[test]
    fn session_subtitle_sits_below_the_label_without_overlap() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            session_row_spec(
                ui,
                SessionRowSpec {
                    label: "Terminal 2",
                    icon: "CircleCheck",
                    selected: true,
                    trailing: "",
                    tint: Color32::GRAY,
                    icon_tint: Some(Color32::BLUE),
                    spin: false,
                    subtitle: Some("Tests pass; ready for review"),
                    brand: None,
                },
            );
        });
        output.textures_delta.clear();
        let texts: Vec<egui::Rect> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if !text.galley.text().is_empty() => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2, "label and subtitle: {texts:?}");
        assert!(
            texts[0].bottom() <= texts[1].top(),
            "label {:?} overlaps subtitle {:?}",
            texts[0],
            texts[1]
        );
        assert_eq!(texts[0].left(), texts[1].left());
    }

    #[test]
    fn session_rows_share_one_icon_column_with_and_without_brand() {
        fn label_left(ctx: &egui::Context, brand: Option<&str>) -> f32 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                session_row_spec(
                    ui,
                    SessionRowSpec {
                        label: "Terminal 2",
                        icon: "Terminal",
                        selected: false,
                        trailing: "background",
                        tint: Color32::GRAY,
                        icon_tint: None,
                        spin: false,
                        subtitle: None,
                        brand,
                    },
                );
            });
            output.textures_delta.clear();
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Terminal 2" => {
                        Some(text.pos.x)
                    }
                    _ => None,
                })
                .expect("session label")
        }
        let ctx = egui::Context::default();
        install(&ctx);
        // Settle fonts/image loaders before measuring.
        label_left(&ctx, None);
        assert_eq!(
            label_left(&ctx, None),
            label_left(&ctx, Some("AgentCodex")),
            "plain and branded session rows must start the label in one column"
        );
    }

    #[test]
    fn running_status_icon_spins_on_a_bounded_interval() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(16.0, 16.0));
        for spin in [false, true] {
            let ctx = egui::Context::default();
            install(&ctx);
            let run = || {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    paint_status_icon(ui, rect, "LoaderCircle", Color32::GREEN, spin);
                });
                output.textures_delta.clear();
                output
            };
            // Settle startup repaints (fonts, image loaders) before checking.
            for _ in 0..3 {
                run();
            }
            let output = run();
            let delay = output
                .viewport_output
                .values()
                .map(|viewport| viewport.repaint_delay)
                .min();
            if spin {
                let delay = delay.expect("working icon schedules a repaint");
                assert!(
                    !delay.is_zero() && delay <= super::STATUS_SPIN_INTERVAL,
                    "working status icon repaint delay {delay:?}"
                );
            } else {
                assert!(
                    delay.is_none_or(|delay| !delay.is_zero()),
                    "idle status icon must not request an immediate repaint"
                );
            }
        }
    }

    #[test]
    fn clipped_status_icon_does_not_schedule_spin() {
        let ctx = egui::Context::default();
        install(&ctx);
        let icon = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(16.0, 16.0));
        let run = |clip: egui::Rect| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_clip_rect(clip);
                paint_status_icon(ui, icon, "LoaderCircle", Color32::GREEN, true);
            });
            output.textures_delta.clear();
            output
                .viewport_output
                .values()
                .map(|viewport| viewport.repaint_delay)
                .min()
        };
        let visible = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(80.0, 80.0));
        for _ in 0..3 {
            let _ = run(visible);
        }
        let scheduled = run(visible).expect("visible working icon schedules a repaint");
        assert!(!scheduled.is_zero() && scheduled <= super::STATUS_SPIN_INTERVAL);
        let hidden = run(egui::Rect::from_min_size(
            egui::pos2(400.0, 400.0),
            egui::vec2(10.0, 10.0),
        ));
        assert!(
            hidden.is_none_or(|delay| delay > super::STATUS_SPIN_INTERVAL),
            "clipped working icon repaint delay {hidden:?}"
        );
    }

    #[test]
    fn unfocused_status_icon_does_not_schedule_spin() {
        let ctx = egui::Context::default();
        install(&ctx);
        let icon = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(16.0, 16.0));
        let run = |focused: Option<bool>| {
            let mut input = egui::RawInput::default();
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .expect("root viewport")
                .focused = focused;
            let mut output = ctx.run_ui(input, |ui| {
                paint_status_icon(ui, icon, "LoaderCircle", Color32::GREEN, true);
            });
            output.textures_delta.clear();
            output
                .viewport_output
                .values()
                .map(|viewport| viewport.repaint_delay)
                .min()
        };
        for _ in 0..3 {
            let _ = run(None);
        }
        let scheduled = run(None).expect("unknown focus still schedules a repaint");
        assert!(!scheduled.is_zero() && scheduled <= super::STATUS_SPIN_INTERVAL);
        let hidden = run(Some(false));
        assert!(
            hidden.is_none_or(|delay| delay > super::STATUS_SPIN_INTERVAL),
            "unfocused working icon repaint delay {hidden:?}"
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
                    modifiers: egui::Modifiers::default(),
                }],
            );
            assert!(
                draw(
                    &ctx,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default()
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
            i32_from_f32((response.rect.height() / line_height(ui, font)).round())
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
            i32_from_f32((response.rect.height() / line_height(ui, font)).round())
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
                modifiers: egui::Modifiers::default(),
            }],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
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
