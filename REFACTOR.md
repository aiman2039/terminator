# Refactoring plan

Selective extraction. Draw boundaries around ownership, then move code. Navbar, sidebar, panes, terminal attachment, and shared chrome stay modules inside the app.

Each phase is its own change. Keep the Git CLI and the existing `NativePool` / `Supervisor` / `Processes` limits. A move that also swaps the implementation hides regressions.

`terminator` (`crates/app`) is a binary crate. `main.rs` holds startup, `struct App`, every module declaration, and the large test modules (`daemon_compatibility_tests`, `terminal_find_tests`, `layout_tests`, `navigation_tests`). Another crate cannot depend on it until those types live in a library. Moving files alone leaves the current dependencies in place.

## What is coupled today

- `workspace_ui::Viewer` holds `&mut App`, so every dock pane can reach the whole application.
- `sidebar_ui`, `settings_ui`, `worktree_ui`, `native_editor`, `player`, `exit`, `palette`, and `installation_ui` are `impl App` behind `use super::*`.
- `gui_services::Services` owns the daemon client, filesystem pool, platform pool, process pool, editor locks, snapshot, and egui context. `diff::git_async`, `services::context_async`, `services::directory_async`, and `markdown_images::Images` all take that object.
- Git command lines are copied in four places: `diff::git_async` (status, ignore, native diff), `metadata::git` / `git_async` (root and branch), `daemon::review::git` (CodeDiff byte snapshots), and `core::worktrees` (add / list / remove). Timeouts and stdout limits differ and stay different.

## Target

| Area | Where it lives | Owns |
| --- | --- | --- |
| Git CLI | New `terminator-git` crate, after the app seam exists | `git -C`, `GIT_OPTIONAL_LOCKS=0`, porcelain status, ignore checks, read-only blob and diff snapshots. Callers pass the existing process pool and their own `CommandOptions`. |
| Worktrees | Stay in `terminator-core` and `terminator-hook` | `worktree` add / list / remove, dirty / locked / live-session refusal, coordinator lock. |
| Markdown | New crate only after it can compile against a snapshot | Render a document snapshot, image cache, link actions. Neovim polling and the editor session stay in the app. |
| Native editor | Stay in `terminator-native-edit` plus `native_editor.rs` | Buffer, modal engine, and `show_source` are already in the crate. File load/save and dirty-close stay in the app. Neovim stays in the daemon. |
| Terminal | App module | GUI attach, paint, selection, search via `egui_term`. The daemon keeps the PTY. |
| Panes and workspaces | App module | Layout, focus, splits, drag, pane identity. Serialized layout versions stay compatible. |
| Header, sidebars, theme, icons | App modules | Paint explicit view data and return actions. |
| Player and browser | App modules until their handles are narrow | GUI-lifetime features. Extract only after they no longer need `Services` or `App`. |

## Phase 1 — Library target and explicit imports

Give the app a library so later phases have a crate boundary to stand on.

1. Add `src/lib.rs` and a `[lib]` target. Move the module tree and `App` there.
2. Leave `main` as startup only: preflight, `--data-dir` / `Paths`, crash install, `ui.lock`, `ensure_running`, `eframe`.
3. Move the four test modules with the code they exercise. Until that lands, `cargo test -p terminator --bin terminator` remains the way to run them. After it lands, the same tests run as `cargo test -p terminator --lib`.
4. Replace top-level `use super::*` one file at a time. Start with `gui_services`, `sidebar_ui`, `diff`, `services`, and `markdown_images`. `#[cfg(test)]` modules may keep the glob import.
5. Leave `struct App` as one struct. Splitting its fields is a later change, once callers already pass slices of state.

Done when a second crate can name an app type without depending on the binary, and those five files compile without `use super::*`.

## Phase 2 — Actions inside the app

Still one crate. This is the ownership change the Git crate needs.

The Git sidebar paints `services::ContextData` and returns the `FileAction` values that already exist (`NativeWorkingDiff`, `NativeStagedDiff`, `WorkingDiff`, `StagedDiff`, `Open`, `Text`, plus refresh). It also receives the diff-viewer preference and whether `nvim-review-v1` is advertised. `App` performs the action: open a diff, queue a refresh, or open an editor. The sidebar does not create sessions, write layouts, or call `Services`.

