# Native-edit reference verdicts (Phase 0)

Date: 2026-09-20. Each entry: what was read, verdict, integration seam.
Rule: `sred` is ideas-only (GPL-3.0-only) — nothing copied.

## Use as library

- **ropey 1.6** ([docs](https://docs.rs/ropey)) — MIT text rope.
  Verdict: shared buffer for doc core and terminal-find row extraction.
- **egui_code_editor 0.3.8** (MIT, egui/eframe 0.36 = ours) — source widget.
  API read from `src/lib.rs`: `CodeEditor::show(ui, text: &mut dyn egui::TextBuffer, syntax: &Syntax) -> TextEditOutput`;
  `show_with_completer` variant; `Syntax`/`Patch`/`TokenType`, `ColorTheme`,
  `Completer`, `Editor` trait for custom token rendering.
  Verdict: adopt behind a wrapper. Buffer takes `dyn TextBuffer`, so a
  ropey-backed adapter is possible (no forced `String` copy for large files).
  `TextEditOutput` gives cursor state to the vim adapter.
- **hjkl-engine + hjkl-buffer + hjkl-vim** (MIT, rust 1.95, ropey 1.6,
  no TUI deps in core) — vim grammar.
  API read from `hjkl-vim` README: `Mode` enum, `feed_input`/`dispatch_input`
  drive normal/operator-pending/insert, `CountAccumulator`, `OperatorKind`,
  `PendingState`, `EngineCmd`.
  Verdict: adopt behind a `ModalEngine` trait (key in, engine commands out);
  egui input mapping is our adapter. Pre-1.0 churn contained by the seam.
  Status 2026-09-20: sandbox blocks crate downloads, so `native_edit::vim`
  ships a hand-rolled Minimal engine behind the same seam; swap in when
  vendoring is possible. Same for `ropey` (`native_edit::doc::Buffer`) and
  `egui_code_editor` (`native_edit::view` is the equivalent surface).

## Patterns only

- **Ferrite** (MIT app, not a library; macOS experimental) — rendered-block
  switching, split raw/preview scroll sync, ropey + comrak + syntect stack.
  Skip its code-exec blocks and terminal workspace (conflicts with
  daemon-owns-PTY model).

## Ideas only / fallback

- **sred-core** — byte-lossless source-anchored buffer, single `command()`
  dispatch. Read only; copy nothing (GPL-3.0-only, egui 0.29).
- **modalkit 0.0.28** — fallback reference only (`crossterm` in lib core,
  ropey 1.5). Hand-rolled minimal grammar is the other fallback.
