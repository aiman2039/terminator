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

Wgpu is the default renderer on all platforms. To use the Glow recovery renderer, launch with
`TERMINATOR_RENDERER=glow cargo run -p terminator --locked`, or set that environment
variable when launching the app executable.
`cargo build -p terminator --no-default-features` builds the GUI with Glow only.

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

Debian/Ubuntu desktop runtime libraries include `libxkbcommon-x11-0`, `libxkbcommon0`, `libgl1`, and the usual X11/Wayland desktop libraries. Linux compilation also needs `pkg-config`, `libfontconfig1-dev` so resvg can use system fonts, `libasound2-dev` for rodio/ALSA, plus the matching X11/Wayland/GL headers. `libwebkit2gtk-4.1-dev` is required at build time; `libwebkit2gtk-4.1-0` is required at runtime. WebKitGTK is required for in-app Browser tabs on Linux (X11). Build with `--no-default-features --features wgpu` (or `cargo xtask package --portable`) to drop the `webview` feature and the WebKitGTK dependency; Browser tabs then show a message and Open in browser still works. Install an XDG desktop portal backend appropriate to your desktop for file dialogs. `cargo xtask linux-check` configures a disposable Linux test container; `scripts/README.md` lists the Rust validation tasks.

## Daily workflow

- Add a project through the native folder picker. File and folder pickers start at the focused terminal's current directory, falling back to the selected project and launch directory.
- Open a terminal. The automatic shell preference is **zsh → bash → sh**, selecting the first installed executable. Settings accepts an explicit shell override; clear it to restore automatic selection.
- Use **+** in the top-level tab strip to create a terminal tab with its own split layout. Explorer hides Git-ignored entries by default; **Show ignored files** reveals them (including Git metadata). Ordinary dotfiles remain visible.
- Right-click a terminal or its tab for **New tab**, **Split up**, **Split down**, **Split left**, and **Split right**. Each split stays inside its owning top-level tab.
- Launch agents and manage worktrees in the terminal yourself. The app does not launch agents or perform Git writes.
- Project arrows expand/collapse independently of selection and persist across restarts. The project-list header icons order visible projects by name A → Z (default), name Z → A, or latest activity, and restore a removed project; the choice persists in `ui-preferences.json`. Latest activity is the latest of session creation, agent updates, notice times, and restoring, adding, or creating a project. Selecting a visible project does not change its rank. The window header holds project tabs and the Explorer, Agents, Git, and Settings icons. Explorer, Agents, and Git switch the right sidebar; click the active tool to collapse it. The left Agents row stays compact (waiting/unread counts) until opened. Explorer, Git, and History do not list attention events. Agents lists undismissed, unsnoozed notifications for all projects, including retained events from ended sessions and terminal notices. Unresolved waiting events appear first. Waiting input and permission notices leave the inbox when the agent continues or a newer request replaces them; completed and failed notices remain until the session is focused or the notice is dismissed. The Agents header waiting badge uses that same list, not live agent sessions. Working agents without notifications do not create cards. Sidebar width persists globally. Settings → Place notifications at the side hides the top Attention strip (default); unchecking it restores the top strip.
- Right-click a local project for **Init command…** (`project-init-v1`). The daemon types that one line and Enter into each new shell terminal of the project, for example `source .venv/bin/activate`. Editors, reviews and SSH terminals skip it. Blank removes it.
- Select a project to restore its own layout. A terminal that changes directory stays under its owning project; the file/Git sidebar follows its effective directory.
- Single-click anywhere on an Explorer file row to open a new editor tab. Right-click a file path to open the editor, a new editor split, or an external editor. The Explorer, Git, and Perforce file menus show no shortcut next to **Open file** or **Open in editor split**, because no shortcut does these actions. `command+O` uses the native file picker (Cmd+O on macOS, Ctrl+Shift+O on Linux). Change it in Settings → Shortcuts. Settings shortcuts and JetBrains keymap chords are consumed by the GUI and are not typed into the terminal; on Linux a focused terminal keeps Alt+letter and Alt+digit. Other Command/Ctrl chords go to the focused terminal as kitty CSI u (Command is Super, so Neovim `<D-u>` works); Ctrl+A–Z stay control characters, and Cmd/Ctrl+C/V copy and paste. Drag selects terminal text even in nvim or a full-screen agent. Scrolling during that drag extends the selection by the lines the viewport actually moved through local history, including when the terminal is not full screen; a scroll that cannot move leaves the selection where it is. On the alternate screen the wheel goes to the agent and the selection grows by that line count until the pointer moves. Holding the pointer outside the pane keeps scrolling. Then Cmd+C (macOS) or Ctrl+Shift+C (Linux) copies it. Empty copy does not clear the clipboard. A wheel that is not part of a selection still goes to the app when it enables mouse reporting. On the alternate screen, a wheel that is not a mouse report is sent as cursor keys.
- Settings → Terminal & Editor retains embedded Neovim, terminal-editor, and external-editor modes. External presets include System default, VS Code, Cursor, RustRover, Zed, and Custom. Named presets use macOS application launching or Linux CLI launchers. Custom takes an executable and one literal argument per row; the absolute file path is appended without shell evaluation. **Choose file and test…** launches the draft without saving. Missing launchers and failed exits appear in the status bar; long-running editors remain independent of the GUI.
- The default editor is real Neovim, rendered inside the native terminal widget and controlled through Neovim RPC for save/compare actions. Your Neovim configuration and plugins load normally. An editor-side tree can therefore come from your own configuration.
- Closing a tab with live shells or agents asks whether to terminate or background them. Clean file-only tabs close directly. Closing the GUI leaves the daemon and sessions running.
- After a reboot or daemon loss, historical tabs show retained output and known resume commands. Agents are never restarted automatically.

