# vt100

Vendored vt100 0.16.2 from the published crates.io package.
Upstream: https://github.com/doy/vt100-rust. MIT license retained in LICENSE.
The manifest omits unused upstream development dependencies.

## Muse scrollback and clear-history handling (2026-10-12)

- Retain rows scrolled off a region that starts at screen row zero, including
  regions with a fixed footer. This matches alacritty_terminal in the GUI.
  Regions below a fixed header and alternate-screen output do not add history.
- Handle `CSI 3 J` inside the parser, at its position in the byte stream. Clear
  retained rows and the history offset while keeping the visible screen, cursor,
  attributes, scroll margins, saved cursor, and history capacity. This replaces
  the daemon's end-of-read parser rebuild, which lost output after a clear in
  the same read.

Behavior tests live in crates/daemon/src/terminal_events.rs, including attachment
replay through the GUI's alacritty_terminal parser.
