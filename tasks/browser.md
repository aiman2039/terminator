# Plan: first-class OS webview tabs

Approved in session. SPEC.md absent — this file is spec until a human adds one.

Do not clobber `tasks/todo.md` (player, layout v5).

## Summary

Replace Blitz `Tab::Html` with GUI-owned OS webview tabs (wry: WKWebView / WebKitGTK). Real JS/CSS. Dies with GUI. Local HTML + http(s). Isolated profile. No Chromium/CEF/Servo/Electron.

```
T1 layout ─ T2 allowlist ─ T3 wry+occlusion ─ T4 chrome ─ T5 policy ─ T6 drop Blitz ─ T7 gui ─ T8 docs
```

- [x] Task 1: `Tab::Browser` + layout v6 + migrate v4 Html
- [x] Task 2: Allowlist + isolated `webview/` profile
- [x] Task 3: wry child view + hide when covered
- [x] Task 4: Back/forward/reload/Open in browser/Open as text
- [x] Task 5: No JS bridge; deny downloads; window.open http(s) → tab; Wayland fail closed
- [x] Task 6: Remove blitz-* raster
- [x] Task 7: `cargo xtask gui browser`
- [x] Task 8: Docs + AGENTS.md

## Accept

- Archify HTML matches browser
- Covered pane does not float over other tabs
- GUI quit destroys webviews; PTYs remain
- v6 restore; older GUI read-only
