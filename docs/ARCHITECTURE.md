# Implementation architecture

All application code lives in this Cargo workspace. The root `../plan.md` records the interview specification.

- `terminator-core`: versioned length-prefixed JSON IPC, stable session/invocation identities, settings, lifecycle and notification transitions, local paths, and bounded helper execution.
- `terminator-daemon`: per-user Unix socket service, process-group/PTY ownership through `portable-pty`, SQLite state, bounded history, embedded editor ownership, and background notifications.
- `terminator-hook`: observational hook delivery, manual integration, installer commands, and a raw-terminal attachment bridge. It never prints permission decisions or starts an agent.
- `terminator-integrations`: managed JSON/TOML/plugin installation, normalized lifecycle events, and resume templates. Upstream hooks differ; unsupported events are not inferred from silence.
- `terminator`: `eframe`/`egui` native application with `egui_dock` layouts, `egui_term` terminals, asynchronous native `rfd` dialogs, filesystem/Git workers, and configurable editor/attention behavior.

## Terminal boundary

The daemon, not the GUI or its bridge processes, owns the real shells and editor PTYs. The GUI uses the existing Alacritty-backed `egui_term` widget. Its child is a small attachment bridge connected to the daemon; losing that bridge does not terminate the original shell.

The daemon uses one `vt100` parser and screen model for bounded screen/history snapshots and terminal query responses through its callback API. The GUI retains the Alacritty-backed terminal widget. There is no duplicate daemon-side Alacritty parser. Reconnection sends generated screen/history state, followed by ordered raw output. Historical display goes through the parser and cannot replay clipboard/OSC side effects. Full compatibility with every vendor-specific terminal extension is not established by the current smoke suite.

Only visible terminal widgets remain attached in the GUI. Switching projects or hiding a tab releases the temporary bridge while preserving the daemon-owned session. Returning reattaches the same session. Slow subscribers are disconnected rather than blocking unrelated sessions; their next attachment obtains a fresh snapshot.

## Identity and persistence

A terminal ID is independent of its project path, PID, or provider conversation. Invocation IDs include the agent process identity and provider session. Hook events are deduplicated, and sequence numbers are honored when provided. Notification read/dismissal state is distinct from the observed agent state.

Environment-based session capabilities are the fast path. For agents such as Muse that clear hook environments, installers include the local endpoint paths. The helper must then find an actual live ancestor shell in this daemon's inventory. It never correlates by working directory or window title. A provider running in an unrelated shared process cannot be assigned to a terminal by this fallback.

SQLite serializes the application state with monotonic revision guards to prevent an older concurrent save overwriting a newer one. A new daemon generation reconciles previously live records as interrupted; it does not revive a PID or execute a resume command. IPC/database versions reject incompatible future formats.

Layout serialization normalizes non-finite initial rectangle coordinates used by the docking library. Tabs, proportions, and focused nodes survive restoration even when saved before the first layout pass.

Removing a project from the sidebar is a persisted navigation preference in
`ui-preferences.json`. Project/session records and layouts remain intact, including
unsaved daemon-owned editors. Empty/sidebar restoration filters hidden IDs rather
than accepting an older daemon's selected-project value. Explicit restoration,
folder reopening or session navigation reveals the existing project. Folder
opening resolves saved path aliases on its worker to avoid duplicate identities.

## Editing

Terminal-editor mode passes only the absolute file operand to custom programs.
Known Vim, vi, Nano/Pico, and Emacs launchers receive their supported position
arguments. Neovim RPC, startup commands and autoread configuration belong to
embedded mode; ordinary embedded editors still load user configuration.

Embedded mode starts a real Neovim process with a private RPC endpoint and the user's normal configuration. It is rendered as a native terminal surface, not a web editor. The application supplies save-all and read-only disk-comparison commands; Neovim owns buffers, plugins, language tooling, autoread, and conflict prompts. Unsaved buffers survive GUI closure while the editor process remains alive. Reboot recovery relies on editor-native recovery, not terminal scrollback.

