Vendored from https://github.com/kemokempo/egui_term at 31bbc7ab8503c9518fcee5717cfa29011e59f451. MIT license retained. Workspace manifest simplified and egui advanced from 0.35 to 0.36 for egui_dock compatibility. Local changes are documented here as made.

Local integration changes: expose a path token lookup for the context menu; allow keyboard input to the focused terminal when the pointer is elsewhere while preserving pointer hit testing.

The daemon answers terminal queries, so the widget does not send duplicate replies. Event subscription exits cleanly when its receiver is gone.

## Islands typography (2026-09-08)

Default size is 13 logical points. Font measurement applies 1.2 line spacing and
ceil-quantizes both cell dimensions once; PTY grid sizing, paint, cursor, selection,
mouse coordinates and pixel-wheel scrolling use that same geometry. Bold cells
use the host's optional `Terminal Bold` font family, falling back to the normal
family for other embedders. Default foreground/background are Islands Dark
#D1D3D9/#191A1C; terminal applications can still supply ANSI/truecolor colors.

## Plan 3 target actions (2026-09-08)

Added `LinkTarget` and `TerminalBackend::target_at`: a bounded logical-line scan
across grid wrapping and scrollback, skipping wide-character spacer cells, with
quoted-path tokenization and local underline rectangles. The widget performs no
filesystem access. The application resolves targets on its worker and supplies
hover actions. `TerminalView::external_links(true)` delegates modified-click
opening to the application while retaining terminal mouse reporting and ordinary
selection. Standalone users retain the original link behavior by default.

The native fixture checks pointer hover -> editor split; widget unit tests cover
quoted spaces, punctuation, wrapping, scrolling and wide-cell hit testing.

The flat terminal context menu adds `select_all()` and reads copied selection text
from the cached grid, including offscreen history, wrapped lines, combining marks,
and wide-cell spacing. Copying does not acquire an additional live terminal lock
while rendering. A regression checks offscreen copying and newline preservation.

## Agent terminal wheel scrolling (2026-09-14)

Wheel input now sends mouse wheel press reports when the terminal application
enables mouse reporting, allowing full-screen agents to scroll their own history.
Ordinary terminals retain local scrollback and alternate-screen arrow scrolling;
Shift bypasses mouse reporting. Pixel deltas still accumulate into complete rows.
Regression tests cover both wheel directions, partial trackpad deltas, and Shift.

## Terminal keyboard focus (2026-09-14)

Focused terminals lock Tab, arrow keys, and Escape to terminal input using egui's
focus event filter. This prevents widget navigation from briefly focusing and
highlighting dock separators. A headless egui regression covers Tab, Shift+Tab,
arrows, Escape, and repeated presses while confirming input remains available.

## Unbound Command/Ctrl chords (2026-09-16)

Keys with no static binding are no longer dropped while Ctrl or Command is
held (egui-winit omits `Event::Text` for those modifiers). Unbound chords are
written as kitty CSI u: Command is Super (`Cmd+E` → `CSI 101;9 u`), Ctrl stays
Ctrl. Existing bindings still win (Ctrl+A–Z C0, arrows, macOS Cmd+C/V copy and
paste). Alt-only letters remain Text events so Option continues to compose.

Modified navigation keys and F1–F12 preserve the protocol's CSI letter/tilde
suffixes instead of emitting private-use CSI u codes. Regression coverage checks
all navigation keys, F1–F12, and the F13/F35 CSI u boundaries.

## 2026-09-15: hover wheel routing and local-history bypass

Wheel eligibility now follows the enabled, clipped response under the pointer,
independently of keyboard focus. Consumed raw wheel events and egui's derived
smooth delta are removed from enclosing scroll controls. Application wheel reports
use current viewport pointer coordinates. Per-view point and line fractions reset
on hover loss, gesture start/cancellation, or an idle gap; page units use the visible
row count. Shift sends `ScrollLocal`, bypassing both application mouse reporting
and alternate-scroll arrow translation. Ordinary `Scroll` retains mode-controlled
alternate scrolling. The host disables terminal responses during blocking overlays
and disabled editor-close states. See `docs/VALIDATION.md` for native versus live
Codex verification boundaries.