## Agent setup

Settings → Agent hooks installs, repairs, and removes managed hooks for Claude Code, Codex, OpenCode, Muse, Grok, and Cursor. Installation preserves unrelated hooks and writes a backup before changing an existing configuration. It never installs agent binaries or configures accounts.

Read [the integration contract and capability notes](INTEGRATIONS.md) before connecting a custom agent. Installed hooks are inert outside terminals owned by this application. Notification defaults cover input, permission, completion, and failure events where the agent exposes them; opening the relevant terminal dismisses the notification without changing agent state. Settings → Notifications can play a system sound with desktop banners (default on). The in-app Agents inbox stays silent. The control requires a daemon that advertises `notification-sound-v1`.

## Local state and updates

The default data directory follows the OS application-data convention. `TERMINATOR_DATA_DIR=/absolute/path` or `terminator --data-dir /absolute/path` selects an isolated installation. The runtime directory contains a private Unix socket and authentication file; do not share these files.

SQLite stores projects, layouts, session metadata, agent state, and notification state. `ui-preferences.json` stores versioned per-installation navigation/sidebar choices, project sort/activity, player playlists/volume/custom radio stations, and the one-time typography, Attention, and keymap migration markers. Attention migration turns on side placement once after the daemon acknowledges the settings update; later placement choices are preserved. Side placement keeps the compact Agents row as the persistent indicator and does not overlay Git, Explorer, or History. The Islands update sets terminal/editor size to 13 once; subsequent user size choices are preserved. Inter and JetBrains Mono are bundled with their licenses. Scrollback is stored separately and pruned by configurable age/per-session/total limits. Defaults: 30 days, 50 MiB/session, 512 MiB total. Metadata and resume commands remain until explicitly removed. Truncated output is labeled.

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
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --bins --examples --features terminator/test-support --locked
sh scripts/run-built-xtask.sh integration
sh scripts/run-built-xtask.sh gui all
sh scripts/run-built-xtask.sh gui ui-cleanup --output /tmp/terminator-ui-2x --scale 2
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

Toolbar buttons show 14 px icons in every row, and so does the close button on
the top-level tab strip. Sidebar toolbars, tab strip controls, pane bars, the
Markdown toolbar, and the status bar use 22 px buttons. The Player controls in
the status bar use the same size, so the bar height does not change when audio
plays. The window header uses 28 px buttons for all its icons. At the default
sidebar width, the header shows all its buttons. A narrower right sidebar moves
the last buttons into the **More** menu.

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

A project folder can be moved or deleted, for example a removed Git worktree.
The sidebar then shows the project with a dim name and a warning icon. Its
tooltip starts with **Folder not found**. A project without a live terminal
leaves the sidebar until the folder is back. The Explorer shows **Folder not
found: PATH. It was moved or deleted.** It does not show the Privacy & Security
hint. **Locate Folder…** opens the folder you pick as a project and selects it.
The session service cannot move a project to a new path. Thus Terminator then
asks to remove the missing project. **Remove Project** is the same as **Remove
project from sidebar**. It also removes the card of a managed worktree. Its
terminals continue to run. **Retry** reads the folder again. Terminator does
not run Git, for example `git worktree prune`. A managed worktree card with a
deleted folder stays in the sidebar. **New terminal** on that card shows the
same message. Without a live terminal, a click or **Open** shows it too. If the
folder of the focused terminal is deleted, the Explorer shows the project
folder. A note above it says **Terminal folder PATH was moved or deleted.
Showing the project folder.** The terminal continues to run. The Explorer
shows the terminal folder again when the terminal reports a different folder,
or when you click **Retry** or **Refresh**. Terminator does not send input to
the terminal. If the project folder is also missing, the Explorer shows **Folder
not found** with **Locate Folder…**, **Remove Project** and **Retry**. If you
pick the folder of the open project, the status bar tells you that the project
is already open. If you open the folder of a removed worktree again, Terminator
adds that project again. A new worktree at the path of a removed worktree opens
as its new project. Other read errors keep **Choose folder again** and the
macOS Privacy & Security hint.

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

Git diffs default to the native viewer. It shows HEAD → index for staged changes
and index → working tree for working changes, with syntax highlighting, Unified
or Side by side layout, and collapsed unchanged lines (click a gap row, or use
the `↓20` / `↑20` steps). Changed lines use the Git sidebar colors: added
`git_added`, deleted `git_deleted`, and modified `git_modified`, with a soft row
tint and a stronger tint on the changed words. The toolbar is one row: the
changed-files panel toggle, previous/next change (F7 / Shift+F7, which continue
into the next or previous file), Open at change (F4), Refresh, previous/next
file, the gear menu, the layout toggles, Undo, and the change counter. A
one-row header names each side, and a fixed notification bar under the toolbar
shows results, the discard confirmation, and hunk progress without moving the
panes. The changed-files panel lists every changed file on the same side,
grouped by folder with Git status colors; click a file to show its diff in the
same pane.