Markdown is a presentation mode of an existing editor session, not another dock
tab identity or PTY. `markdown.rs` renders through `egui_commonmark`, and
`ui-preferences.json` stores modes by session ID. A separate worker polls only
visible previews at 350 ms intervals. It uses bounded MessagePack requests on the
editor's existing Neovim socket, checking fast `nvim_get_mode` before requesting
text with `nvim_exec_lua`. No client process, new daemon request or runtime plugin
is needed, so older running daemons remain compatible. The expression resolves the
tab's file among loaded buffers and returns text only when its buffer number,
changed tick or modified state changes. Polling neither changes the active buffer
nor writes it. Terminal editors without a socket use a labeled saved-file view.
Blocking prompts or a 400 ms RPC deadline pause live updates while preserving
the last live snapshot; without one, the saved file is displayed. Refresh retains
cached unsaved text during a prompt, and normal polling resumes after the user
answers it. Markdown defaults to Preview; explicit Edit/Split choices persist.
The filename, flat mode tabs, refresh icon and pane close share one header row.
Only the filename owns caption rename/context actions; the separate controls do
not trigger filename clicks or close the editor accidentally.

Watch generations reject delayed results after navigation. Failed refreshes keep
the last preview with an error label. Local Markdown images use the bounded image
decoder on a separate worker, with a 32-image/64 MiB cache and stale-result
rejection. Hidden tabs release preview caches and image textures. Markdown links
are routed to explicit local-file or HTTP(S) opening actions; remote image fetches
and HTML execution are absent. Preview focus suppresses editor input, while the
original editor and its unsaved-close lifecycle remain daemon-owned.

## Platform boundaries

macOS builds use Cocoa/native dialogs and a locally signed `.app` bundle. Linux builds use native windowing with X11/Wayland and portal file dialogs. The daemon's OS-notification callback worker is separate from terminal handling. macOS pumps its native notification run loop on the daemon thread; Linux uses the desktop notification service. No Electron, Chromium, or webview is included.


## Preview, metadata, and automation boundaries

Snapshot clients opt into `snapshot-chunks-v1` using `snapshot_chunks: true` on
the existing request envelope. The daemon advertises the capability; no new
request variant is sent to older daemons. Ordinary snapshots retain the single
`State` frame. Oversized snapshots use base64 `SnapshotChunk` frames with a
`last` marker, each below the unchanged 8 MiB frame limit. Clients deserialize
the payload incrementally. Saved notification details and pending attention are
preserved. Legacy clients receive an explicit update-required error when state
cannot fit their single-frame protocol; newer clients still accept old daemons.

Every attachment exit shuts down both socket directions, including write
timeouts and framing errors, so cloned input readers cannot keep failed output
connections alive. Reattachment still targets the original daemon-owned PTY.

Worktree removal checks every live session's ownership, recorded cwd and editor
file, plus current cwd of its process and descendants. Canonical paths cover
symlink aliases. Missing process identity/directory information refuses removal.
Creation, cwd reports and removal share a coordinator lock. Process inspection
is a point-in-time check; it cannot lock out arbitrary external OS or Git actions.

Image previews belong to the GUI and allocate no PTYs. A dedicated bounded worker
loads raster/SVG data, with stale-generation rejection and a texture-memory budget.
Only paths are persisted in version-3 image-bearing layouts. Unknown layout
versions remain read-only. Explicit text/external actions retain the editor paths.

`gui.sock` is a separate, mode-0600 authenticated GUI endpoint for explicit
presentation commands. It does not move PTY ownership into the GUI. The daemon's
existing protocol is unchanged; added requests are advertised through capabilities.
UI-control retries focus an already-visible session instead of duplicating it.

Git owns worktree state. The SQLite-backed application state records the project,
common Git directory, checkout path, creation time and removal marker. Creation
and removal are serialized against session creation; active sessions and Git's
own dirty/locked protections prevent destructive removal. Removed checkouts retain
history and their branch references.

