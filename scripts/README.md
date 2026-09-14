# Rust development tasks

## Quality checks

Run `sh scripts/check.sh` for formatting, compiler checks (`lint`), build, and
Clippy with warnings denied across all workspace targets and features. Run
`sh scripts/check.sh test` for workspace tests. Individual checks accept `fmt`,
`lint`, `build`, or `clippy`. Checks use the lockfile and do not rewrite files.

[cargo-husky](https://github.com/rhysd/cargo-husky) installs the tracked
`.cargo-husky/hooks/pre-commit` when its development dependency is first built
(for example, `cargo test -p terminator --no-run --locked`). The hook runs
`sh scripts/check.sh` against the working tree; stage any fixes before committing.
Existing non-cargo-husky hooks are preserved. CI skips hook installation and runs
all checks plus tests on native macOS and Linux for pushes and pull requests to
`master`, or via manual dispatch. CI uses Rust 1.95.0.


Project-owned Python automation has moved to `crates/xtask`. Run commands from
`terminator/`; no Python interpreter is used by these tasks.

| Former script | Rust command |
| --- | --- |
| `package.py` | `cargo xtask package [--debug]` |
| `integration.py` | `cargo xtask integration` |
| `gui_smoke.py` | `cargo xtask gui smoke --sessions 50 --seconds 5` |
| `workspace_tabs_smoke.py` | `cargo xtask gui workspace-tabs` |
| `editor_lifecycle_smoke.py` | `cargo xtask gui editor-lifecycle` |
| `file_close_smoke.py` | `cargo xtask gui file-close` |
| `inline_rename_smoke.py` | `cargo xtask gui inline-rename` |
| `focus_editor_close_smoke.py` | `cargo xtask gui focus-editor-close` |
| `ui_cleanup_smoke.py` | `cargo xtask gui ui-cleanup` |
| `external_editor_smoke.py` | `cargo xtask gui external-editor` |
| `review_smoke.py --gui` | `cargo xtask gui reviews` |
| `legacy_diff_smoke.py` | `cargo xtask gui legacy-diff` |
| `ui_flat_smoke.py`, `ui_plan3_smoke.py` | `cargo xtask gui terminal-actions` and `cargo xtask gui ui-cleanup` |
| `integration.py --load-seconds N` | `cargo xtask load --seconds N --output report.json` |
| `compare_load.py --conditional` | `cargo xtask load --conditional --seconds 30 --output report.json` |
| before/after load reports | `cargo xtask compare-load before.json after.json` |
| `command_counts.py` | `cargo xtask command-counts --seconds 6 --output counts.json` |
| `muse_echo.py --muse PATH` | `cargo xtask muse-echo --muse PATH` |

New cases: `cargo xtask gui images`, `cargo xtask gui control`,
`cargo xtask gui window-controls`, `cargo xtask gui split-file-opening`,
`cargo xtask gui markdown`, `cargo xtask gui markdown-busy`, and `cargo xtask browser-check`.
`cargo xtask gui project-sidebar` covers project removal and restoration.
`cargo xtask gui agent-sidebar` covers the bell, waiting/unread counts, left Agents navigation, restart persistence, and live status updates.
`cargo xtask gui all` runs the ordinary native fixture suite. All GUI cases accept
`--scale 1|2`, `--narrow`, and `--output PATH`. Captures default to the Cargo target
validation directory; earlier committed screenshots are not overwritten.

`split-file-opening` reproduces unavailable working directories after a project
move, checks the error and unchanged session inventory, then restores a path
alias. Real native menu clicks verify all four split directions, editor tab
placement, double-click deduplication, and normal/split file-menu actions while
preserving the original shell PID. It runs as part of `gui all`.

`markdown` opens a real Neovim file and exercises Edit/Preview/Split, unsaved
typing, preview focus, per-file buffer identity, GUI restart, and dirty-file
cancel/save-close behavior. It checks editor/shell PIDs and disk bytes, and runs
as part of `gui all`. Use `--scale 2 --narrow` for the compact Retina layout.
`markdown-busy` opens Neovim at a real pager prompt, verifies that Preview still
renders the saved file, then checks that live rendering resumes on the same PID.
The regular Markdown fixture also tests unsaved text through a prompt and Refresh,
Preview on the initial file click, and persistence of an explicit Edit choice.
It checks that filename, view tabs and refresh icon share one row without overlap.
`project-sidebar` removes/restores active projects and tests an empty sidebar
across GUI restarts while retaining the same shell/editor PIDs, unsaved buffers,
file bytes and project layouts. Both cases run as part of `gui all`.

Build first with Rust 1.95+ and a C compiler. Editor/review fixtures require
Neovim 0.10+ on PATH, Git, and a native desktop. Run GUI cases serially:

`cargo xtask integration` also covers worktree use across projects and descendant
processes, large notification snapshots and restart recovery, stalled attachment
cleanup with original-PID reconnect, and terminal-editor argument fixtures.


```sh
cargo build --workspace --bins --examples --features terminator/test-support --locked
cargo xtask gui all
cargo xtask gui ui-cleanup --scale 2
```

The native window/picker test checks macOS event-posting permission before doing
anything, or requires `TERMINATOR_X11_TEST=1` in an isolated Xvfb/Openbox display.
It targets the fixture GUI; macOS native mouse gestures are bounded to its verified
foreground window and use a non-executing, non-echoing PTY. In ordinary
macOS fixtures, the GUI starts inactive and desktop input is filtered out.

`cargo xtask linux-check --browser --wayland` is restricted to a disposable Linux
container. It installs test dependencies, including an external test browser,
and uses Xvfb/Openbox and headless Weston. The application package does not bundle
that browser. The container source can be mounted read-only; use `CARGO_TARGET_DIR`
and `CARGO_HOME` for writable caches, and Docker `--init` for correct child signals.

`TERMINATOR_TEST_BIN_DIR` optionally chooses a separate binary directory for
before/after measurements. Provider echo testing is opt-in and requires a supplied
Muse executable. Historical validation reports retain their original command names.
