# Terminator setup and reference

[Back to the overview](../README.md).

A native Rust terminal workspace for macOS and Linux. Projects have independent split layouts; a local daemon keeps shells and editors alive when the GUI closes. Agent hooks feed a configurable attention bar.

## Build and run

Requires Rust 1.97.1+, a C compiler for the bundled CodeDiff library, and a native desktop. Install Neovim for the default embedded editor; Git reviews require Neovim 0.10+ on PATH.

```sh
cd terminator
cargo build --workspace --locked
cargo run -p terminator --locked
```

Build all three executables together: `terminator`, `terminator-daemon`, and `terminator-hook`. The GUI starts the daemon if needed. Keep the executables beside one another.

GitHub release automation builds macOS (Apple Silicon) and Linux
(x86-64 and ARM64) archives. Intel Macs are source-build only and unvalidated.
See [Releasing](RELEASING.md) for triggers, downloads, signing, and
compatibility limits.

On macOS, build a local app bundle:

```sh
cargo xtask package
open target/package/Terminator.app
```

`--debug` produces a faster development bundle. Bundles are signed ad hoc locally; they are not notarized or published. On Linux, the same task produces a relocatable archive and desktop entry. X11 and Wayland backends are compiled; native file selection uses the desktop's XDG portal on Linux and NSOpenPanel on macOS.

Debian/Ubuntu desktop runtime libraries include `libxkbcommon-x11-0`, `libxkbcommon0`, `libgl1`, and the usual X11/Wayland desktop libraries. Linux compilation also needs `pkg-config` and `libfontconfig1-dev` so resvg/blitz can use system fonts, plus the matching X11/Wayland/GL headers. Install an XDG desktop portal backend appropriate to your desktop for file dialogs. `cargo xtask linux-check` configures a disposable Linux test container; `scripts/README.md` lists the Rust validation tasks.

## Daily workflow

- Add a project through the native folder picker. File and folder pickers start at the focused terminal's current directory, falling back to the selected project and launch directory.
- Open a terminal. The automatic shell preference is **zsh → bash → sh**, selecting the first installed executable. Settings accepts an explicit shell override; clear it to restore automatic selection.
- Use **+** in the top-level tab strip to create a terminal tab with its own split layout. Explorer hides Git-ignored entries by default; **Show ignored files** reveals them (including Git metadata). Ordinary dotfiles remain visible.
- Right-click a terminal or its tab for **New tab**, **Split up**, **Split down**, **Split left**, and **Split right**. Each split stays inside its owning top-level tab.
- Launch agents and manage worktrees in the terminal yourself. The app does not launch agents or perform Git writes.
- Project arrows expand/collapse independently of selection and persist across restarts. **Sort** beside PROJECTS orders visible projects by name A → Z (default), name Z → A, or latest activity; the choice persists in `ui-preferences.json`. Latest activity is the latest of session creation, agent updates, notice times, and restoring, adding, or creating a project. Selecting a visible project does not change its rank. The window header holds project tabs and the Explorer, Agents, Git, and Settings icons. Explorer, Agents, and Git switch the right sidebar; click the active tool to collapse it. The left Agents row stays compact (waiting/unread counts) until opened. Explorer, Git, and History do not list attention events. Agents lists undismissed, unsnoozed notifications for the owning project or All projects, including retained events from ended sessions and terminal notices. Unresolved waiting events appear first. Waiting input and permission notices leave the inbox when the agent continues or a newer request replaces them; completed and failed notices remain until the session is focused or the notice is dismissed. The Agents header waiting badge uses that same list, not live agent sessions. Working agents without notifications do not create cards. Sidebar width and scope persist globally. Settings → Place notifications at the side hides the top Attention strip (default); unchecking it restores the top strip.
- Select a project to restore its own layout. A terminal that changes directory stays under its owning project; the file/Git sidebar follows its effective directory.
- Single-click anywhere on an Explorer file row to open a new editor tab. Right-click a file path to open the editor, a new editor split, or an external editor. `command+O` uses the native file picker (Cmd+O on macOS, Ctrl+Shift+O on Linux). Change it in Settings → Shortcuts. Settings shortcuts are consumed by the GUI and are not typed into the terminal. Other Command/Ctrl chords go to the focused terminal as kitty CSI u (Command is Super, so Neovim `<D-e>` works); Ctrl+A–Z stay control characters, and Cmd/Ctrl+C/V copy and paste.
- Settings → Terminal & Editor retains embedded Neovim, terminal-editor, and external-editor modes. External presets include System default, VS Code, Cursor, RustRover, Zed, and Custom. Named presets use macOS application launching or Linux CLI launchers. Custom takes an executable and one literal argument per row; the absolute file path is appended without shell evaluation. **Choose file and test…** launches the draft without saving. Missing launchers and failed exits appear in the status bar; long-running editors remain independent of the GUI.
- The default editor is real Neovim, rendered inside the native terminal widget and controlled through Neovim RPC for save/compare actions. Your Neovim configuration and plugins load normally. An editor-side tree can therefore come from your own configuration.
- Closing a tab with live shells or agents asks whether to terminate or background them. Clean file-only tabs close directly. Closing the GUI leaves the daemon and sessions running.
- After a reboot or daemon loss, historical tabs show retained output and known resume commands. Agents are never restarted automatically.

