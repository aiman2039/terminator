Vendored egui-winit 0.36.1 from https://github.com/emilk/egui at
4c1f2fae95475a40e524884ebb298bcb1714b08e (crates/egui-winit).
MIT and Apache-2.0 licenses retained. Cargo.toml is the published crate manifest.

## Ctrl+E control character (2026-09-21)

AppKit delivers Ctrl+E as ENQ (`\u{5}`) in `text_with_all_modifiers`. If egui's
modifier snapshot does not yet have Control, the key event is a plain `E` and
the printable `"e"` is what remains. Map that C0 byte back to the letter and
set Control on the key event so the terminal binding sends `0x05`. Command
chords are unchanged (`text_with_all_modifiers` is not a C0 byte).

## Image-only paste intent (2026-09-14)

The keyboard paste branch now emits `egui::Event::Paste("")` when the OS
clipboard has no text, instead of dropping the shortcut. Terminator consumes
this event only in the focused terminal and asks its worker to read/save the
clipboard image. Nonempty text paste is unchanged; egui TextEdit explicitly
ignores empty paste events. The terminal context menu uses its own worker
request, avoiding eframe's text-only RequestPaste implementation.

No clipboard polling, image decoding, or filesystem work is added here.

## Native fixture rendering while occluded (2026-09-15)

An explicit `test-support` feature permits `TERMINATOR_TEST_RENDER_OCCLUDED` to
clear viewport occlusion only when `TERMINATOR_CAPTURE_PATH` is also set. This
lets eframe run actual UI/input passes for a background, click-through fixture
without raising it over the user's work. Minimized behavior is retained. Normal
packaging does not enable this feature, and normal application behavior is
unchanged without both fixture environment flags. Long-running live Codex tests
exposed the need: eframe otherwise keeps only logic/IPC running once covered.
