# Bundled CodeDiff

Source: https://github.com/esmuellert/codediff.nvim
Pinned commit: `61521381445aab22d5f988e7d99c2d7a76ccc1a1`.

The Lua/plugin runtime, C library source, documentation and build references are
vendored unchanged. Top-level Lua tests, CI and repository development tooling
are omitted; the native source subtree retains its upstream tests.
Retain LICENSE, ATTRIBUTION.md (including Microsoft's MIT notice), and
libvscode-diff/vendor/utf8proc_LICENSE.md.

Terminator's daemon/build.rs compiles the C sources as a shared library using the
Cargo target compiler, with bundled utf8proc and without OpenMP. It embeds that
library and the Lua runtime into the daemon. No prebuilt binaries or runtime
network installation are used. cc handles target/compiler selection; the supported
native targets are macOS and Linux. Neovim itself is not bundled.

The build script watches VERSION, the Lua/plugin runtime directories, and the
native C/header inputs, including utf8proc_data.c. Documentation and upstream
tests do not trigger a native rebuild. Runtime directories remain watched so
new and removed assets update the embedded manifest. Generated version.h and
review_assets.rs files are written only when their contents change.

The app-owned daemon/src/review.lua profile uses the pinned internal
codediff.commands.handlers.file_diff.run API to pass filenames literally. On
upgrades, recheck this API, read-only enforcement after view creation/layout
switches, native library loading, and q/tab-X closing. The profile loads no user
configuration or plugins and disables CodeDiff mutation keymaps. It compares two
private immutable snapshots, rather than giving CodeDiff a repository root.
When updating the pin, run from `terminator/` with a C compiler and Neovim 0.10+
on PATH, on a native desktop:

```sh
cargo build --workspace --bins --examples --features terminator/test-support --locked
cargo test --workspace --all-features --locked
cargo xtask gui reviews
```

Only these development instructions changed during the maintenance extraction;
the bundled runtime, pin, sources and license notices are unchanged.