In Side by side, the narrow band between the panes connects each change and
always shows its action chevrons. Working tree: `>>` reverts the change
(discard, after a confirmation) and `<<` stages it. Staged: `>>` unstages it.
Unified shows the same actions as gutter buttons. Alt+click changed lines to
act on those lines only. The gear menu holds the unchanged-line context,
whitespace mode (do not ignore / trim / ignore all), Ignore case, and highlight
granularity (words, characters, lines, or none); these options are saved in
`ui-preferences.json` and apply to every native diff. They change only the
comparison: rows show the real text, and staging or discarding applies the real
lines. Remote and Perforce diffs are read-only.

Settings → Terminal & Editor → Diff viewer
can switch to Neovim review (bundled CodeDiff). Neovim 0.10+ must be on PATH;
users do not install plugins. The review uses an isolated Neovim profile, without
loading user configuration. Ordinary file editors still use the editor settings.
In Neovim review, use `]c` / `[c` for changes, `t` for inline/side-by-side layout,
`gc` to fold unchanged context, and `q` or the tab X to close without a session
prompt.

Neovim reviews are read-only snapshots: HEAD → index for staged changes, index → disk
for working changes. Reopen or Refresh a review to reload it. Renames, added/deleted
files, untracked files and partially staged files are supported. Two-way reviews
reject unresolved conflicts, binary/non-UTF-8 files, symlinks/submodules, and
files over 1 MiB with an error. Neovim reviews never change Git state. The GUI
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
Projects row; double-clicking a top tab also edits its name inline. Press Enter to save or Escape to cancel. Right-click a top-level tab to close it, close all tabs to the left or right, or add a new top-level terminal to the left or right.

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

File tabs show the icon of their file type, on the tab strip and in pane tab
bars. Native editor, image and Neovim editor tabs show it. A native editor
with unsaved changes shows a dot in place of its close button. Point at the
dot to see the close button again; a click closes the tab with the usual
unsaved-changes choices. On the tab strip, the dot means that a file in that
top-level tab has unsaved changes. When two open tabs show the same file name,
each tab also shows its parent folder in muted text, for example `mod.rs app`.
When the parent folders have the same name too, the tab shows more folders,
for example `lib.rs core/src`. SSH files show their remote folders. The tab
strip compares the tabs on the strip. Pane tab bars compare the files in the
same top-level tab or floating window.

Top-level tabs, including native and Neovim editor tabs, can be pinned from
their context menu. Bulk close actions leave pinned groups open; an explicit
close still closes one.
The History icon beside the tab-strip plus button lists recently selected tabs
for that project. Pin state and recent order survive a layout reload. The
native editor's separate **Bookmark file** and **Recent files** items keep file
paths for quick reopening; they do not pin a workspace tab.

Native editors have no path bar above the text. The status bar shows the
focused editor's path and the symbol at the caret, for example
`project › src › main.rs › main`. When the status bar is short of room, the
first folders fold into `…`, and then the symbol hides. Unsaved changes show
as a dot on the file's tab. Click a folder or the file name to show it in
Explorer. Click the symbol to open File structure. The status bar also shows
**Saving…** and **Renaming…** while the file is busy. Right-click the
editor's pane tab for **Reload from disk**, **Bookmark file**, **Recent
files** and **Local History**. Reload is off while the file has unsaved
changes. In the editor's
status row, click the line-break cell (LF or CRLF) or the encoding cell to
change them; the next save writes the new format. With Vim keys on, the
status row also shows the Vim mode. The editor context menu has Save and Find
references.

Markdown files show a small floating toolbar in the top-right corner of the
editor: the format buttons, **Go to heading**, then **Edit**, **Preview** and
**Split**. It fades out while you type, so it does not cover your text. It
comes back after a short pause, or when the pointer is on it.

The native editor offers Problems, Structure, Quick Documentation,
definition, references, and preview-first symbol rename for Rust and Python
when their language servers are available. F2 or the editor context menu
starts rename. The
preview shows the proposed changes by file, then Apply checks that files and
open buffers have not changed. Unsupported file operations and paths outside
the project are rejected. JavaScript and TypeScript syntax highlighting does
not require an LSP server; their native LSP actions remain disabled.

Quick Documentation shows the docs of a symbol in one popup under it. Rest
the pointer on a symbol for half a second, or press F1, Ctrl+J, Cmd+K
(Ctrl+K on Linux), or `K` in vim Normal mode for the symbol at the caret.
The editor context menu has Quick documentation too. Problems at that spot
show at the top of the same popup, with no second tooltip. Markdown docs
show highlighted code blocks; plain-text docs stay plain text. Long docs
scroll. Move the pointer into the popup to scroll or select its text, and
Cmd+C (Ctrl+C) copies the selection. A key press, a click or scroll
outside the popup, or a switch to another app hides it. A pointer popup
also hides when the pointer leaves the symbol. To turn off the pointer
docs, clear Settings → Editor → Documentation. Problems under the pointer
still show, and F1 still works.

