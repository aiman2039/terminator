# Repository Guidelines

## Project Structure & Module Organization

The native macOS/Linux, a Rust 2024 workspace requiring Rust 1.97.1+. Root documents such as `plan.md` describe product requirements; consult `README.md` and `docs/` for implementation details and validation limits.

`crates/` contains:

- `app`: native GUI, docking, file dialogs, and filesystem/Git services.
- `core`: shared models, settings, IPC, and lifecycle transitions.
- `daemon`: persistent PTYs, SQLite, scrollback, notifications, and editor ownership.
- `integrations`: agent hook installers and event normalization.
- `hook`: hook delivery and terminal attachment executable.

Rust tests are colocated in `#[cfg(test)]` modules. Rust integration tests and packaging tasks live in `crates/xtask/`; GUI fixtures include `crates/app/examples/layout.rs`.

Key GUI modules include `workspace.rs` for project tabs/layouts, `editor_close.rs` for editor-close handling, `appearance.rs` for theme configuration, `preferences.rs` for navigation preferences, and `services.rs`/`refresh.rs` for filesystem and Git work. Read `docs/ARCHITECTURE.md` before changing GUI/daemon ownership and `docs/INTEGRATIONS.md` before changing agent hooks.

Vendored dependencies include `vendor/egui_term/` (terminal widget), `vendor/egui_dock/` (docking), and `vendor/codediff.nvim/` (Neovim Git reviews). Document changes in the corresponding `UPSTREAM.md` and retain license notices.

## Build, Test, and Development Commands


```sh
cargo build --workspace --locked
cargo run --bin terminator --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
```

Build the workspace before launching so `terminator`, `terminator-daemon`, and `terminator-hook` exist alongside one another. Install Neovim for the default editor; bundled CodeDiff reviews require Neovim 0.10+ on PATH and a C compiler at build time. `cargo xtask integration` checks real PTYs and recovery. For native smoke testing:

```sh
cargo build --workspace --bins --examples --features terminator/test-support --locked
cargo xtask gui smoke --sessions 50 --seconds 5
```

`cargo xtask package` creates the platform package; `--debug` creates a development build.

After the test-support build, use `cargo xtask gui CASE` with `workspace-tabs`, `editor-lifecycle`, `file-close`, `inline-rename`, or `focus-editor-close`. `cargo xtask gui reviews` exercises Neovim reviews and native menu actions; `cargo xtask gui legacy-diff` checks fallback with an older daemon. `images`, `control`, and `window-controls` cover native previews, CLI controls, and platform window/picker interactions. See `scripts/README.md` for the complete task mapping. Native fixtures require a desktop; inspect script arguments and prerequisites before running. Local macOS packages are signed ad hoc, not notarized or published.

## Coding Style & Naming Conventions

Use rustfmt defaults, four-space indentation, `snake_case` functions/modules, and `PascalCase` types. Keep IPC and lifecycle models in `core`; keep blocking work outside GUI rendering. Prefer existing native packages over custom rendering infrastructure. No Electron, Chromium, or webview.

## Architecture & Behavior Constraints

- The daemon owns persistent PTYs, shells, and editors; the GUI attaches to them. Closing the GUI must leave sessions running. Historical sessions are not automatically restarted.
- Each project owns top-level tabs, each with its own split layout and focus. Preserve the originating project/tab for asynchronous creation. Layout JSON is versioned; do not overwrite unknown versions or restart sessions during migration.
- Opening a supported image creates a GUI-only preview tab without a PTY; explicit Open as text bypasses preview. Opening a text file creates a new editor session in a new top-level tab; explicit editor-split actions stay in the current tab. Clean file-only tabs close directly. Unsaved buffers offer Save and close, Discard changes, or Cancel on an in-file bar; unknown editor state must not silently discard changes. The rest of the GUI stays usable while that bar is visible. Shell/active-agent tabs retain the background-or-terminate choice.
- Ordinary editors load the user's Neovim configuration. Git reviews use bundled CodeDiff with an isolated profile and read-only snapshots: HEAD to index for staged changes, index to disk for working changes. Reopening refreshes a review; review actions must not mutate Git state.
- Gate new daemon requests on advertised capabilities. Git reviews default to the native GUI viewer. Neovim CodeDiff is Settings → Diff viewer, and still requires `nvim-review-v1`. Missing that capability falls back to native. A newly built GUI or package does not replace a running daemon; preserve compatibility with live older daemons.

## Testing Guidelines

Name tests after observable behavior, such as `dismissal_does_not_resume_agent`. Add focused regressions for lifecycle, identity, persistence, and input-handling changes. Use isolated temporary state and fixture hooks. Distinguish unit tests, real-PTY checks, native rendering, and live-provider verification; record material evidence in `docs/VALIDATION.md`.

## Commit & Pull Request Guidelines

History currently contains terse subjects such as `nice`, `ok`, and `init`; no consistent convention is established. Prefer descriptive imperative subjects, for example `Fix terminal reconnect after GUI exit`. PRs should explain behavior changes, link relevant issues, list validation, and include screenshots for UI changes. Preserve unrelated edits.

## Configuration & Session Safety

Use `TERMINATOR_DATA_DIR` for isolated development. Never commit authentication files, credentials, runtime databases, or scrollback. Do not terminate live daemon sessions merely to reload code. Agent launching and Git mutations remain user-driven terminal operations. The `terminator-hook ctl worktree` commands are explicit Git operations; they must reject removal of dirty, locked, or live-session checkouts.

Appearance lives in `~/.config/terminator/config.toml`, honoring `XDG_CONFIG_HOME` and `TERMINATOR_CONFIG_DIR`; explicit data-directory installations keep it in their isolated data directory. Preserve comments and unrelated TOML keys when saving. SQLite holds functional/session state, `ui-preferences.json` holds navigation/sidebar preferences, and scrollback is stored separately.

Hook installation is an explicit Settings action that preserves unrelated configuration and creates backups. Tests must use isolated hook fixtures rather than changing user agent configurations. Never infer agent lifecycle state from terminal text or automatically launch agents.

## Updates
- We should have "check for updates" button in the menu
- Periodically check for updates once per minute

## AI 
- Always be brief and concise, if the user need more info they will ask for you to elaborate more
- Always opt first for known popular packages and dont reinvent the wheel
- Always ask for specific permission and show what you want to chage in AGENTS.md

