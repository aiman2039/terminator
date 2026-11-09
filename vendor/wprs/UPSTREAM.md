# wprs

Vendored from https://github.com/wayland-transpositor/wprs (formerly
`google/wprs`) at commit `12b864dc5b308e63edede4714f664afa88507fce`
(2026-08-31). Apache-2.0; `LICENSE` and the per-file copyright headers are
retained.

## Local changes

- `Cargo.lock`: crates with security advisories are updated to fixed versions.
- `serialization::Request::ReplayDone` (last variant, so the other
  variants keep their tags): wprsd sends it in `handle_connect` after the
  surfaces that already exist, so Terminator's bridge knows the window list
  is complete without a timer. `wprsc` ignores it. A wprsd without it still
  works with the bridge, which then falls back to a quiet-time guess.

## How Terminator uses it

Remote apps (`crates/remote-apps`, `docs/REMOTE_APPS.md`). On a Linux host,
the host's Terminator install ships `wprsd` and `xwayland-xdg-shell` built from
this tree, plus Terminator's host bridge, which links this crate as a wprs
client. The wprs (rkyv) protocol stays on the host; only the versioned
Terminator display protocol crosses the network.

The crate is Linux-only for Terminator: smithay 0.7 always links
`libxkbcommon`, and its Xwayland module does not build on macOS. The macOS
GUI and daemon never compile it.

`rust-toolchain.toml` pins 1.88; Terminator builds it with the workspace
toolchain (1.97.1+).