Apply the same shape to explorer rows, which already build `FileAction` in `file_actions.rs`.

`Viewer` keeps `&mut App` through this phase. The dock also creates tabs, splits panes, and closes sessions. Narrow it only after those paths return actions too. Header, settings, and player stay `impl App` until their inputs are as small as `ContextData`.

Done when the Git sidebar function's signature does not mention `App` or `Services`.

## Phase 3 — `terminator-git`

Own change, after phase 2. New package at `crates/git`, depending on `terminator-core`. No egui, no `App`, no daemon `State`, no `Session`.

Move:

- The command setup in `diff::git_async`: `git`, `-C`, `GIT_OPTIONAL_LOCKS=0`, `async_process::git_key`, `Processes::run`.
- Porcelain parsing, `Change`, `GitGroup`, decorations, and `status_description` used by `context_async` and the test-only `context_cached`.
- The `check-ignore --stdin -z` batching in `directory_async` and the test helper `entries_known`.
- The read-only snapshot sequence shared by `diff::document_async` and `daemon::review::snapshots`: `rev-parse`, `diff --raw -z`, `cat-file blob`, worktree file read, symlink and submodule refusal, 1 MiB bound, UTF-8 check.

Keep each caller's timeout and stdout limit. Metadata uses a 3 second / 64 KiB command. Status and diffs use a 4 MiB stdout limit. The helper takes `CommandOptions`; it does not pick one limit for everyone.

`metadata.rs` switches its `rev-parse` and `branch --show-current` calls to the helper. Pull-request lookup (`gh`) and listening ports (`lsof`) stay in metadata.

Stay outside the crate:

- `core::worktrees::{add,list,remove,ensure_unused}` and `terminator-hook ctl worktree`.
- `daemon::review::prepare`: Neovim probe, CodeDiff runtime directory, lock, and snapshot lifetime.
- syntect highlighting and `DiffDocument` layout. The crate returns bytes and parsed status. The app still builds the painted document.
- `Services`. Callers pass `&Processes` and the filesystem pool they already use to compute `git_key`.

Move the unit tests with the parser: ignore rules, porcelain renames, partial staging, symlink refusal. Daemon review tests and `cargo xtask gui reviews` stay the behavioral check.

## Phase 4 — Markdown, after the seam is real

Extract only when the new crate can compile from a document snapshot (`text`, path, revision, paused) and return rendered output plus the existing `markdown::Link` actions.

Stays in the app:

- `markdown::Source`, the editor socket, and the 350 ms poll (`nvim_get_mode`, then `nvim_exec_lua` / `markdown_snapshot.lua`).
- Session ownership, preview mode in `ui-preferences.json`, and unsaved-close.
- `Images` stops storing `Services`. It keeps the 32-image / 64 MiB cache and takes the filesystem/CPU submit handle it already uses.

If the crate still needs `Services`, `Session`, or `egui::Context` to build, it stays a module and the dependency shrink happens in the app first.

## Leave for later

- Terminal attachment, as a module. `egui_term` stays the widget. The daemon keeps the PTY.
- Pane and workspace layout, as a module. Do not bump the layout version and do not restart sessions. `unsupported_layout_version_is_rejected` and `media_layout_version_round_trips_and_preserves_legacy_tabs` cover that contract.
- Theme, spacing, buttons, menus, and icons. Extract a UI crate only when two crates actually paint with it.
- Player and browser, after their service and platform handles are as narrow as the Git runner. Both still die with the GUI.
- `terminator-native-edit`. It already owns the buffer, the modal engine, and egui rendering. Further editor work in the app is load, save, and dirty-close.

## Checks

Every phase:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p terminator --locked --bin terminator
```

Phase 1 replaces the last command with `cargo test -p terminator --locked --lib` once the test modules have moved.

Phase 3 also runs the moved Git unit tests and the existing daemon review tests. `cargo xtask gui reviews` when a desktop is available.

Full workspace `cargo test --workspace --all-features --locked` before calling a phase done.

## Out of scope

- Replacing the Git CLI.
- A runtime per feature.
- A crate per panel.
- Worktree mutation and session-safety checks.
- Layout migration.
- Editing `AGENTS.md`. After `terminator-git` exists, ask before adding it to the architecture notes.
