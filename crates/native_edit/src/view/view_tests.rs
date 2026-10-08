use super::super::doc::Buffer;
use super::super::highlight::{HighlightCache, Language};
use super::super::vim::{Effect, Key as VKey, ModalEngine, Mode, VimEngine};
use super::keys::{
    IdeAction, command_prompt, ctrl_h_is_motion, ctrl_v_is_block, echo_of_handled_key, egui_key,
    ide_shortcut, mode_name, source_focus_id,
};
use super::render::{SourceOptions, char_display_width, col_at_display_x, display_x, show_source};
use egui;
#[cfg(test)]
mod tests {
    use super::*;

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
            &[(egui::Key::V, egui::Modifiers::COMMAND)],
        );
        assert_eq!(engine.mode(), Mode::VisualBlock);
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
        // Vim off: the replace prompt.
        let mut doc = Doc::new("ab");
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
            &[(egui::Key::H, egui::Modifiers::COMMAND)],
        );
        assert_eq!(engine.mode(), Mode::Command);
        assert_eq!(engine.command_line(), "%s/");
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
}
