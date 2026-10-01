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

## Viewport snapshot and style-run paint (2026-09-19)

`last_content.grid` is a viewport-sized copy (`display_offset` 0). Live 10k
history stays in `FairMutex<Term>`. `RenderableContent.display_offset` is the
live scroll for mouse reports and host fixtures. Copy uses
`Term::selection_to_string()` cached in `selected_text`. `open_link` reads the
live grid. Paint groups same-style cells into one `LayoutJob` galley with
`extra_letter_spacing` so glyphs sit on the integer cell grid (cursor, wrap, and
text stay aligned). `TerminalSize::from_layout` is the single pane-to-grid
mapping; the host PTY is created at the painted size instead of 80×50.

## Mouse-mode selection and copy (2026-09-17)

Left-drag always selects host text, even when the application enables mouse
reporting. Wheel still sends mouse reports (Shift bypasses to local scroll).
Pointer release outside the pane ends the drag. Empty Copy does not write an
empty clipboard (Linux Ctrl+C still interrupts when there is no selection).
`sync()` snapshots the visible grid only after PTY/output/scroll/resize;
selection updates the cached range and copy string in place. Widget sense is
`click_and_drag` so the pointer stays captured.

## Ctrl+E text is ENQ (2026-09-21)

A Ctrl+letter `Event::Text` (`"e"` while Control is held) is written as the C0
byte. It used to be ignored whenever a binding existed, which dropped Ctrl+E
when the Key event had been stamped without Control. The Key event is skipped
when that Text event is also in the frame, so the byte is written once.
Ctrl+A stays `0x01`. Command chords are unchanged.

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

## In-terminal literal find (2026-09-20)

New `find` module with a UI-free literal matcher (`find_in_rows`, char-offset
hits, `MAX_MATCHES` cap) plus widget unit tests. `TerminalBackend::search_rows`
extracts per-row text and grid columns across the live grid including retained
scrollback (wide-char spacers skipped, trailing whitespace trimmed); `find`
maps hits to `FoundMatch` grid coordinates; `reveal_grid_line` scrolls the
viewport with a two-line margin via local `Scroll::Delta` only, so it never
writes to the PTY and stays a no-op in alt-screen mode. `TerminalView::
find_highlight` paints other matches dimmed and the current match in the
selection color, with ranges grouped by line once per frame. Wrapped logical
lines stay split across rows, so matches cannot span rows in v1.

## Alternate-scroll respects application cursor keys (2026-09-25)

Wheel scrolling on the alternate screen with alternate-scroll enabled always
emitted SS3 (`ESC O A/B`), even when the application had not set DECCKM
application-cursor mode and expected CSI (`ESC [ A/B`) — the same distinction
the keyboard arrow bindings already make. Those wheels were ignored by such
agents, so scrolling felt dead. The byte encoding moved into `scroll_key_bytes`,
which follows `APP_CURSOR` like the bindings do. Unit tests cover CSI without
application cursor, SS3 with it, both directions, and zero delta.

## Bracketed paste for pasted text (2026-09-28)

Pastes wrote the clipboard text to the PTY verbatim, so multi-line pastes
arrived as line-by-line submits: at a shell prompt each newline ran a command,
leaving nothing editable to backspace through. `paste_input` now normalizes
newlines to carriage returns (the shared terminal paste convention) and wraps
the text in `ESC [ 200 ~` / `ESC [ 201 ~` markers when the application enabled
bracketed paste (mode 2004), so shells and TUIs insert the paste as one
editable block and backspace can cross its newlines. Both paste paths use it:
the widget's `Event::Paste` handling and the application's clipboard-Paste
command. Unit tests cover bracketed and plain pastes, CRLF normalization, and
the empty paste.

## Coalesced PTY repaint wakeups (2026-09-29)

The PTY event subscription thread called `request_repaint()` for every
terminal event, so a fast-producing terminal woke the UI thread once per
event and could hold the window at back-to-back full frames. It now calls
`request_repaint_after(16 ms)`; egui keeps the smallest pending deadline, so
a burst coalesces into at most one frame per 16 ms while hidden panes are
still updated lazily on their next paint. Host-side behavior is unchanged:
the application already drops backends for terminals it is not painting.

## Painted-only 30 fps repaint wakeups (2026-09-29)

