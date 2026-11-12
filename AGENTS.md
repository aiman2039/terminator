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

Key GUI modules include `workspace/` for project tabs/layouts, `files/editor_close.rs` for editor-close handling, `appearance/` for theme configuration, `preferences/` for navigation preferences, and `daemon_link/services.rs`/`daemon_link/refresh.rs` for filesystem and Git work. Size and structure limits live in `sarah.toml` (`sh scripts/check.sh sarah`). Read `docs/ARCHITECTURE.md` before changing GUI/daemon ownership and `docs/INTEGRATIONS.md` before changing agent hooks.

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

Use rustfmt defaults, four-space indentation, `snake_case` functions/modules, and `PascalCase` types. Keep IPC and lifecycle models in `core`; keep blocking work outside GUI rendering. Prefer existing native packages over custom rendering infrastructure. No Electron, Chromium, CEF, or Servo. GUI-only OS webview tabs (WKWebView / WebKitGTK via wry) are allowed. They die with the GUI, persist path/URL in layout, use an isolated data-dir profile, and load local HTML files plus http(s) only. Hide the native view when the pane is covered. No JS bridge into the app.

Keep every toolbar icon on one row and one size. Use the shared `appearance` helpers (`sidebar_action`, `selectable_icon`, `icon_menu_button`, `compact_menu_button`, `text_menu_button`; in the window header `framed_icon`, `framed_icon_button`; glyph plus text `icon_label_button`) so frameless controls keep a fixed square regardless of ambient `interact_size`; never mix a framed or differently sized button beside them. Sizes are `TOOLBAR_ICON` (glyph), `TOOLBAR_BUTTON` (compact rows) and `HEADER_BUTTON` (window header). A menu item shows a shortcut only when that key runs that exact item. Tool windows open from the stripes only; the window header holds window controls (Settings, IDE mode, sidebar toggles) and the centre Search. Diff stage verbs have one glyph each: Stage Plus, Unstage Minus, Discard Eraser, Undo Undo2.

Center-pane pages (Settings, SSH, VNC, worktree form) paint in `appearance::center_card`: a margin on each side, a click on the margin closes the page, and content scrolls inside the card. Right-aligned form footers use `appearance::footer_row`, not `with_layout(right_to_left)`.

Take text sizes and corner radii from `appearance/scale.rs` (`FONT_*` and `RADIUS_*` tokens, or `egui::CornerRadius::ZERO`). Do not write a literal font size or corner radius in `crates/app/src`; `appearance::scale_tests` fails on one. Terminal and editor text follow the user's size settings; give any other exception a reason in that test's allowlist.

## Architecture & Behavior Constraints