Metadata work runs on its own coordinator, scoped to the selected directory and
session PID/start time. sysinfo identifies descendants; lsof reports their TCP
listeners. Optional gh lookups are read-only and cached. Terminal OSC messages are
bounded records separate from agent notifications; history replay has no callback
side effects and cannot manufacture agent lifecycle transitions.

All project-owned test/package automation is Rust in `crates/xtask`. The native
fixtures use isolated data/config directories and a test-support input/capture
surface. The optional external-browser driver uses a local CDP WebSocket through
tungstenite; no Chromium, Electron, or webview is bundled into the app.


## GUI updates and exit checkpoints

Window close, native Quit and Sparkle share the GUI's asynchronous exit
coordinator. A worker drain applies pending results; follow-up jobs trigger a
further drain before the final layout/focus/preference checkpoint. Successful
write acknowledgments permit exit; errors or a deadline restore interaction.
The macOS bridge intercepts `NSApplication.terminate:` while retaining winit's
delegate and calls its original implementation on the main queue after saving.

The daemon survives GUI replacement. Optional `daemon_version` metadata and
`shutdown-if-idle-v1` allow a later GUI launch to retire an older idle daemon.
A same-version daemon is also eligible when its helper is unavailable or it
predates `stable-helper-v1`. Unknown/newer versions and daemons without
`shutdown-if-idle-v1` remain untouched. The check and shutdown decision share the
session-creation lock. A failed RPC
with a held daemon lock is a connection error, not permission to replace it.
See `docs/UPDATES.md` for packaging, native fixture and signed rollout boundaries.

## Installation and history health

Before starting a daemon, the macOS GUI requires bundled launches to originate
under `/Applications` or the user's `Applications` directory, and rejects disk
image/translocated bundles with native installation instructions. Unbundled
Rust development executables remain supported. Both sibling executables must be
regular files executable by the current user. Packaging sets their mode to 0755.
This does not request Accessibility access or modify user privacy permissions.

Before publishing its socket, the daemon copies its bundled helper into a new,
mode-0700 `.daemon-helper-*` directory under persistent application data. The
complete executable is synced and mode 0500 before its path is used. Each daemon
keeps that exact inode, independent of replacement, renaming or removal of the
original app bundle. Shell cwd hooks use the private path. GUI attachment bridges
also use it when the daemon advertises `stable-helper-v1` and reports it available;
older daemons retain the GUI's bundled attachment bridge. No helper is downloaded
or substituted into an already running daemon. Normal teardown removes the private
directory; startup removes crash leftovers only after obtaining the daemon lock.

Optional snapshot fields `daemon_executable`, `attachment_helper_executable`, and
`attachment_helper_available` report the running daemon's installation and actual
private helper. Each authenticated snapshot refreshes
health before evaluating conditional-snapshot hints, so a helper removal or
permission change invalidates cached health. Status diagnostics show GUI/daemon
versions and the daemon path; live sessions always prevent automatic retirement.

Settings → Updates → Installation lists live sessions with ordinary navigation
back to their tabs. Explicit repair saves the workspace on the GUI worker,
rechecks the observed daemon generation/version/capability and sends only
`ShutdownIfIdle`. After the socket is removed and lock is released it starts the
installed daemon and verifies its new generation and private helper. Concurrent
creation refuses shutdown. Unknown/newer or unsupported daemons are preserved;
the oldest installations get one-time logout/login guidance after saving and
closing sessions. Legacy missing-helper errors route to this screen even when
the old daemon has no installation-health fields.

The bounded history queue applies backpressure on the dedicated PTY reader,
without holding GUI, parser or state locks. Output bursts are not discarded when
storage temporarily falls behind. Sustained slow storage can slow the producing
terminal process. A disconnected worker or write failure reports actual history
loss and marks the affected session truncated; it does not terminate that session.
A new daemon clears previous runtime health warnings and rechecks its installation;
per-session history-loss flags remain intact across recovery.