## Agent setup

Settings → Agent hooks installs, repairs, and removes managed hooks for Claude Code, Codex, OpenCode, Muse, and Grok. Installation preserves unrelated hooks and writes a backup before changing an existing configuration. It never installs agent binaries or configures accounts.

Read [the integration contract and capability notes](INTEGRATIONS.md) before connecting a custom agent. Installed hooks are inert outside terminals owned by this application. Notification defaults cover input, permission, completion, and failure events where the agent exposes them; opening the relevant terminal dismisses the notification without changing agent state. Settings → Notifications can play a system sound with desktop banners (default on). The in-app Agents inbox stays silent. The control requires a daemon that advertises `notification-sound-v1`.

## Local state and updates

The default data directory follows the OS application-data convention. `TERMINATOR_DATA_DIR=/absolute/path` or `terminator --data-dir /absolute/path` selects an isolated installation. The runtime directory contains a private Unix socket and authentication file; do not share these files.

SQLite stores projects, layouts, session metadata, agent state, and notification state. `ui-preferences.json` stores versioned per-installation navigation/sidebar choices, project sort/activity, player playlists/volume/custom radio stations, and the one-time typography and Attention migration markers. Attention migration turns on side placement once after the daemon acknowledges the settings update; later placement choices are preserved. Side placement keeps the compact Agents row as the persistent indicator and does not overlay Git, Explorer, or History. The Islands update sets terminal/editor size to 13 once; subsequent user size choices are preserved. Inter and JetBrains Mono are bundled with their licenses. Scrollback is stored separately and pruned by configurable age/per-session/total limits. Defaults: 30 days, 50 MiB/session, 2 GiB total. Metadata and resume commands remain until explicitly removed. Truncated output is labeled.

A running daemon keeps its current executable version until it exits. GUI updates
reconnect to compatible daemons, whose private terminal helpers survive app
replacement or removal. If an older installation reports a missing helper, use
**Fix installation… → Settings → Updates → Installation**. Save your files and
finish the listed sessions, then choose **Repair installation**. Backgrounded
sessions still count as live. Repair preserves history and never relaunches ended
sessions. For the oldest daemons, the screen provides a **Copy shutdown command**
button: finish all sessions, copy it, fully quit Terminator, then run it in
Terminal.app or another terminal application. After `"Ok"`, wait two seconds and
reopen Terminator. Logout/login is an alternative. Do not kill a daemon with live
work merely to reload a build.