Saving a native buffer (Save, `:w`, autosave, or Save and close) tells its
language servers about the save. An applied rename does the same after the
buffer reloads. rust-analyzer then checks the project again, so underlines,
Problems, and the status-bar counts update for the saved file and the other
open files. Problems also update while no editor is visible.

Native completion opens when you type an identifier character or a trigger
character of the language server (for example `.`), or press Ctrl+Space.
Each row shows the parameters after the name, and the type or module on the
right in dim text. Text that does not fit is cut. When the selection stays
on an item for a short time, the editor asks the server for its
documentation. A panel next to the list shows it with the Markdown renderer
of Quick Documentation. The panel is on the right of the list, or on the left
when the right has no space. When the server marks its list as incomplete,
each keystroke asks the server again. Enter or Tab inserts the item and its
import. When the import arrives after the insert, it is a separate undo
step. When the text above the item changed first, the import is not added
and a message tells you.

A snippet item (for example a function call with its arguments) selects
its first placeholder. Type to replace it. Tab goes to the next
placeholder, and Shift+Tab goes back. A thin box shows each placeholder.
When a placeholder occurs more than once, each copy gets a caret, so the
copies change together. After the last placeholder, Tab puts the caret on
the final stop (`$0`), or at the end of the snippet. Enter goes there at
once and adds no new line. Escape, Undo, or a caret move out of the
snippet ends it. In vim mode this works in Insert mode only: the caret
goes after the placeholder, and the first key you type replaces it.
Leaving Insert mode ends the snippet. A choice placeholder inserts its
first choice. Snippet variables such as `TM_FILENAME` and
`TM_SELECTED_TEXT` give their value, else their default, else nothing.


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

Explorer's Locate button reveals the focused local file: it clears Explorer
search, selects Names, opens parent folders, and scrolls to the file. If the
current shell directory does not contain that file, Explorer uses the selected
project root for the reveal without changing the shell directory. Ignored
parents and the target remain visible for that focused file. The button is
disabled when no local file is focused or the file is outside the project and
Explorer roots.

Explorer **Contents** search lists matching lines by file. Click a line to open
the file at that line. The native editor also puts the caret on the match.
Images, HTML, and audio still open in their default viewer. A file that is
already open in a Neovim or terminal editor tab gets focus and keeps its cursor.

**Find in Files** searches the text of project files. Press Cmd+Shift+F on
macOS or Ctrl+Shift+F on Linux (Settings can change the key), or use Search
Everywhere. It works from any pane. On Linux, a terminal with keyboard focus
keeps Ctrl+Shift+F for Find in terminal. In a native editor, Cmd/Ctrl+Shift+F
also opens it with the selected text. Otherwise the last query comes back,
selected. If the Explorer has focus, the search uses the selected folder. The
folder menu item **Find in Folder…** does the same. Up and Down select a
result. The preview shows about ten lines around it, with the match marked.
Enter or a double click opens the file at the match. Esc closes the window. The
search includes dotfiles that Git ignore rules do not exclude, and skips version
control metadata (`.git`, `.svn`, `.hg`, `.bzr`, `_darcs`, `.jj`, `CVS`). Spaces
around the query are part of it: `int ` does not find `print`.
**File mask** and **Exclude** take globs, for example `*.rs, src/**`. Results
show while the search runs. At 1000 results the search stops and asks you to
refine it. Remote projects are not searched; a note tells you so.

**Replace in Files** is the same window in replace mode. Press Cmd+Shift+R on
macOS or Ctrl+Shift+R on Linux, use Search Everywhere, or click the pencil icon
in the window. A terminal with keyboard focus keeps that key for Rename
terminal. Everywhere else, the key opens Replace in Files. If Settings gives
Rename terminal another key, both keys work everywhere. In a native editor,
Cmd/Ctrl+Shift+R opens it with the selected text; Cmd/Ctrl+R still replaces in
the file. The folder menu item **Replace in Folder…** limits it to that folder.
Type the replacement under the query. With regex on, `$1` inserts group 1 and
`$$` inserts a dollar sign. If a letter, digit, or `_` comes next, write
`${1}`, for example `${1}Id`. Each file and each result line has a checkbox. An
unchecked line does not change. The pane under the results shows each line of
the selected file before (−) and after (+) the replace. **Replace All** changes
every checked line, closes the window, and says what changed. **Replace in
Selected File** changes only that file and removes it from the list. Files are
written in the background, all of them or none. One replace changes at most 64
files, 4096 matches, and 16 MB, and no file over 1 MB. A read-only file stops
the replace; with Perforce, open it for edit first. A native editor with
unsaved changes stops the replace; save it first. A clean native editor gets
the change as one undo step. Neovim and terminal editors see the change on
disk, as with any other outside edit. If a file changed after the search,
nothing is written and the search runs again. Remote projects are not changed;
a note tells you so.

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