Terminal output still forced 60 fps full-window frames, including for a
terminal an agent was animating while it was not on screen. The subscription
thread now uses a leading-edge 33 ms throttle (about 30 fps): an event after an
idle period calls `request_repaint()` immediately, so the first echo after a
keystroke is not delayed, while a sustained burst keeps a pending
`request_repaint_after` cap. It only wakes the UI when the backend was painted
in the current frame: `TerminalBackend::set_painted` is cleared for every
backend at frame start (including before the host's exit-screen early return)
and set by the host when the terminal is drawn. A hidden terminal records
`grid_dirty` but does not wake the UI, so its next paint picks up the new grid
instead of driving frames the user cannot see.


## Shared output repaint budget and parsed colors (2026-09-30)

Visible terminal panes now share a context-local repaint budget. The budget is
reset by actual paints (`set_painted(true)`), not by independent subscription
threads' last immediate wakeups. This avoids staggered panes multiplying output
frames and a scheduled frame being immediately followed by another leading-edge
wake. Hidden panes still only mark their grids dirty. Output after an idle period
can wake immediately; input events remain independent of the output budget.

TerminalTheme parses palette strings once into an immutable, Arc-shared color
array. Named/indexed colors use direct lookup, including the xterm cube and
grayscale ramp; truecolor stays direct RGB. Default themes share one initialized
table, avoiding rebuilding a discarded default table on every widget creation.
The host caches its configured theme until foreground/background changes.
Regressions cover shared frame deadlines, idle wakeups, separate contexts,
indexed colors, custom colors, bright-foreground fallback, and theme sharing.

## Cached default keymap (2026-09-30)

`TerminalView::new` used to call `BindingsLayout::new()` on every frame. The
default keymap now lives in a `OnceLock` behind `Arc`. `new` clones that
`Arc`. `add_bindings` uses `Arc::make_mut`, so a custom layout copies once
and does not mutate the shared default. `sync` is unchanged: an idle PTY
still skips the viewport copy unless the grid is dirty.

## Scroll-frame font-measurement costs (2026-09-30)

Wheel handling measured the cell font twice per scroll frame and paint took
two separate font-atlas locks (bold-family probe plus glyph width). The wheel
path now measures once and paint reads both values under one lock. No
behavior change; existing scroll unit tests cover the path.

## Terminal focus no longer steals from other widgets (2026-09-30)

`focus_terminal` unconditionally called `request_focus` every frame a pane was
the active session. Because the terminal repaints after the sidebars, clicking a
sidebar text field focused it and the terminal pulled focus back in the same
frame, so the field never kept focus and keystrokes reached the shell. Focus is
now requested only when nothing is focused or the terminal itself already holds
focus; egui's press-based focus surrender hands focus to the terminal when the
user clicks it. Regression: `terminal_focus_does_not_steal_from_another_widget`.

## Wheel scrolls during a selection drag (2026-10-01)

Left-drag always selects host text. Pinning the moving end to a stationary
pointer does not select the line that enters at the far edge of the viewport.
A drag wheel grows the selection by the lines the viewport actually moved.
Primary history, including mouse-reporting terminals that are not on the
alternate screen, uses `ScrollLocal` and extends by the display-offset change.
That change is zero at a history boundary or when the grid has no scrollback,
so the selection stays put. The alternate screen reports the wheel to the
agent and grows by the reported line count. A stationary pointer inside the
pane does not `SelectUpdate` in that same frame; the extended end stays until
the pointer moves. Holding the pointer outside the pane keeps scrolling and
pins the end to that edge. An application clear during the drag restores the
selection with the original press point still fixed, so an upward drag keeps
its bottom endpoint when the pointer moves again. A scroll in that moment
restores the cleared range before extending it. A same-row drag compares
columns, so a right-to-left selection keeps the press cell. Output scrolling
rotates the selection's fixed end and leaves the stored anchor coordinate
behind; a later wheel reads that fixed end from the selection instead of
matching the stale coordinate. Plain (non-drag)
wheel routing is unchanged. Regressions:
`alternate_screen_local_scroll_does_not_move_the_viewport`,
`drag_scroll_selects_the_history_line_that_enters_the_view`,
`local_scroll_extends_by_the_lines_the_viewport_moved`,
`local_scroll_extends_the_selection_to_the_cell_under_the_pointer`,
`local_scroll_with_no_history_does_not_grow_the_selection`,
`reviving_a_cleared_selection_keeps_the_dragged_text`,
`reviving_an_upward_drag_keeps_the_bottom_endpoint`,
`backward_same_row_drag_keeps_the_press_cell_when_scrolling`,
`extend_after_clear_keeps_the_previous_selection`,
`output_during_drag_does_not_swap_fixed_endpoint`,
`stationary_drag_inside_the_pane_does_not_reset_the_selection`,
`wheel_during_drag_on_the_alternate_screen_scrolls_the_agent_and_updates_the_selection`,
`wheel_during_drag_on_the_primary_screen_scrolls_history_and_updates_the_selection`,
`drag_outside_an_alternate_screen_scrolls_the_agent`.

## Test-only drag helper (2026-10-01)

`scroll_local_drag` is only called from widget unit tests, so it is gated
with `#[cfg(test)]` to avoid a `dead_code` warning in normal builds.
