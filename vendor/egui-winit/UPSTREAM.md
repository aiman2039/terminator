Vendored egui-winit 0.36.1 from https://github.com/emilk/egui at
4c1f2fae95475a40e524884ebb298bcb1714b08e (crates/egui-winit).
MIT and Apache-2.0 licenses retained. Cargo.toml is the published crate manifest.

## Image-only paste intent (2026-09-14)

The keyboard paste branch now emits `egui::Event::Paste("")` when the OS
clipboard has no text, instead of dropping the shortcut. Terminator consumes
this event only in the focused terminal and asks its worker to read/save the
clipboard image. Nonempty text paste is unchanged; egui TextEdit explicitly
ignores empty paste events. The terminal context menu uses its own worker
request, avoiding eframe's text-only RequestPaste implementation.

No clipboard polling, image decoding, or filesystem work is added here.