HTML files and http(s) URLs open as a GUI-only **Browser** tab (OS webview, no PTY).
The header offers Back, Forward, Reload, **Open in browser** (system browser), and
Open as text for local files. The native view hides when the pane is covered and
dies with the GUI. Its profile is isolated by data directory (platform storage
details below). No Chromium is
bundled. Browser-bearing layouts use version 6; v4 HTML tabs migrate. Linux
Wayland cannot host a child webview yet — use Open in browser.

Click PNG, JPEG, WebP, GIF, BMP, ICO, TIFF, or SVG files to open a native image tab.
Fit, 100%, pan/zoom, Reload, Open as text, and Open externally are available.
Raster previews decode in a bounded worker and use the first frame of animated
formats. SVGs are rasterized with resvg; external references are not loaded.
Corrupt or over-limit images display an error instead of opening binary text.
Image-bearing layouts use version 3; ordinary version-2 workspaces remain readable.
Older GUIs refuse the newer layout rather than overwriting it.

Click mp3, flac, ogg, wav, m4a, opus, or aac to play in the chrome **Player**
next to the project-sidebar Agents bell (no PTY). While audio is on, the bar
shows the title plus previous / play-pause / next / playlist. Files skip the
selected global playlist (Default is created on first file). Radio skips bundled
then custom stations. The sound-wave icon or playlist button opens a single global
**Player** view in the main pane (not a popup and not a workspace tab). Opening it
again shows the same instance; leftover Player tabs are closed. First open seeds
**Default** with bundled tone files (`pulse.wav`, `hum.wav`, `chime.wav`) if that
playlist is empty.
Files can be seeked; live radio cannot. Shuffle and repeat persist.
The Radio button switches the Player to the bundled station catalog
(search and category filters). Users can add HTTP(S) stations. Live radio
cannot seek. Palette → **Open player**.
Open as text and Open externally still work. Playback stops when the GUI exits,
not when the Player view hides.

Git sidebar click on a modified, untracked, or deleted file opens the diff.
Explorer, palette, and **Open file** still use the default viewer even when the
file is git-dirty.

**Search Everywhere** (`Command+P`, double Shift, or the header search button) uses one fuzzy
list for files in visible projects, open-editor symbols, symbols from active
Rust/Python language servers, projects, live sessions, waiting agents, actions,
and Settings sections. Type a name or path, use the arrow keys, then press
Enter. File and symbol results open in the owning project; symbol results open
at their source line. Add `:line` or `:line:column` to a file name
(`main.rs:42:5`) to open the file at that line. Files match on the name part
only. The file index runs in the background. It lists dotfiles unless Git
ignore rules exclude them, and skips version control metadata (as Find in
Files does) and common generated directories.
Language-server symbols appear when a supported server is ready.

**JetBrains keys** (Settings → Editor → Keymap, on by default) work in every
pane: Explorer, Git, terminals, diffs, Settings, and with no editor open.
Double Shift opens Search Everywhere. The second Shift tap must come soon, with
no other key between the taps, so typing capitals does not open it.
Shift+Command+A (Find Action) opens the palette on **Actions**. Shift+Command+O
(Go to File) opens it on **Files**. These keys never reach the terminal. VNC
shortcut capture still takes every key. Command+[ and Command+] (Back and
Forward) work only in a focused editor; elsewhere Command+] is **Next pane**.
On Linux the keymap uses Ctrl+Shift+A, Ctrl+Shift+N (Go to File), and Ctrl+E
(Recent files). A focused terminal keeps Ctrl+E (end of line) and
Ctrl+Shift+A (**Select all**).

**Editor commands** show in Find Action, and in Search Everywhere when you
type. Each row shows the JetBrains name and the native editor key, for example
**Duplicate Line or Selection** (Command+D), **Go to Line** (Command+L),
**Reformat Code** (Option+Command+L), and **Find Usages** (Option+F7). The
command runs in the native editor on screen that had focus last, the same as
its key. That editor then gets the keyboard focus again. The list shows only
the commands that work in that editor: language-server commands need a server
for the file type, multiple-caret commands need Vim off, and a read-only file
shows no edit commands. With no native editor on screen, there are no editor
commands. With the JetBrains keymap off, rows show the other keys, for example
F12 for **Go to Declaration**. On Linux the keys show as Ctrl, Alt, and Shift.

**Recent files** is `command+E` in every pane (Cmd+E on macOS, Ctrl+Shift+E on
Linux). **Toggle IDE mode** moved to `command+alt+E`. On the first start after
this change, a binding still at its old default moves once: Toggle IDE mode off
`command+E`, Recent files off `command+alt+E`. A binding you changed stays. A
move never takes a chord that another action uses.

