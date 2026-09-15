# Plan: Blitz HTML preview

SPEC Alignment: aligned — `docs/ARCHITECTURE.md` forbids Chromium/webview. GUI-only raster tab, no PTY. System browser stays the JS/full-page path.

Stay in current workspace.

```text
open .html/.htm/.xhtml
  ├─ default            → Tab::Html (Blitz CPU raster, no PTY)
  │     ├─ ok           → texture + header Open in browser
  │     └─ fail/timeout → error + same Open in browser
  ├─ Open as text       → Neovim (today)
  └─ Open in browser    → file:// OS browser (already shipped)
```

- [x] 1. Spike Blitz CPU (`blitz-html/dom/paint` + `anyrender_vello_cpu` only). Abort if cold build/size/crash is bad.
- [x] 2. `html_preview.rs` worker: file → `ColorImage`. DummyNet default; optional `file://` only under parent dir. Bounds + unit tests.
- [x] 3. `Tab::Html { path }` + layout v4 (load 2|3|4). Gate in `open_file_mode` like images.
- [x] 4. Pane header: Reload · Open as text · **Open in browser** (always, including error).
- [x] 5. Docs REFERENCE/VALIDATION; MPL note for transitive `stylo`.

No JS. No network except optional local images. Never raster on the GUI thread.

Unresolved: markdown raw HTML stays text (yes); relative `<img>` sandbox under parent (yes).