## Validation

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --bins --examples --features terminator/test-support --locked
cargo xtask integration
cargo xtask gui all
cargo xtask gui ui-cleanup --output /tmp/terminator-ui-2x --scale 2
```

The `test-support` feature enables opt-in renderer capture and synthetic input only when the test environment requests them. Normal builds do not include that code. Tests use temporary directories and do not install hooks into your agent configurations.

See [unreleased changes](../CHANGELOG.md), [panic audit](PANIC_AUDIT.md), [architecture](ARCHITECTURE.md), [validation evidence](VALIDATION.md), and [upstream widget changes](../vendor/egui_term/UPSTREAM.md). Native desktop notification permissions/actions, Wayland desktop behavior, and all live provider-specific event combinations require separate desktop/provider validation; a build or synthetic hook fixture does not establish them.

## Appearance and navigation

Appearance is stored as a versioned `[appearance]` table in
`~/.config/terminator/config.toml` on macOS and Linux. `XDG_CONFIG_HOME` replaces
`~/.config`; `TERMINATOR_CONFIG_DIR` overrides the configuration directory.
Explicit `--data-dir` or `TERMINATOR_DATA_DIR` installations keep `config.toml`
in their isolated data directory. A missing file uses defaults without creating
it. For example:

```toml
[appearance]
version = 1
window = "#26282C"
surface = "#191A1C"
text = "#D1D3D9"
accent = "#3871E1"
border_width = 1.0
pane_divider_width = 8.0
```

Settings shows all supported tokens, including terminal, selection, Git and
agent-state colors. Valid edits preview immediately. Apply saves atomically and
preserves comments and unrelated TOML keys; Cancel restores the committed theme.
Reset appearance changes only the draft. Invalid external edits retain the last
valid theme and display an error. An externally changed configuration requires
reloading a dirty draft before saving. Functional settings and navigation remain
in their existing stores; font choices are not part of the appearance file.

Ended sessions are under initially collapsed History sections, with expansion
saved per project. Each project has a top-level tab strip, and each tab owns an
independent split layout and pane focus. Clicking a file opens a new top-level
editor tab; explicit editor-split actions stay in the current tab. Async split
creation stays in its originating tab, even if you navigate elsewhere.

Right-click a project and choose **Remove project from sidebar** to hide it.
Its files, layouts, running terminals and unsaved editors remain intact. Restore
it from **Removed** beside the Projects heading, or add the same folder again.
Removal persists across GUI restarts and does not delete the directory or Git
worktree. Explicitly navigating to one of its sessions restores its sidebar entry.

Explorer and Git share filename icons and status colors. Git separates conflicts,
staged changes, working-tree changes and untracked files; a partially staged file
appears in both applicable groups and opens the corresponding diff. Native
filesystem notifications are debounced by 250 ms. Visible panels reconcile every
30 seconds, or poll every three seconds if watching fails. Hidden panels suspend
refresh work. The app leaves Git fsmonitor configuration alone.

Hover an HTTP(S) URL or existing file path for 400 ms to show actions. Paths are
resolved against the originating session directory, including `~/`, quoted spaces,
and `file:line:column`. Cmd-click (macOS) or Ctrl-click (Linux) opens the default
action. Terminal mouse-reporting applications keep their input behavior. No
Tree-sitter dependency is needed for these targets; Neovim owns editor syntax.


The top-level tab strip sits above the split workspace. Every pane has a compact
title caption and right-click actions for splitting, finding text, saving/comparing
an editor, and closing a session. Legacy stacks of panes remain accessible through
“Tabs in this pane” in the context menu. A normal click on an existing Git file opens its editor. Double-click a
modified, added, renamed, or untracked file to open a diff (working-tree side
when that side is dirty, otherwise staged). Deleted entries open a diff
immediately. Staged and working-tree diffs remain in the file menu.

Git diffs default to the native viewer (syntax highlighting and word-level
hunks, Unified or Split in the tab). Settings → Terminal & Editor → Diff viewer
can switch to Neovim review (bundled CodeDiff). Neovim 0.10+ must be on PATH;
users do not install plugins. The review uses an isolated Neovim profile, without
loading user configuration. Ordinary file editors still use the editor settings.
In Neovim review, use `]c` / `[c` for changes, `t` for inline/side-by-side layout,
`gc` to fold unchanged context, and `q` or the tab X to close without a session
prompt.

Reviews are read-only snapshots: HEAD → index for staged changes, index → disk
for working changes. Reopen or Refresh a review to reload it. Renames, added/deleted
files, untracked files and partially staged files are supported. Two-way reviews
reject unresolved conflicts, binary/non-UTF-8 files, symlinks/submodules, and
files over 1 MiB with an error. Git mutations remain terminal operations. The GUI
checks the running daemon’s advertised capabilities before opening a Neovim
review. Native review does not need that capability. With Diff viewer set to
Neovim and an older daemon, the built-in viewer is used and the GUI explains that
Neovim review needs a daemon update, keeping live sessions intact. Restarting just
the GUI loads this compatibility fix; building a package does not replace a
running daemon.

CodeDiff and its C diff library are pinned and bundled in the daemon, built locally
with the C compiler used by Cargo; there are no first-use downloads or OpenMP
runtime dependency. Attribution and upgrade instructions are in
[`vendor/codediff.nvim/UPSTREAM.md`](../vendor/codediff.nvim/UPSTREAM.md).

Settings has category navigation and grouped appearance controls. The attention
bar distinguishes missing managed hooks from having no pending agent events.
Configure an agent under Settings → Agent Hooks, then start a fresh agent CLI
inside a Terminator terminal to receive its initial lifecycle events. Merely
running an agent process does not produce events; Terminator does not install
hooks or infer agent state from terminal text automatically.


The selected terminal is outlined with the appearance accent color (blue by
default). Rename a terminal through its right-click menu, its tab menu, or its
Projects row; double-clicking a top tab also edits its name inline. Press Enter to save or Escape to cancel.

File opening always creates a separate editor session in a new top-level tab.
Use “Open in editor split” to place an editor beside a pane in the current tab. When Vim exits, its view closes
and focus returns to the originating session when available. Closing or exiting
the last session in any split removes that pane and expands its sibling. Ended
records remain in History, and stale ended panes are cleaned up on GUI restart.

All panes display a compact terminal-title caption inside their top border. Click it to focus the pane, or double-click/right-click to rename it.


Existing project layouts migrate into the first top-level tab without restarting
sessions or changing their split arrangement. Tab order, selected tab, and each
tab's layout/focus are saved in version 2 of the project's layout JSON; the daemon
protocol and SQLite schema are unchanged. Use the updated GUI with these layouts.
Unknown layout versions are reported and are not overwritten.

Closing a top-level tab containing shells or active agents offers keeping them
running in the background or terminating them. A sidebar session click locates its owning tab, or restores a
background session into a new top-level tab using the same process. Quitting a
sole editor removes its tab and restores the other tab's last focused pane.

Focus outlines briefly emphasize a newly selected terminal, then fade to a thin, soft outline. File-editor pane captions include an X button with the existing close/save choices.

Top-level tabs use a flat strip with a terminal icon, close button, and muted active underline. Renaming edits the label in place on the tab, pane caption, or sidebar row; valid edits also save when focus leaves the field.


Clean file-only tabs and editor panes close directly without a terminal-session
confirmation. If any editor buffer has unsaved changes, a high-visibility bar
inside that file offers Save and close, Discard changes, or Cancel. The rest of
the app stays usable while the bar is visible. Tabs containing shells or active
agents retain the session-close confirmation. Double-clicking an Explorer/Git file opens one editor,
not a duplicate editor for the second click.

Explorer uses Git colors on filenames, icons, and badges: **U** for untracked,
**M/A/D/R/C/T** for tracked changes, and **!** for conflicts. Conflicts take
precedence, followed by working-tree status and then staged status. Folder
badges aggregate descendants, prioritizing conflicts and deletions. Hover the
Explorer icon for the effective directory and any last-known-directory qualifier.

## Native previews and explicit controls

Markdown editor tabs place the filename, flat **Edit / Preview / Split** tabs and
a refresh icon together on one header row. Edit uses the existing Neovim session; Preview shows native rendered
Markdown; Split places a resizable editor beside the preview. Files start in
Preview, and each tab's selected mode survives GUI restarts. Preview follows that
file's live Neovim buffer, including unsaved changes, even when another Neovim
buffer is selected. Switching modes never creates another editor or saves a file.
The unsaved-close bar also applies while the editor is hidden by Preview.
If Neovim is busy or waiting at a prompt, Preview keeps the last live text or
shows the saved file with a **live preview paused** label. Live updates resume
when the editor is ready; switch to Edit to inspect or answer its prompt.

The native viewer supports headings, links, lists, tables, read-only task lists,
highlighted code blocks, and local raster/SVG images. Relative links and images
resolve beside the Markdown file. The **refresh icon** also reloads images. Remote images
show their alt text without being downloaded. HTML stays text; browser rendering,
Mermaid and typeset math are not included. Preview text is limited to 1 MiB.
Custom terminal editors without Neovim RPC show a clearly labeled **Saved file**
preview. Ordinary Neovim configuration and Git review profiles are unchanged.

HTML files open as a GUI-only Blitz preview tab (no PTY, no JavaScript). The
header always offers **Open in browser** (`file://` to the system browser) when
the raster is incomplete, plus Reload and Open as text for Neovim. Explorer,
Git, and terminal menus still offer **Open in browser**. No Chromium or webview
is bundled. HTML-bearing layouts use version 4. Transitive `stylo` is MPL-2.0.

Click PNG, JPEG, WebP, GIF, BMP, ICO, TIFF, or SVG files to open a native image tab.
Fit, 100%, pan/zoom, Reload, Open as text, and Open externally are available.
Raster previews decode in a bounded worker and use the first frame of animated
formats. SVGs are rasterized with resvg; external references are not loaded.
Corrupt or over-limit images display an error instead of opening binary text.
Image-bearing layouts use version 3; ordinary version-2 workspaces remain readable.
Older GUIs refuse the newer layout rather than overwriting it.

Click mp3, flac, ogg, wav, m4a, opus, or aac to open a GUI-only **Player** tab
(no PTY). Palette → **Open player**. Transport, playlist, volume, and radio
stations share that tab. Bundled Icecast/Shoutcast HTTP(S) streams plus custom
stream URLs. Open as text and Open externally still work. Playback stops when
the GUI exits or the player tab closes. Player-bearing layouts use version 5.

`terminator-hook ctl --help` exposes explicit controls. Examples:

```sh
terminator-hook ctl add-project /absolute/project
terminator-hook ctl create PROJECT_ID
terminator-hook ctl create PROJECT_ID --background
terminator-hook ctl split SESSION_ID right
terminator-hook ctl send SESSION_ID 'cargo test' --enter
terminator-hook ctl read SESSION_ID --screen
terminator-hook ctl open-file PROJECT_ID /absolute/image.png
terminator-hook ctl worktree add PROJECT_ID /absolute/task-checkout --branch task-name
terminator-hook ctl worktree list PROJECT_ID
terminator-hook ctl worktree remove WORKTREE_PROJECT_ID
terminator-hook ctl metadata SESSION_ID --pr
terminator-hook ctl notify SESSION_ID 'Build finished'
terminator-hook ctl shutdown
terminator-hook ctl shutdown --stop-all
terminator-hook ctl shutdown --stop-all --relaunch
```

Run shutdown from Terminal.app or another terminal outside Terminator. The default
refuses live sessions. Explicit `--stop-all` closes the GUI through its normal
workspace checkpoint, stops every live shell/editor through the daemon, waits for
session exit, flushes saved history, and waits for daemon teardown. `--relaunch`
then starts this installation's GUI with the same data and runtime directories
(never a bare `open`). **Save your work first: unsaved editor buffers are discarded and running jobs stop.** The
command retains session records and history.

When the status bar shows that the app and session service use different
installations, **Restart session service** asks for confirmation and runs that
same detached stop-all plus relaunch. Idle mismatch still auto-repairs without
stopping live sessions. Sparkle updates still replace only the GUI. A failed GUI close, changed daemon,
new concurrent session or timeout aborts cleanup; it never force-kills processes.
If an older installed GUI times out while hidden or minimized, restore its window,
quit it completely, and retry. Current builds service control requests and exit
checkpoints even while minimized.
The default wait is 30 seconds per phase (`--timeout 1..300`). After successful
completion, reopening the app starts its installed daemon without relaunching
historical sessions. No fixed sleep is needed before reopening.

The GUI control endpoint is a private authenticated `gui.sock`; the daemon retains
its existing versioned JSON socket. Unsupported daemon features fail before any
worktree mutation. UI commands acknowledge acceptance; layouts retain the usual
asynchronous persistence behavior. Worktree removal keeps branches and historical
session records, and refuses dirty/locked checkouts or projects with live sessions.
Starting work from another checkout resolves HEAD in that checkout.

Selected-context metadata includes branch, worktree status, listening ports from
the session process tree, and optional PR details through `gh`. PR lookup is opt-in
in Settings, cached for 60 seconds; ports refresh every 3 seconds and repository
metadata every 30 seconds. Missing helpers and lookup failures are exposed in the
metadata result. A terminal notification is distinct from an authenticated agent
lifecycle event; OSC 9, OSC 777 notify, and OSC 99 title/body messages are supported.
Desktop delivery of terminal messages is separately configurable.

External-browser automation stays outside the native application. Use
`terminator-hook ctl browser open URL` for the system browser. For an existing
local CDP page endpoint, the browser subcommands `navigate`, `snapshot`, `click`,
`fill`, `evaluate`, and `screenshot` accept `--endpoint ws://127.0.0.1:PORT/devtools/page/ID`.
They neither embed nor download a browser. A remote endpoint requires an explicit
local tunnel. HTML/CSS snapshots and screenshots are produced only when requested.