- The daemon owns persistent PTYs, shells, and editors; the GUI attaches to them. Closing the GUI must leave sessions running. Historical sessions are not automatically restarted.
- Each project owns top-level tabs, each with its own split layout and focus. Preserve the originating project/tab for asynchronous creation. Layout JSON is versioned; do not overwrite unknown versions or restart sessions during migration.
- Opening a supported image creates a GUI-only preview tab without a PTY; explicit Open as text bypasses preview. Opening a text file creates a new editor session in a new top-level tab; explicit editor-split actions stay in the current tab. Navigation from a native editor (`gf`, `:e file`, `:e#`, Ctrl-^, go to declaration, Back/Forward) works like JetBrains: the target opens as a tab in the focused pane, or its existing tab in the project is focused. Explorer, palette and Open file keep the new top-level tab. Clean file-only tabs close directly. Unsaved buffers offer Save and close, Discard changes, or Cancel on an in-file bar; unknown editor state must not silently discard changes. The rest of the GUI stays usable while that bar is visible. Shell/active-agent tabs retain the background-or-terminate choice.
- Explorer (and palette / Open file) always uses the file's default viewer: image → preview, html → browser, markdown → editor preview, .mmd/.mermaid → diagram preview (pan/zoom), csv/tsv → table viewer (read-only `Tab::Document`, no PTY), audio → chrome player. Git sidebar click on a modified, untracked, or deleted file opens the diff. Context "Open file" still uses the default viewer. Open as text bypasses special viewers.
- Opening a supported HTML file or an http(s) URL creates a GUI-only Browser tab without a PTY (layout v6). WKWebView/WebKitGTK via wry; dies with the GUI; isolated data-dir profile. Hide the native view when the pane is covered. Open as text bypasses it. Zoom, find (GUI-initiated `window.find`) and devtools (Settings, default off) are allowed; still no JS bridge.
- Opening a supported audio file plays in the GUI chrome player (icon next to the project-sidebar Agents bell). Click the icon or playlist control to open a single global Player view in the main pane between the sidebars (same pattern as Settings; not a popup and not a workspace tab). Chrome live controls can be hidden; the icon expands them while audio is playing. Opening it again shows the same instance; leftover Player tabs are closed. Switching to a workspace tab, new terminal, or file/session hides the view without destroying it. Open as text bypasses the player. Named playlists are global; opening a file appends to the selected playlist (Default is created on first file, and first Player open seeds bundled tone files if Default is empty). One engine; radio shares it. Playback stops when the GUI exits. Hiding the Player view does not stop audio. The player is native egui, using the app theme and Lucide icons. Spectrum is an FFT of the playback tap (`spectrum-analyzer`). Playlist footer: ADD/REM/SEL/MISC/LIST. Shuffle and repeat persist. Radio mode lists the bundled Icecast/Shoutcast catalog (search + category) plus user-added stations.
- OS notification sound is Settings → Notifications (default on). In-app Agents inbox stays silent. Gate the checkbox on notification-sound-v1. A newer alert replaces the pending OS notification immediately. An unanswered alert is removed after 10 seconds so the daemon stops polling Notification Center; a click during that wait still focuses the session. The daemon clears only its own notification.
- macOS menu bar: while the GUI is running, a status item shows the Terminator icon. Waiting agents draw a red badge with the count (Telegram red `#FF3B30`, white digits, `99+` above 99). Clicking the item opens a menu of pending agent notifications; picking one focuses that agent's terminal, and Show Terminator brings the window forward (opening the Agents inbox when an agent is waiting).
- On Linux the same menu and count drive a StatusNotifierItem tray icon (ksni, async-io) that exists only while the GUI runs; without a StatusNotifierWatcher (GNOME without AppIndicator) it fails quietly.
- OSC window titles (`terminal-title-v1`) are display only: never a lifecycle, notice or notification source. The pane Resume chip types the resume command only on an explicit click. When verified presence finds no live agent, the click also presses Enter. When presence is unverified (for example an older daemon), the chip shows "?" and types the command without Enter. When verified presence finds a live agent, the chip is hidden.
- Ordinary editors load the user's Neovim configuration. Git reviews use bundled CodeDiff with an isolated profile and read-only snapshots: HEAD to index for staged changes, index to disk for working changes. Reopening refreshes a review; review actions must not mutate Git state.
- Exception: native Git diff controls may stage, unstage, or discard selected hunks on explicit user action. Discard requires confirmation. Neovim reviews remain read-only.
- Gate new daemon requests on advertised capabilities. Git reviews default to the native GUI viewer. Neovim CodeDiff is Settings → Diff viewer, and still requires `nvim-review-v1`. Missing that capability falls back to native. A newly built GUI or package does not replace a running daemon; preserve compatibility with live older daemons.
- VNC: the daemon owns connections, framebuffers, keychain passwords and saved-host thumbnails. Closing a VNC tab detaches the session (like a terminal); only Disconnect, the detached timeout (Settings, default 30 min) or "Disconnect all and quit" ends it. Connecting a host with a live session reattaches it. Never show VNC records of retired or unreachable generations.
- The VNC tool opens the VNC page in the main pane (same pattern as Settings/Player): address book, host Properties, import. The VNC sidebar only lists sessions and hosts; do not add editing forms there.
- SSH: host Properties and import open in the main pane (like VNC); the Hosts sidebar only lists hosts. Editing a host's connection is refused while it has live sessions, transfers or forwards (`ssh-edit-v1`).
- Mosh: host Properties "Use Mosh for terminals" (`ssh-mosh-v1`) runs new terminals/editors via `terminator-hook mosh`: OpenSSH (shared master) runs the host's Mosh server command on the host (default `mosh-server new …`; editable and saved per host with `ssh-commands-v1`; Terminator adds `-- sh -c <script>`), then the helper execs local `mosh-client` with `MOSH_KEY`. Never install anything on the host. Explorer, Git, editor control and transfers stay on OpenSSH. Gate the checkbox on the capability.
- Terminal command: host Properties "Terminal command" (`ssh-commands-v1`) runs on the host after `cd` for new SSH and Mosh shell terminals (for example tmux). Blank or default saves nothing. Editors ignore it. The local `ssh` arguments stay fixed.
- Never read or write the OS keychain inside an IPC request with a short deadline; load saved passwords on the session thread. Never send an empty VNC password.
- The VNC start command is user-entered and runs over SSH only before connecting; never generate or install server-side scripts.
- JetBrains keymap (default on): double Shift, Find Action (Cmd+Shift+A) and Go to File (Cmd+Shift+O; Linux Ctrl+Shift+N) work in every pane, a focused terminal included, and never reach the PTY. Cmd+E is Recent Files; Toggle IDE mode is Cmd+Alt+E. On Linux a focused terminal keeps Ctrl+E, Ctrl+Shift+A, Alt+letter and Alt+digit. When a default chord changes, migrate only saved bindings still at the old default, once, and never take a chord that another action uses.
- Tool window keys (Cmd+1 Explorer, Cmd+6 Problems, Cmd+7 Structure, Cmd+9 Git, Alt+F12 Terminal, Alt+F1 Select In; Alt+digit on Linux): hidden → show and focus; shown → focus; focused → hide. A new sidebar list that takes keyboard focus calls `shortcuts::take_tool_focus` on its first row. Esc in a tool window goes back to the last editor or terminal and never reaches a PTY.
- Native editors have no header row. The status bar shows the focused editor's path and caret symbol as breadcrumbs. Markdown Edit/Preview/Split float in the editor's top-right corner. Reload from disk, Bookmark file, Recent files and Local History are in the pane tab menu. Unsaved changes show only as a dot in place of the tab's close button.
- Native editor indentation is one setting per document (`Buffer::indent_style`): `.editorconfig` (ec4rs, read in the load job, never on the GUI thread), then detection from the text, then tabs for Go and Makefiles, then `~/.vimrc`, then 4 spaces. Do not hard-code 4. Remote snapshots do not read `.editorconfig`. Save rules apply only when set to true, keep caret lines, wait while an undo group is open, and make one undo step.
- Replace in Files is GUI-only and writes through the native rename staging off the GUI thread: every file or none. An unsaved, loading or saving buffer, a read-only file, or a file that changed since the search stops it before any write.
- Git panel Commit (also amend) and Commit and Push run as cancellable GUI jobs: 30-minute cap, one at a time. Cancel sends SIGINT to the Git process group, then SIGKILL after 5 seconds. All other Git panel calls keep the 20-second cap. Do not add new Git mutations to this path. While a Git panel commit runs, every action that can change it in its repository goes through `App::refuse_during_commit` with `CommitRisk`: index and stash writers wait until the push starts, and HEAD writers wait until the push ends. A refused action sends no job and shows a status note. Its control is disabled with "Wait for the commit to finish". Index writes from diff tabs and the gutter use the repository's `workspace-op` queue, so they land before a later commit starts.
- Create PR (Git panel) runs `gh pr create --fill` or `glab mr create --fill --yes`, picked by the `origin` host, and only on an explicit click. It is a link on the readiness line, shown only when the branch has an upstream, has commits that `origin`'s default branch lacks, and no pull request is known; a known one (PR tab or session metadata, never a new call) shows as `PR #n`. It runs as a quick Git panel call with the 20-second cap, never in the commit job. The title and body come from the pushed branch's commits. The CLI never prompts. The GUI shows the created URL or the CLI error.
- The Git panel shows a readiness line above the Commit buttons that counts the staged set (`1 of 3 files staged  +2 −1`); it shows line counts only when the refresh carried diffstats and every staged file is staged whole, else file counts only. With nothing staged it shows `Nothing staged` and a Stage all link.
- Compare with… is read-only and opens the native revision diff. A typed revision that starts with `-` or has spaces is refused, and it must pass `git rev-parse --verify <rev>^{commit}` before a diff opens.
- Commit Log filters and Go to Hash run local `git log` as replaceable background jobs, never a daemon request. Typed values never reach Git as options: use `--grep=`/`--author=` values, full ref names, and paths after `--`.
- A missing project folder (moved or deleted, for example a removed Git worktree) shows "Folder not found" with Locate Folder…, Remove Project and Retry, not the Privacy hint. Locate Folder… opens the picked folder as a project. The daemon has no relocate request; a new one needs a new capability and this fallback. Never run `git worktree prune` or another Git command for this. Every folder pick gives a visible result.
- New task makes a worktree, a terminal and an optional Browser tab. It types the agent command but never presses Enter.
- Resume conversation and Ask agent's Paste and send run text only on an explicit click.
- Per-project init command (`project-init-v1`) runs in new local shell terminals only; editors, reviews and SSH terminals skip it.
- Fetch, Pull and Push run as their own cancellable GUI job (not the commit job), one at a time per repository, with a 5-minute cap; Cancel sends SIGINT to the Git process group, then SIGKILL after 5 seconds.
- Git panel branch actions (New/Rename/Delete branch, Merge, Rebase) and Commit Log commit actions (Copy hash, New branch from commit, Checkout revision, Cherry-pick, Revert) run only on an explicit click as quick Git panel calls with the 20-second cap, never in the commit job. Each is a HEAD writer for `refuse_during_commit`. Delete uses `git branch -d`; force delete needs a second explicit choice. Typed names never reach Git as options: new names pass `git check-ref-format`, branches go as full ref names, commits as full hashes.
- The Git panel Pull request view is read-only: `gh pr view`/`gh api graphql` or `glab mr view`/`glab api`, picked by the `origin` host, as a replaceable background read with the 20-second cap, only with Settings → Fetch PR metadata on and only when the view shows or Refresh is clicked. No merge or auto-merge.
- A Git panel row click opens its diff in one reused preview tab (italic title, GUI memory only, layout JSON unchanged); a double click or Open changes keeps it; pinned tabs and diffs with unsaved edits are never replaced.
- Draft commit message pastes a fixed prompt into a live agent through Ask agent; only Paste and send presses Enter.
- Native diff review notes are GUI-only (lost on GUI exit) and never change Git. Send opens Ask agent; Paste types one line without Enter, only Paste and send presses Enter.
- New task saves its task base (start commit and Start-from name) in `ui-preferences.json` `task_bases`. Worktree row counts and Changes since task start read from it (fallback: merge base with the main checkout's branch). Read-only Git jobs only.
- New task copies ignored files named in the source checkout's `.worktreeinclude` (GUI job, never follows links, never overwrites). The per-project setup command is a GUI preference, typed as `setup && agent` without Enter.
- Perforce Sync, Submit, Shelve and Unshelve run as cancellable GUI jobs, one at a time: 30-minute cap; Cancel sends SIGINT to the p4 process group, then SIGKILL after 5 seconds. Read queries keep their short caps. Submit keeps its confirmation.
- Local History keeps at most `local_history_file_entries` snapshots per file and prunes the oldest entries until the store fits `local_history_mib` (`local-history-limits-v1`). 0 days keeps all snapshots by age. Timelines are read with LocalHistoryPage when advertised, else LocalHistoryList.
- The ntfy access token lives only in the OS credential store. The daemon's ntfy worker thread reads and writes it, never an IPC handler. curl gets it on stdin, never in argv or a file. Notice text in pushes is opt-in (`ntfy-server-v1`).

## Testing Guidelines

Name tests after observable behavior, such as `dismissal_does_not_resume_agent`. Add focused regressions for lifecycle, identity, persistence, and input-handling changes. Use isolated temporary state and fixture hooks. Distinguish unit tests, real-PTY checks, native rendering, and live-provider verification; record material evidence in `docs/VALIDATION.md`.

## Commit & Pull Request Guidelines

History currently contains terse subjects such as `nice`, `ok`, and `init`; no consistent convention is established. Prefer descriptive imperative subjects, for example `Fix terminal reconnect after GUI exit`. PRs should explain behavior changes, link relevant issues, list validation, and include screenshots for UI changes. Preserve unrelated edits.

## Configuration & Session Safety

Use `TERMINATOR_DATA_DIR` for isolated development. Never commit authentication files, credentials, runtime databases, or scrollback. Do not terminate live daemon sessions merely to reload code. Agent launching and Git mutations remain user-driven terminal operations. The `terminator-hook ctl worktree` commands are explicit Git operations; they must reject removal of dirty, locked, or live-session checkouts.

Fatal panics: after data-dir init, each binary installs a panic hook. Main-thread panics write `$data/crashes/{binary}-{timestamp}.log` (0600, last 20) with message, location, thread, version, and backtrace. The GUI shows a native dialog with that path and exits. It does not resume. Recovered worker panics stay Failure::Panicked and do not write dumps. Segfaults/aborts are not caught.

Appearance lives in `~/.config/terminator/config.toml`, honoring `XDG_CONFIG_HOME` and `TERMINATOR_CONFIG_DIR`; explicit data-directory installations keep it in their isolated data directory. Preserve comments and unrelated TOML keys when saving. SQLite holds functional/session state, `ui-preferences.json` holds navigation/sidebar preferences, and scrollback is stored separately.

Hook installation is an explicit Settings action that preserves unrelated configuration and creates backups. Tests must use isolated hook fixtures rather than changing user agent configurations. Never infer agent lifecycle state from terminal text or automatically launch agents.

## Updates
- We should have "check for updates" button in the menu
- Periodically check for updates once per minute

## AI 
- Always be brief and concise, if the user need more info they will ask for you to elaborate more
- Always opt first for known popular packages and dont reinvent the wheel
- Always ask for specific permission and show what you want to chage in AGENTS.md