**Tool window keys** follow the JetBrains keymap: Command+1 Explorer,
Command+6 Problems, Command+7 Structure, Command+9 Git, Option+F12 Terminal,
and Option+F1 Select In. On Linux they are Alt+1, Alt+6, Alt+7, Alt+9,
Alt+F12, and Alt+F1. Change them in Settings → Shortcuts. A key shows a hidden
tool window and focuses it. If the tool window shows but has no focus, the key
focuses it. If it has focus, the key hides it. A tool window opens on the side
where it shows now. Else it opens on its usual side: the left sidebar if a left
stripe put it there, else the right sidebar. Focus is keyboard focus: arrow
keys work at once in the Explorer tree and in the Git, Problems, and Structure
lists, and Enter opens the row. In a list, the key focuses the first row on
screen, and the list scrolls to keep the focused row in view. Esc in a focused
tool window returns focus to the last editor or terminal. That Esc does not go
to the terminal or editor, so it does not stop an agent. In the Explorer, the
first Esc ends a type-to-search.
In IDE mode, Alt+F12 shows the terminal strip and focuses a shell there. It
hides the strip when a strip shell has focus. In regular mode, Alt+F12 focuses
the last shell, or opens one. Select In shows the Explorer and selects the
current file, like the Explorer's locate button. Stripe and header tooltips,
the header overflow menu, and the palette show these keys. On Linux, a focused
terminal keeps Alt+letter and Alt+digit keys (Meta keys for the shell), so
there Alt+1, Alt+6, Alt+7, and Alt+9 go to the terminal. Alt+F12 and Alt+F1
work in every pane. Some Linux desktops use Alt+F1 for their own menu. On a Mac
keyboard, hold Fn for Option+F1 and Option+F12 unless F-keys are standard
function keys.

`terminator-hook ctl --help` exposes explicit controls. Examples:

```sh
terminator-hook ctl add-project /absolute/project
terminator-hook ctl create PROJECT_ID
terminator-hook ctl create PROJECT_ID --background
terminator-hook ctl split SESSION_ID right
terminator-hook ctl send SESSION_ID 'cargo test' --enter
terminator-hook ctl read SESSION_ID --screen
terminator-hook ctl agents --project PROJECT_ID
terminator-hook ctl wait SESSION_ID --state waiting --timeout 600
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

`ctl agents` prints each session's newest agent record as JSON: session,
project, label, live, kind, hook-reported state and age in seconds. `ctl wait`
checks the daemon snapshot every 500 ms. `--state` is `waiting` (input or
permission), `done`, `failed` or `any-stop`. It prints the state and exits 0
on a match, 2 when `--timeout` seconds pass, and 3 when the session ends. It
needs `agent-presence-v1`. Both use hook state only, never terminal text.

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

Browser navigation uses persistent pane IDs (compatible with layout v6); legacy
browser entries receive IDs when loaded. Links, redirects and history navigation
use the same HTTP(S)/local-HTML policy as opening a tab. Closing a pane or its
containing top-level tab destroys its webview. WebKitGTK stores its profile below
the data directory. macOS 14+ uses a named WebKit store whose UUID is saved under
that directory; macOS 12–13 uses nonpersistent private browsing to avoid sharing
the default store. OS-managed macOS store contents are not in the data directory.

Playback has an explicit owning project and is polled from app logic, including
when the GUI is minimized. Switching projects does not change the current
playlist. Closing the owner's Player stops playback; closing another project's
Player does not. Only one audio source plays at a time. Radio uses connection and
per-read timeouts without a total stream deadline; initial output honors saved
volume, including mute.

### Native editor block selection

With Vim keys enabled, press Ctrl+V in Normal mode to start block selection.
Ctrl+Q also starts block selection when the platform reserves Ctrl+V for paste.
Use arrow keys or `h`, `j`, `k`, `l` to select the rectangle.
Press `d`, `x`, or Delete to delete it. Press `I` to insert before the block,
`A` to insert after it, or `c` to replace it. Type the text, then press Escape
to apply it to the other selected lines. Press `u` to undo the whole action.
Command+V on macOS remains paste.

In a native editor selection (including Markdown), Tab indents all selected
lines by one indent level (see Native editor indentation). Shift+Tab removes
one level of leading spaces or one leading tab. The selection stays active.
Vim Visual mode also accepts `>` and `<` for these actions. Each action is
one undo step.

### Native editor undo and new lines

Undo and redo move the caret to the change. If the change is off screen, the
view scrolls to it. With Vim keys off, undo also puts back the caret and the
selection from before the edit. Redo puts the caret at the start of the
change. In Vim mode, `u` and Ctrl+R put the cursor at the start of the change.
With many carets, undo keeps one caret at the first change.

Shift+Enter starts a new line: the caret goes to the end of the line, then
Enter adds an indented line below. The text after the caret stays on its line.
Command+Option+Enter on macOS (Ctrl+Alt+Enter on Linux) adds an indented line
above the current line. Above a closing bracket, the new line gets one more
indent level. A selection is cleared, not deleted. With Vim keys off, each
action is one undo step. With Vim keys on, these keys work in Insert mode and
join its undo step. In Normal mode, Shift+Enter stays Enter.

### Native editor indentation

Each file has one indent setting: tabs or spaces, the indent width, and the
tab width. When the file opens, the editor reads `.editorconfig`
(`indent_style`, `indent_size`, `tab_width`). It reads the files from the
file's folder up to the first `root = true`; a nearer file wins. A value
that `.editorconfig` does not set comes from the indentation the file already
uses. A Go file or a Makefile without indentation uses tabs. Other files
without indentation use the `shiftwidth`, `tabstop`, and `expandtab` options
of `~/.vimrc` when they are set, else four spaces. The status bar shows the
setting.

Enter, Tab, Shift+Tab, Backspace in the leading indent, selection indent, Vim
`>` and `<`, and Tab in Vim Insert mode use this setting. Language-server
formatting uses it too. Tabs show at the tab width. Indent guides are one
indent level apart.

On save, `insert_final_newline = true` adds a missing newline at the end of
the file. `trim_trailing_whitespace = true` removes spaces and tabs at line
ends, but lines with a caret keep them. These changes are one undo step.
After Undo, while a change can still be redone, a save (also an autosave)
does not apply these rules, so Redo stays available.
`end_of_line` sets the line breaks of a file that has no line break yet
(`lf` or `crlf`; `cr` is not supported). A reload of the file reads
`.editorconfig` again. Remote (SSH) snapshots do not read `.editorconfig`;
they use the indentation in the file.

### Native editor reopen position

A native editor file opens again where you left it: the caret, the top line,
and the folded blocks. This also works after a restart. The editor saves this
in `ui-preferences.json` when the file closes and when Terminator quits. It
keeps the 500 most recent files. A target line (go to definition, a search
result, or `file:line`) always wins. A fold that no longer starts a foldable
block is ignored. A new split of an open file starts at the same caret, top
line, and folds.

### Commit and push

**Commit** and **Commit and Push** in the Git panel run as background jobs.
Hooks and pushes can take as long as they need. The safety limit is 30
minutes. Other Git panel actions (staging, shelves, branches) keep their
20-second limit. While the job runs, the commit area shows **Committing…** or
**Pushing…** with a spinner and **Cancel**. The commit buttons stay disabled,
so a second commit or push cannot start. Actions that you started before the
commit finish first, also a hunk or line that you staged in a diff tab or in
the editor gutter. The rest of the GUI stays usable.

While Git commits, the actions that can change the commit in that repository
are disabled, with the hover text "Wait for the commit to finish": staging
and unstaging (Git panel, Stage All, diff tabs, editor gutter, Mark
resolved), the Git panel's Discard changes, Shelve, Unshelve, Drop, and the
branch switch. Find Action does not show **Git: Stage All**. A diff tab key
for one of them (for example `s`) does nothing, and the status bar shows
"A commit is running in this repository. Wait for it to finish." These
actions work again when the commit ends or its push starts. The branch switch
waits until the push ends, because `git push` pushes the branch that is
checked out when it starts. Reads, Explorer file operations (also **Delete
file** in the Git panel), and other repositories are not blocked. Another
worktree of the same repository can stage and switch branches, but its
Shelve, Unshelve, and Drop wait: all worktrees share the stash. A stopped
hook can leave its own stash entry, so when the commit ends the shelf list
loads again before you can unshelve or drop, and an open Drop question
closes.

**Cancel** stops Git and its hooks (the full process group). It sends Ctrl-C
first, so Git removes its lock files and a hook manager puts stashed edits
back; after 5 seconds it stops what still runs. A cancelled commit puts its
message and the Amend choice back into an empty draft. If Git made the commit
before Cancel (a `post-commit` hook still ran), the commit stays, the draft
stays empty, and no push follows. If the push fails or is cancelled, the
commit stays and the draft stays empty. Long hook output does not fail the
job: the last part is kept. When a commit or push fails, the commit area shows
the last lines of the hook and Git output; **X** hides them. The status bar shows the first line. Git does not ask for
passwords in these jobs: use a credential helper or an SSH agent. If you quit
while a job runs, Terminator waits 15 seconds, then asks you to cancel the job
first. SSH projects have no commit buttons; make Git changes in the SSH
terminal.

### Commit log graph

The **Commit Log** tab draws a commit graph at the left of each commit row.
Each line of history has its own lane and color. A dot on a lane is a
commit. Lines curve where a branch starts and where a merge joins. Labels
before the subject show `HEAD` (amber), local branches (green), remote
branches (blue), and tags (gray, square corners). When labels do not fit,
the last ones fold into a **+N** label. Hover a row to see all of its labels.

The log shows the newest 1000 commits of all branches, remotes, and tags,
newest first. A commit always comes before its parents. Stash entries are
not shown. A line that reaches the bottom of the list continues into older
commits. Each subject starts after the lanes of its row. When the graph is
wider than 40% of the list, the lanes get narrower; very wide graphs are
cut at that width. The log is read-only.

### Commit log filters

The **Commit Log** tab has a filter bar above its lists:

- **Message or hash** shows commits that have this text in their message.
  Case does not matter. Characters such as `(` or `*` match as typed. A
  commit hash (7 to 64 hex digits) also shows that commit first, also when
  the other filters leave it out.
- **Branch** is **All** (all branches, remote branches, and tags),
  **HEAD**, or one local or remote branch.
- **User** is one author, or **me** (your Git `user.name`). The list has
  the authors of the commits that the tab loaded. Names match as the
  commits store them; `.mailmap` does not change them.
- **Date** is **Any time**, **Last 24 hours**, **Last 7 days**, or
  **Last 30 days**, by commit date.
- **Path** shows only commits that change this file or folder. Type the
  path from the repository root.
- **Clear filters** (eraser) removes all filters. **Reload** loads the log
  again.

Typed text loads when you stop typing for 0.4 seconds. Enter and a menu
pick load at once. Git runs in the background, and the old list stays until
the new one is ready. A spinner shows while Git runs. A newer filter stops
the Git of an older one. If Git fails, the old list stays and the bar shows
the error. If loading stops (for example when you start to quit), the bar
says so; click **Reload**. The log shows the newest 1000 matching commits.
A line under the bar tells how many commits match.

Git never reads a typed value as an option. Text and names go in
`--grep=` and `--author=` values, a branch goes as its full ref name, and
the path goes after `--`.

The Branch and Date filters keep the graph complete. The Path filter uses
the simplified history of Git (`--parents`): each commit joins the next
older commit that changes the path. The Message and User filters hide the
commits between the listed ones. A line does not join a commit to a hidden
parent, so the line stops at that commit, as in `git log --graph`. The
line under the bar then says that lines join only the commits in the list.

**Go to hash** finds a commit. Type a commit hash, a branch, or a tag and
press Enter, or click the target icon. A commit in the list is selected and
scrolled into view. Otherwise the log loads again with that commit added at
the end, and the commit is selected. A newer Go to hash replaces one that
still loads. Text that starts with `-` or has spaces is refused. Go to hash
has no default shortcut.

The Commit Log needs a local repository. SSH projects show a message
instead; use the SSH terminal. The filters run in the GUI and send no
daemon request, so an older daemon works too.

### Compare with a branch, tag, or revision

**Compare with…** compares a file with a branch, a tag, or a commit. It is
in the editor context menu, the Explorer file menu, and the Git panel file
menu. A popup lists local branches (the current branch first), remote
branches, and tags (newest first). Type to filter the list. To use another
revision, type it (for example a commit hash or `HEAD~2`) and press Enter.
Git checks it with `git rev-parse --verify` before the diff opens, and the
diff shows the commit hash it found. Text that starts with `-` or has spaces
is refused.

**Compare with Revision…** lists the commits that changed the file
(`git log --follow`). Type to filter by message, author, or hash.

Use the arrow keys and Enter, or click a row. Esc or a click outside the
popup closes it. A pick opens the read-only native diff of that revision
against the working file. Git runs in the file's folder, so a file in a
nested repository compares with its own repository. Remote (SSH) files get
a message instead; use the SSH terminal.

### Git actions in Search Everywhere

Search Everywhere lists Git actions. Type `git` to see them:

- **Git: Commit…** shows the Git panel and puts the cursor in the commit
  message.
- **Git: Commit and Push** runs the panel's Commit and Push. It shows only
  when there are staged changes and a message.
- **Git: Refresh**, **Git: Show Log**, and **Git: Stage All** do what the
  Git panel buttons do.
- **Git: Compare with…**, **Git: Compare with Revision…**, and
  **Git: Show History** act on the file in the focused pane.
- **Git: Annotate with Git Blame** shows or hides blame in the focused
  editor.

An action shows only where it can run. The Git actions have no default
shortcut; set one in Settings → Shortcuts. The JetBrains Commit
(`Command+K`) and Push (`Command+Shift+K`) keys already belong to editor
Hover and Clear saved scrollback.

### Merge conflicts and image diffs

Click a file in the Git **Conflicts** group (or use **Resolve conflicts…** in
its menu) to open the three-way merge tab. A diff that refuses a conflicted
file also shows **Resolve…**. The tab shows **Ours** (left), **Result**
(center, editable), and **Theirs** (right). The sides come from the index
stages `:2:` and `:3:`; the result starts as the base (`:1:`). The panes stay
aligned chunk by chunk. Red marks a conflict, yellow a one-sided change, and
green the same change on both sides.

- `>` in the left gutter accepts ours; `<` in the right gutter accepts
  theirs; `+` accepts both (ours, then theirs); `x` keeps the base.
- Typing in a change chunk resolves it by hand.
- **Apply non-conflicting changes** accepts every one-sided or identical change.
- F7 and Shift+F7 (or the arrow buttons) go to the next and previous
  unresolved change. The counter shows the conflicts and changes left.
- **Save** writes the file. Unresolved conflicts are saved with diff3 markers.
- **Mark resolved** is available when no change is left. It saves and then
  stages the file (`git add`). Only this click stages the file.
- **Reload** reads the stages from Git again and discards edits.

Long lines are clipped, not wrapped. Binary conflicts must be resolved in a
terminal. The tab is saved in layout version 10.

**Git: Show All Changes** (Find Action), or the file-diff button on a Git
panel group header, opens one tab with every changed file of that side:
staged, or the working tree with untracked files. Each file has a header with
its path and line counts. An open section is the native single-file diff, so
hunk Stage, Unstage, and Discard (with confirmation) work as there and wait for
a running commit. A section loads its diff only while it is on screen. Files
after the first 50 start collapsed. The list follows the Git panel's
repository. The tab is saved in layout version 11.

When a native diff refuses a binary PNG, JPEG, WebP, GIF, BMP, ICO, or TIFF
file, the diff shows the old and new images. The captions give file size and
pixel dimensions. Select **Side by side**, **Swipe** (drag on the image or use
the slider), or **Onion skin** (the slider sets the opacity of the new image).
Other binary files keep the size message.
