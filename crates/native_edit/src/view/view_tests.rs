use super::super::doc::Buffer;
use super::super::highlight::{HighlightCache, Language};
use super::super::vim::{Effect, Key as VKey, ModalEngine, Mode, VimEngine};
use super::keys::{
    IdeAction, IdeEdit, command_prompt, ctrl_h_is_motion, ctrl_v_is_block, echo_of_handled_key,
    egui_key, ide_edit_key, ide_shortcut, mode_name, source_focus_id,
};
use super::render::{SourceOptions, show_source};
use super::{char_display_width, col_at_display_x, display_x};
use egui;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_source_focus_is_registered_and_outside_pointer_press_does_not_edit() {
        let ctx = egui::Context::default();
        let mut doc = crate::doc::Doc::new("source");
        let mut engine = VimEngine::new();
        let mut highlight = HighlightCache::default();
        for frame in 0..2 {
            let events = if frame == 1 {
                vec![
                    egui::Event::PointerMoved(egui::pos2(700.0, 150.0)),
                    egui::Event::PointerButton {
                        pos: egui::pos2(700.0, 150.0),
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Text("outside input".into()),
                ]
            } else {
                vec![]
            };
            let mut output = ctx.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0))),
                events, ..Default::default()
            }, |ui| {
                ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0))), |ui| {
                    show_source(ui, &mut doc, &mut engine, &SourceOptions { vim: false, ..Default::default() }, &mut highlight, Language::Rust, "file.rs#project.tab.split");
                });
                assert!(crate::view::source_has_focus(ui.ctx()));
                if frame == 1 {
                    assert!(ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Text(text) if text == "outside input"))));
                }
            });
            output.textures_delta.clear();
        }
        assert_eq!(doc.text(), "source");
    }

    #[test]
    fn source_keyboard_focus_has_an_accessibility_node() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut doc = crate::doc::Doc::new("private file contents");
        let mut engine = VimEngine::new();
        let mut highlight = HighlightCache::default();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show_source(
                    ui,
                    &mut doc,
                    &mut engine,
                    &SourceOptions::default(),
                    &mut highlight,
                    Language::Plain,
                    "accessibility-editor",
                );
            });
            output.textures_delta.clear();
            let update = output.platform_output.accesskit_update.unwrap();
            assert_ne!(update.focus, update.tree.as_ref().unwrap().root);
            assert!(update.nodes.iter().any(|(id, _)| *id == update.focus));
            assert!(!format!("{:?}", update.nodes).contains("private file contents"));
        }
    }

    fn plain(key: egui::Key) -> Option<VKey> {
        egui_key(key, egui::Modifiers::NONE)
    }

    #[test]
    fn command_key_reaches_colon() {
        // `:` must stay reachable: ex commands live behind it.
        assert_eq!(plain(egui::Key::Colon), Some(VKey::Char(':')));
    }

    #[test]
    fn letters_follow_shift_case() {
        assert_eq!(plain(egui::Key::A), Some(VKey::Char('a')));
        assert_eq!(
            egui_key(egui::Key::A, egui::Modifiers::SHIFT),
            Some(VKey::Char('A'))
        );
    }

    #[test]
    fn shifted_digits_are_symbols_not_counts() {
        // `$` is a motion; it must not arrive as the count digit `4`.
        assert_eq!(
            egui_key(egui::Key::Num4, egui::Modifiers::SHIFT),
            Some(VKey::Char('$'))
        );
        assert_eq!(plain(egui::Key::Num4), Some(VKey::Char('4')));
    }

    #[test]
    fn command_held_keys_stay_with_the_app() {
        assert_eq!(egui_key(egui::Key::F, egui::Modifiers::COMMAND), None);
    }

    #[test]
    fn symbol_keys_pass_through() {
        // Backends may report `!` directly instead of shifted `1`.
        assert_eq!(plain(egui::Key::Exclamationmark), Some(VKey::Char('!')));
    }

    #[test]
    fn arrows_and_escape_map() {
        assert_eq!(plain(egui::Key::ArrowLeft), Some(VKey::Left));
        assert_eq!(plain(egui::Key::Escape), Some(VKey::Escape));
        assert_eq!(plain(egui::Key::F5), None);
    }

    #[test]
    fn ide_shortcuts_need_command_held() {
        assert_eq!(
            ide_shortcut(egui::Key::C, egui::Modifiers::COMMAND),
            Some(IdeAction::Copy)
        );
        assert_eq!(
            ide_shortcut(egui::Key::X, egui::Modifiers::COMMAND),
            Some(IdeAction::Cut)
        );
        assert_eq!(
            ide_shortcut(egui::Key::A, egui::Modifiers::COMMAND),
            Some(IdeAction::SelectAll)
        );
        assert_eq!(
            ide_shortcut(egui::Key::V, egui::Modifiers::COMMAND),
            Some(IdeAction::PasteKey)
        );
        assert_eq!(
            ide_shortcut(egui::Key::Z, egui::Modifiers::COMMAND),
            Some(IdeAction::Undo)
        );
        let shift_command = egui::Modifiers {
            shift: true,
            ..egui::Modifiers::COMMAND
        };
        assert_eq!(
            ide_shortcut(egui::Key::Z, shift_command),
            Some(IdeAction::Redo)
        );
        // Plain keys stay with the vim engine; bare Ctrl (macOS) and Alt
        // chords stay with the terminal/app layer. Ctrl+C on Linux arrives
        // with `command` set, so it still resolves.
        assert_eq!(ide_shortcut(egui::Key::C, egui::Modifiers::NONE), None);
        assert_eq!(
            ide_shortcut(
                egui::Key::C,
                egui::Modifiers {
                    ctrl: true,
                    ..egui::Modifiers::NONE
                }
            ),
            None
        );
        assert_eq!(
            ide_shortcut(
                egui::Key::C,
                egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..egui::Modifiers::NONE
                }
            ),
            Some(IdeAction::Copy)
        );
        assert_eq!(
            ide_shortcut(
                egui::Key::C,
                egui::Modifiers {
                    command: true,
                    alt: true,
                    ..egui::Modifiers::NONE
                }
            ),
            None
        );
        // Ctrl+F/S/H are IDE actions now (find/save/replace); the ex
        // search prompt and `%s/` handling live behind them.
        assert_eq!(
            ide_shortcut(egui::Key::F, egui::Modifiers::COMMAND),
            Some(IdeAction::Find)
        );
        assert_eq!(
            ide_shortcut(egui::Key::S, egui::Modifiers::COMMAND),
            Some(IdeAction::Save)
        );
        assert_eq!(
            ide_shortcut(egui::Key::H, egui::Modifiers::COMMAND),
            Some(IdeAction::Replace)
        );
    }

    #[test]
    fn focus_id_is_stable_per_file() {
        assert_eq!(source_focus_id("a"), source_focus_id("a"));
        assert_ne!(source_focus_id("a"), source_focus_id("b"));
    }

    #[test]
    fn rust_view_derives_highlight_once() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("fn main() {}\n");
        let mut engine = VimEngine::new();
        let mut highlight = super::HighlightCache::default();
        // Escape is a no-op in Normal mode: it only pumps frames.
        let idle = [(egui::Key::Escape, egui::Modifiers::NONE)];
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &super::SourceOptions::default(),
            super::Language::Rust,
            &mut highlight,
            &idle,
        );
        assert!(!highlight.line(0).is_empty());
        // A second frame with no edits reuses the cache.
        let revision = doc.revision();
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &super::SourceOptions::default(),
            super::Language::Rust,
            &mut highlight,
            &idle,
        );
        assert!(highlight.is_current(super::Language::Rust, true, revision));
    }

    #[test]
    fn command_footer_stays_at_pane_bottom_for_short_and_long_files() {
        use crate::doc::Doc;
        fn footer_y(ctx: &egui::Context, text: &str) -> f32 {
            let mut doc = Doc::new(text);
            let mut engine = VimEngine::new();
            engine.begin_search();
            let mut highlight = HighlightCache::default();
            let mut footer = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let result = show_source(
                        ui,
                        &mut doc,
                        &mut engine,
                        &SourceOptions::default(),
                        &mut highlight,
                        Language::Plain,
                        "footer-test",
                    );
                    footer = result.footer_rect.expect("footer").top();
                });
            });
            output.textures_delta.clear();
            footer
        }
        let ctx = egui::Context::default();
        let short = footer_y(&ctx, "one line");
        let long = footer_y(&ctx, &"many lines\n".repeat(200));
        assert!((short - long).abs() < 1.0, "short={short}, long={long}");
    }

    #[test]
    fn right_click_in_blank_editor_space_opens_context_menu() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("short file");
        let mut engine = VimEngine::new();
        let mut highlight = HighlightCache::default();
        let draw = |input: egui::RawInput,
                    doc: &mut Doc,
                    engine: &mut VimEngine,
                    highlight: &mut HighlightCache| {
            let mut outcome = None;
            let mut output = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    outcome = Some(show_source(
                        ui,
                        doc,
                        engine,
                        &SourceOptions::default(),
                        highlight,
                        Language::Plain,
                        "context-menu-test",
                    ));
                });
            });
            output.textures_delta.clear();
            outcome.expect("source outcome")
        };
        let first = draw(
            egui::RawInput::default(),
            &mut doc,
            &mut engine,
            &mut highlight,
        );
        let pos = first.content_rect.left_top() + egui::vec2(100.0, 100.0);
        assert!(first.content_rect.contains(pos));
        let mut press = egui::RawInput::default();
        press.events.push(egui::Event::PointerMoved(pos));
        press.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        draw(press, &mut doc, &mut engine, &mut highlight);
        let mut release = egui::RawInput::default();
        release.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let result = draw(release, &mut doc, &mut engine, &mut highlight);
        assert!(result.context_menu_opened);
    }

    #[test]
    fn read_only_rename_window_keeps_buffer_text() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("before");
        let mut engine = VimEngine::new();
        let options = SourceOptions {
            read_only: true,
            ..SourceOptions::default()
        };
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            Language::Plain,
            &mut HighlightCache::default(),
            &[
                (egui::Key::I, egui::Modifiers::NONE),
                (egui::Key::A, egui::Modifiers::NONE),
            ],
        );
        assert_eq!(doc.text(), "before");
    }

    #[test]
    fn wide_chars_take_two_display_cells() {
        assert_eq!(char_display_width('a'), 1);
        assert_eq!(char_display_width('あ'), 2);
        assert_eq!(display_x("aあb", 0), 0);
        assert_eq!(display_x("aあb", 1), 1);
        assert_eq!(display_x("aあb", 2), 3);
        assert_eq!(display_x("aあb", 3), 4);
        // Floor: the second cell of a wide char resolves to that char.
        assert_eq!(col_at_display_x("aあb", 0), 0);
        assert_eq!(col_at_display_x("aあb", 1), 1);
        assert_eq!(col_at_display_x("aあb", 2), 1);
        assert_eq!(col_at_display_x("aあb", 3), 2);
        assert_eq!(col_at_display_x("aあb", 99), 3);
    }

    #[test]
    fn handled_key_echo_is_dropped_once() {
        // The `i` that opened Insert mode must not type itself.
        let mut handled = vec!['i'];
        assert!(echo_of_handled_key("i", &mut handled));
        assert!(handled.is_empty());
        assert!(!echo_of_handled_key("i", &mut handled));
        // Multi-char input (IME/paste) is never an echo.
        assert!(!echo_of_handled_key("xy", &mut vec!['x']));
    }

    #[test]
    fn command_prompt_shows_ex_and_search() {
        use crate::doc::Doc;
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        assert_eq!(command_prompt(&engine), None);
        engine.press_key(&mut doc, VKey::Char(':'));
        engine.press_key(&mut doc, VKey::Char('w'));
        assert_eq!(command_prompt(&engine), Some(":w".to_string()));
        engine.press_key(&mut doc, VKey::Escape);
        engine.press_key(&mut doc, VKey::Char('/'));
        engine.press_key(&mut doc, VKey::Char('h'));
        assert_eq!(command_prompt(&engine), Some("/h".to_string()));
    }

    /// Drive the real widget headless: one synthetic keypress per frame.
    fn drive_keys(
        ctx: &egui::Context,
        doc: &mut crate::doc::Doc,
        engine: &mut VimEngine,
        keys: &[egui::Key],
    ) -> Vec<Effect> {
        drive_mods(
            ctx,
            doc,
            engine,
            &super::SourceOptions::default(),
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &keys
                .iter()
                .map(|key| (*key, egui::Modifiers::NONE))
                .collect::<Vec<_>>(),
        )
    }

    /// [`drive_keys`] with modifiers, options, language, and a highlight
    /// cache: Ctrl chords, the vim/modeless switch, and highlighting
    /// exercise the real routing.
    fn drive_mods(
        ctx: &egui::Context,
        doc: &mut crate::doc::Doc,
        engine: &mut VimEngine,
        options: &super::SourceOptions,
        language: super::Language,
        highlight: &mut super::HighlightCache,
        keys: &[(egui::Key, egui::Modifiers)],
    ) -> Vec<Effect> {
        fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }
        }
        let mut effects = Vec::new();
        for (key, modifiers) in keys {
            let mut raw = egui::RawInput::default();
            raw.events.push(key_event(*key, *modifiers));
            // A real backend also delivers the printable echo as Text
            // (never for Ctrl-held chords).
            if !modifiers.command
                && let Some(c) = super::egui_key(*key, *modifiers).and_then(|mapped| match mapped {
                    VKey::Char(c) => Some(c),
                    _ => None,
                })
            {
                raw.events.push(egui::Event::Text(c.to_string()));
            }
            let mut output = ctx.run_ui(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let outcome = super::show_source(
                        ui, doc, engine, options, highlight, language, "headless",
                    );
                    effects = outcome.effects;
                });
            });
            // No renderer headless: acknowledge font texture uploads.
            output.textures_delta.clear();
        }
        effects
    }

    #[test]
    fn colon_q_emits_quit_through_view() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        let effects = drive_keys(
            &ctx,
            &mut doc,
            &mut engine,
            &[egui::Key::Colon, egui::Key::Q, egui::Key::Enter],
        );
        assert!(effects.contains(&Effect::Quit));
        assert_eq!(engine.mode(), Mode::Normal);
    }

    #[test]
    fn colon_split_and_tabnew_reach_the_host_through_view() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        let effects = drive_keys(
            &ctx,
            &mut doc,
            &mut engine,
            &[
                egui::Key::Colon,
                egui::Key::S,
                egui::Key::P,
                egui::Key::L,
                egui::Key::I,
                egui::Key::T,
                egui::Key::Enter,
            ],
        );
        assert_eq!(effects, vec![Effect::Split]);
        let effects = drive_keys(
            &ctx,
            &mut doc,
            &mut engine,
            &[
                egui::Key::Colon,
                egui::Key::T,
                egui::Key::A,
                egui::Key::B,
                egui::Key::N,
                egui::Key::E,
                egui::Key::W,
                egui::Key::Enter,
            ],
        );
        assert_eq!(effects, vec![Effect::TabNew]);
    }

    #[test]
    fn colon_set_wrap_reaches_the_view_option_through_view() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        assert_eq!(engine.wrap(), None);
        drive_keys(
            &ctx,
            &mut doc,
            &mut engine,
            &[
                egui::Key::Colon,
                egui::Key::S,
                egui::Key::E,
                egui::Key::T,
                egui::Key::Space,
                egui::Key::W,
                egui::Key::R,
                egui::Key::A,
                egui::Key::P,
                egui::Key::Enter,
            ],
        );
        assert_eq!(engine.wrap(), Some(true));
    }

    #[test]
    fn insert_opener_echo_does_not_type_through_view() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        drive_keys(&ctx, &mut doc, &mut engine, &[egui::Key::I]);
        assert_eq!(engine.mode(), Mode::Insert);
        assert_eq!(doc.text(), "hi");
    }

    #[test]
    fn mode_names_cover_every_mode() {
        assert_eq!(mode_name(Mode::Normal), "NORMAL");
        assert_eq!(mode_name(Mode::Insert), "INSERT");
        assert_eq!(mode_name(Mode::Visual), "VISUAL");
        assert_eq!(mode_name(Mode::VisualLine), "V-LINE");
        assert_eq!(mode_name(Mode::VisualBlock), "V-BLOCK");
        assert_eq!(mode_name(Mode::Command), "COMMAND");
    }

    #[test]
    fn markdown_selection_tab_indents_and_shift_tab_restores_lines() {
        for vim in [true, false] {
            for selection in [VKey::Char('v'), VKey::Char('V'), VKey::CtrlV] {
                let ctx = egui::Context::default();
                let mut doc = crate::doc::Doc::new("- one\n- two\nlast\n");
                let mut engine = VimEngine::new();
                if !vim && selection == VKey::Char('v') {
                    // Modeless charwise selections end at the caret, so the
                    // selection must reach into the second line to cover it.
                    engine.press_key(&mut doc, selection);
                    engine.press_key(&mut doc, VKey::Down);
                    engine.press_key(&mut doc, VKey::Right);
                } else {
                    engine.press_key(&mut doc, VKey::Down);
                    engine.press_key(&mut doc, selection);
                    engine.press_key(&mut doc, VKey::Up);
                }
                let mode = engine.mode();
                let ends = engine.selection();
                let options = SourceOptions {
                    vim,
                    soft_wrap: true,
                    ..SourceOptions::default()
                };
                let mut highlight = HighlightCache::default();
                // Establish focus before Tab, as a user click does.
                drive_mods(
                    &ctx,
                    &mut doc,
                    &mut engine,
                    &options,
                    Language::Markdown,
                    &mut highlight,
                    &[
                        (egui::Key::F5, egui::Modifiers::NONE),
                        (egui::Key::F5, egui::Modifiers::NONE),
                    ],
                );
                for (modifiers, expected) in [
                    (egui::Modifiers::NONE, "    - one\n    - two\nlast\n"),
                    (egui::Modifiers::SHIFT, "- one\n- two\nlast\n"),
                ] {
                    drive_mods(
                        &ctx,
                        &mut doc,
                        &mut engine,
                        &options,
                        Language::Markdown,
                        &mut highlight,
                        &[(egui::Key::Tab, modifiers)],
                    );
                    assert_eq!(doc.text(), expected);
                    assert_eq!(engine.mode(), mode);
                    assert!(ctx.memory(|memory| memory.has_focus(source_focus_id("headless"))));
                }
                assert_eq!(engine.selection(), ends);
                engine.undo(&mut doc);
                assert_eq!(doc.text(), "    - one\n    - two\nlast\n");
                engine.undo(&mut doc);
                assert_eq!(doc.text(), "- one\n- two\nlast\n");
                engine.redo(&mut doc);
                assert_eq!(doc.text(), "    - one\n    - two\nlast\n");
            }
        }
    }

    #[test]
    fn insert_mode_tab_still_inserts_a_tab() {
        let ctx = egui::Context::default();
        let mut doc = crate::doc::Doc::new("text");
        let mut engine = VimEngine::new();
        drive_keys(&ctx, &mut doc, &mut engine, &[egui::Key::I, egui::Key::Tab]);
        assert_eq!(doc.text(), "\ttext");
        assert_eq!(engine.mode(), Mode::Insert);
    }

    #[test]
    fn ctrl_chords_route_by_vim_and_mode() {
        // Vim owns Ctrl-V/H in its modes; the IDE owns them elsewhere.
        assert!(ctrl_v_is_block(true, Mode::Normal));
        assert!(ctrl_v_is_block(true, Mode::VisualBlock));
        assert!(!ctrl_v_is_block(true, Mode::Insert));
        assert!(!ctrl_v_is_block(true, Mode::Command));
        assert!(!ctrl_v_is_block(false, Mode::Normal));
        assert!(ctrl_h_is_motion(true, Mode::Normal));
        assert!(ctrl_h_is_motion(true, Mode::Visual));
        assert!(!ctrl_h_is_motion(true, Mode::Insert));
        assert!(!ctrl_h_is_motion(true, Mode::Command));
        assert!(!ctrl_h_is_motion(false, Mode::Normal));
    }

    #[test]
    fn ctrl_v_enters_block_in_vim_normal() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("ab\ncd\n");
        let mut engine = VimEngine::new();
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &super::SourceOptions::default(),
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::V, egui::Modifiers::CTRL)],
        );
        assert_eq!(engine.mode(), Mode::VisualBlock);
    }

    #[test]
    fn block_selection_edits_through_control_keys() {
        for control in [
            egui::Modifiers::CTRL,
            egui::Modifiers {
                ctrl: true,
                command: true,
                ..egui::Modifiers::NONE
            },
        ] {
            for toggle in [egui::Key::V, egui::Key::Q] {
                for (action, expected) in [
                    (egui::Key::I, "Xabcd\nXefgh\n"),
                    (egui::Key::A, "abXcd\nefXgh\n"),
                    (egui::Key::C, "Xcd\nXgh\n"),
                    (egui::Key::Delete, "cd\ngh\n"),
                ] {
                    let ctx = egui::Context::default();
                    let mut doc = crate::doc::Doc::new("abcd\nefgh\n");
                    let mut engine = VimEngine::new();
                    let mut keys = vec![
                        (toggle, control),
                        (egui::Key::ArrowRight, egui::Modifiers::NONE),
                        (egui::Key::ArrowDown, egui::Modifiers::NONE),
                        (
                            action,
                            if matches!(action, egui::Key::I | egui::Key::A) {
                                egui::Modifiers::SHIFT
                            } else {
                                egui::Modifiers::NONE
                            },
                        ),
                    ];
                    if action != egui::Key::Delete {
                        keys.push((egui::Key::X, egui::Modifiers::SHIFT));
                        keys.push((egui::Key::Escape, egui::Modifiers::NONE));
                    }
                    drive_mods(
                        &ctx,
                        &mut doc,
                        &mut engine,
                        &SourceOptions::default(),
                        Language::Plain,
                        &mut HighlightCache::default(),
                        &keys,
                    );
                    assert_eq!(doc.text(), expected, "{control:?} {toggle:?} {action:?}");
                    assert_eq!(engine.mode(), Mode::Normal);
                    drive_keys(&ctx, &mut doc, &mut engine, &[egui::Key::U]);
                    assert_eq!(doc.text(), "abcd\nefgh\n");
                }
            }
        }
    }

    #[test]
    fn command_v_pastes_and_control_v_ignores_clipboard_echo() {
        for (modifiers, expected, mode) in [
            (
                egui::Modifiers {
                    mac_cmd: true,
                    command: true,
                    ..egui::Modifiers::NONE
                },
                "Xab\ncd\n",
                Mode::Normal,
            ),
            (egui::Modifiers::CTRL, "ab\ncd\n", Mode::VisualBlock),
        ] {
            let ctx = egui::Context::default();
            let mut doc = crate::doc::Doc::new("ab\ncd\n");
            let mut engine = VimEngine::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        egui::Event::Key {
                            key: egui::Key::V,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers,
                        },
                        egui::Event::Paste("X".into()),
                    ],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        show_source(
                            ui,
                            &mut doc,
                            &mut engine,
                            &SourceOptions::default(),
                            &mut HighlightCache::default(),
                            Language::Plain,
                            "headless",
                        );
                    });
                },
            );
            output.textures_delta.clear();
            assert_eq!(doc.text(), expected);
            assert_eq!(engine.mode(), mode);
        }
    }

    #[test]
    fn ctrl_v_pastes_without_vim() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("ab\ncd\n");
        let mut engine = VimEngine::new();
        let options = super::SourceOptions {
            vim: false,
            ..super::SourceOptions::default()
        };
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::V, egui::Modifiers::COMMAND)],
        );
        assert_ne!(engine.mode(), Mode::VisualBlock);
        assert_eq!(doc.text(), "ab\ncd\n");
    }

    #[test]
    fn ctrl_s_saves_and_ctrl_f_searches() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        let effects = drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &super::SourceOptions::default(),
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::S, egui::Modifiers::COMMAND)],
        );
        assert!(effects.contains(&Effect::Save));
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &super::SourceOptions::default(),
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::F, egui::Modifiers::COMMAND)],
        );
        assert_eq!(engine.mode(), Mode::Command);
        assert_eq!(engine.command_line(), "/");
    }

    #[test]
    fn ctrl_h_motion_in_vim_replace_without() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        // Vim on: Backspace motion in Normal.
        let mut doc = Doc::new("ab");
        let mut engine = VimEngine::new();
        engine.press_key(&mut doc, VKey::Char('l'));
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &super::SourceOptions::default(),
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::H, egui::Modifiers::COMMAND)],
        );
        assert_eq!(engine.cursor(), crate::vim::Cursor { line: 0, col: 0 });
        // Vim off: the GUI find bar in replace mode, not a vim prompt.
        let mut doc = Doc::new("ab");
        let mut engine = VimEngine::new();
        let options = super::SourceOptions {
            vim: false,
            ..super::SourceOptions::default()
        };
        let effects = drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::H, egui::Modifiers::COMMAND)],
        );
        assert!(effects.contains(&Effect::FindBar { replace: true }));
        assert_ne!(engine.mode(), Mode::Command);
    }

    #[test]
    fn modeless_printable_types_in_normal() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        let options = super::SourceOptions {
            vim: false,
            ..super::SourceOptions::default()
        };
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::A, egui::Modifiers::NONE)],
        );
        assert_eq!(doc.text(), "ahi");
        assert_eq!(engine.mode(), Mode::Insert);
    }

    #[test]
    fn modeless_enter_newlines_from_normal() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        let options = super::SourceOptions {
            vim: false,
            ..super::SourceOptions::default()
        };
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert_eq!(doc.text(), "\nhi");
    }

    #[test]
    fn ide_keymap_follows_platform_conventions() {
        use crate::vim::IdeMotion;
        let alt_shift = egui::Modifiers {
            alt: true,
            shift: true,
            ..egui::Modifiers::NONE
        };
        let mac_cmd = egui::Modifiers {
            mac_cmd: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let ctrl = egui::Modifiers::CTRL;
        // macOS: Option moves by word, Command by line edge.
        assert_eq!(
            ide_edit_key(egui::Key::ArrowLeft, egui::Modifiers::ALT, true, 30),
            Some(IdeEdit::Move(IdeMotion::WordLeft, false))
        );
        assert_eq!(
            ide_edit_key(egui::Key::ArrowRight, mac_cmd, true, 30),
            Some(IdeEdit::Move(IdeMotion::LineEnd, false))
        );
        assert_eq!(
            ide_edit_key(egui::Key::D, mac_cmd, true, 30),
            Some(IdeEdit::Duplicate)
        );
        assert_eq!(
            ide_edit_key(egui::Key::Backspace, mac_cmd, true, 30),
            Some(IdeEdit::DeleteLines)
        );
        // Linux/Windows: Ctrl moves by word, Ctrl+Y deletes the line.
        let linux_ctrl = egui::Modifiers {
            command: true,
            ..ctrl
        };
        assert_eq!(
            ide_edit_key(egui::Key::ArrowLeft, linux_ctrl, false, 30),
            Some(IdeEdit::Move(IdeMotion::WordLeft, false))
        );
        assert_eq!(
            ide_edit_key(egui::Key::Y, linux_ctrl, false, 30),
            Some(IdeEdit::DeleteLines)
        );
        // Shared: Shift selects, Alt+Shift moves lines, paging uses rows.
        assert_eq!(
            ide_edit_key(egui::Key::End, egui::Modifiers::SHIFT, false, 30),
            Some(IdeEdit::Move(IdeMotion::LineEnd, true))
        );
        assert_eq!(
            ide_edit_key(egui::Key::ArrowUp, alt_shift, true, 30),
            Some(IdeEdit::MoveLines { up: true })
        );
        assert_eq!(
            ide_edit_key(egui::Key::PageDown, egui::Modifiers::NONE, true, 30),
            Some(IdeEdit::Move(IdeMotion::PageDown(30), false))
        );
        assert_eq!(
            ide_edit_key(egui::Key::A, egui::Modifiers::NONE, true, 30),
            None
        );
    }

    #[test]
    fn modeless_shift_arrows_select_and_typing_replaces_through_view() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hello world");
        let mut engine = VimEngine::new();
        let options = super::SourceOptions {
            vim: false,
            ..super::SourceOptions::default()
        };
        let shift = egui::Modifiers::SHIFT;
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut super::HighlightCache::default(),
            &[
                (egui::Key::ArrowRight, shift),
                (egui::Key::ArrowRight, shift),
                (egui::Key::ArrowRight, shift),
                (egui::Key::ArrowRight, shift),
                (egui::Key::ArrowRight, shift),
                (egui::Key::X, egui::Modifiers::NONE),
            ],
        );
        assert_eq!(doc.text(), "x world");
        assert_eq!(engine.mode(), Mode::Insert);
    }

    #[test]
    fn command_d_and_slash_edit_lines_through_view_in_vim_mode() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("let a = 1;\n");
        let mut engine = VimEngine::new();
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &super::SourceOptions::default(),
            super::Language::Rust,
            &mut super::HighlightCache::default(),
            &[
                (egui::Key::D, egui::Modifiers::COMMAND),
                (egui::Key::Slash, egui::Modifiers::COMMAND),
            ],
        );
        assert_eq!(doc.text(), "let a = 1;\n// let a = 1;\n");
    }

    #[test]
    fn squiggles_convert_utf16_ranges_and_widen_empty_ones() {
        use crate::lsp::{Diagnostic, Position, Range, Severity};
        let doc = crate::doc::Doc::new("let é = bad_name;\n");
        let at = |character| Position { line: 0, character };
        let diagnostics = [
            Diagnostic {
                range: Range {
                    start: at(8),
                    end: at(16),
                },
                severity: Some(Severity::Error),
                message: "unknown".into(),
            },
            Diagnostic {
                range: Range {
                    start: at(4),
                    end: at(4),
                },
                severity: Some(Severity::Warning),
                message: "unused".into(),
            },
        ];
        let squiggles = super::super::squiggles(&doc, &diagnostics);
        assert_eq!(
            squiggles
                .iter()
                .map(|s| (s.line, s.start, s.end))
                .collect::<Vec<_>>(),
            [(0, 4, 5), (0, 8, 16)]
        );
    }

    #[test]
    fn f8_walks_problems_and_command_f_opens_the_find_bar_without_vim() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("a\nb\nc\n");
        let mut engine = VimEngine::new();
        let squiggle = |line| super::super::Squiggle {
            line,
            start: 0,
            end: 1,
            severity: crate::lsp::Severity::Error,
            message: String::new(),
        };
        let options = super::SourceOptions {
            vim: false,
            squiggles: vec![squiggle(1), squiggle(2)],
            ..super::SourceOptions::default()
        };
        let mut highlight = super::HighlightCache::default();
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut highlight,
            &[(egui::Key::F8, egui::Modifiers::NONE)],
        );
        assert_eq!(engine.cursor().line, 1);
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut highlight,
            &[(egui::Key::F8, egui::Modifiers::NONE)],
        );
        assert_eq!(engine.cursor().line, 2);
        // Past the last problem F8 wraps to the first.
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut highlight,
            &[(egui::Key::F8, egui::Modifiers::NONE)],
        );
        assert_eq!(engine.cursor().line, 1);
        let effects = drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Plain,
            &mut highlight,
            &[(egui::Key::F, egui::Modifiers::COMMAND)],
        );
        assert!(effects.contains(&Effect::FindBar { replace: false }));
        // Vim keeps its own `/` prompt.
        let mut vim_engine = VimEngine::new();
        let effects = drive_mods(
            &ctx,
            &mut doc,
            &mut vim_engine,
            &super::SourceOptions::default(),
            super::Language::Plain,
            &mut highlight,
            &[(egui::Key::F, egui::Modifiers::COMMAND)],
        );
        assert!(!effects.contains(&Effect::FindBar { replace: false }));
        assert_eq!(vim_engine.mode(), Mode::Command);
    }

    #[test]
    fn command_minus_folds_the_caret_block_and_hides_its_body() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("fn a() {\n    x();\n}\nfn b() {}\n");
        let mut engine = VimEngine::new();
        let _ = engine.place_cursor(&mut doc, 1, 4);
        let options = super::SourceOptions {
            vim: false,
            ..super::SourceOptions::default()
        };
        let mut highlight = super::HighlightCache::default();
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Rust,
            &mut highlight,
            &[(egui::Key::Minus, egui::Modifiers::COMMAND)],
        );
        assert!(highlight.folds().is_folded(0));
        assert_eq!(engine.cursor().line, 0);
        // The next frame lays out without the hidden lines.
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Rust,
            &mut highlight,
            &[(egui::Key::F5, egui::Modifiers::NONE)],
        );
        let lines: Vec<usize> = highlight.rows().iter().map(|row| row.line).collect();
        assert!(!lines.contains(&1) && !lines.contains(&2), "{lines:?}");
        assert!(highlight.rows().first().is_some_and(|row| row.folded));
        // Cmd+= opens it again.
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Rust,
            &mut highlight,
            &[(egui::Key::Equals, egui::Modifiers::COMMAND)],
        );
        assert!(!highlight.folds().is_folded(0));
    }

    #[test]
    fn arrow_keys_hop_over_folded_blocks() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("fn a() {\n    x();\n}\nnext();\n");
        let mut engine = VimEngine::new();
        let options = super::SourceOptions {
            vim: false,
            ..super::SourceOptions::default()
        };
        let mut highlight = super::HighlightCache::default();
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Rust,
            &mut highlight,
            &[(egui::Key::Minus, egui::Modifiers::COMMAND)],
        );
        assert!(highlight.folds().is_folded(0));
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Rust,
            &mut highlight,
            &[(egui::Key::ArrowDown, egui::Modifiers::NONE)],
        );
        assert_eq!(engine.cursor().line, 3);
        assert!(highlight.folds().is_folded(0));
        drive_mods(
            &ctx,
            &mut doc,
            &mut engine,
            &options,
            super::Language::Rust,
            &mut highlight,
            &[(egui::Key::ArrowUp, egui::Modifiers::NONE)],
        );
        assert_eq!(engine.cursor().line, 0);
        assert!(highlight.folds().is_folded(0));
    }

    #[test]
    fn inlay_hints_push_later_text_right_but_not_the_caret_before_them() {
        use crate::doc::Doc;
        let caret_x = |col: usize, hints: Vec<crate::inlay::InlayHint>| {
            let ctx = egui::Context::default();
            let mut doc = Doc::new("let x = 1;\n");
            let mut engine = VimEngine::new();
            engine.set_modeless(true);
            let _ = engine.place_cursor(&mut doc, 0, col);
            let mut highlight = super::HighlightCache::default();
            let options = super::SourceOptions {
                vim: false,
                autofocus: true,
                inlay_hints: std::sync::Arc::new(hints),
                ..super::SourceOptions::default()
            };
            let mut caret = None;
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    caret = show_source(
                        ui,
                        &mut doc,
                        &mut engine,
                        &options,
                        &mut highlight,
                        Language::Rust,
                        "hints.rs",
                    )
                    .caret_rect;
                });
                output.textures_delta.clear();
            }
            caret.expect("caret painted").min.x
        };
        let hint = || {
            vec![crate::inlay::InlayHint {
                line: 0,
                col: 5,
                label: ": i32".into(),
            }]
        };
        let advance = caret_x(1, Vec::new()) - caret_x(0, Vec::new());
        // Before the hinted char the caret stays put; after it, the caret
        // moves right by the hint's five cells.
        assert!((caret_x(5, hint()) - caret_x(5, Vec::new())).abs() < 0.01);
        let shifted = caret_x(6, hint()) - caret_x(6, Vec::new());
        assert!(
            (shifted - 5.0 * advance).abs() < 0.01,
            "{shifted} vs {advance}"
        );
    }
}
