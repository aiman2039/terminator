# Implementation architecture

All application code lives in this Cargo workspace. The root `../plan.md` records the interview specification.

- `terminator-core`: versioned length-prefixed JSON IPC, stable session/invocation identities, settings, lifecycle and notification transitions, local paths, and bounded helper execution.
- `terminator-daemon`: per-user Unix socket service, process-group/PTY ownership through `portable-pty`, SQLite state, bounded history, embedded editor ownership, and background notifications.
- `terminator-hook`: observational hook delivery, manual integration, installer commands, and a raw-terminal attachment bridge. It never prints permission decisions or starts an agent.
- `terminator-integrations`: managed JSON/TOML/plugin installation, normalized lifecycle events, and resume templates. Upstream hooks differ; unsupported events are not inferred from silence.
- `terminator`: `eframe`/`egui` native application with `egui_dock` layouts, `egui_term` terminals, asynchronous native `rfd` dialogs, filesystem/Git workers, and configurable editor/attention behavior.

## Terminal boundary

The daemon, not the GUI or its bridge processes, owns the real shells and editor PTYs. The GUI uses the existing Alacritty-backed `egui_term` widget. Its child is a small attachment bridge connected to the daemon; losing that bridge does not terminate the original shell.

The daemon uses one `vt100` parser and screen model for bounded screen/history snapshots and terminal query responses through its callback API (xterm DA, DECRQM for alt-screen/mouse, OSC 4/10/11/12). The GUI retains the Alacritty-backed terminal widget. There is no duplicate daemon-side Alacritty parser. Reconnection sends generated screen/history state, followed by ordered raw output. Historical display goes through the parser and cannot replay clipboard/OSC side effects. Full compatibility with every vendor-specific terminal extension is not established by the current smoke suite.

Only visible terminal widgets remain attached in the GUI. Switching projects or hiding a tab releases the temporary bridge while preserving the daemon-owned session. Returning reattaches the same session. Slow subscribers are disconnected rather than blocking unrelated sessions; their next attachment obtains a fresh snapshot.

## Identity and persistence

A terminal ID is independent of its project path, PID, or provider conversation. Invocation IDs include the agent process identity and provider session. Hook events are deduplicated, and sequence numbers are honored when provided. Notification read/dismissal state is distinct from the observed agent state.

Environment-based session capabilities are the fast path. For agents such as Muse that clear hook environments, installers include the local endpoint paths. The helper must then find an actual live ancestor shell in this daemon's inventory. It never correlates by working directory or window title. A provider running in an unrelated shared process cannot be assigned to a terminal by this fallback.

`catalog.sqlite3` holds shared projects, layouts, settings, worktrees and the active
owner. Registered generations each keep a separate `state.sqlite3`, history,
socket, authentication token, daemon lock and immutable executable copies. Only
the active owner saves shared workspace data; session stores reject foreign
ownership. Session UUIDs and their original generation remain unchanged across
upgrades. Catalog and owner revisions are tracked independently. IPC/database
versions reject incompatible future formats.

The core client routes workspace/creation requests to the active owner and
session requests to the original owner, aggregating their inventories. Attachment
bridges and shell hooks use the owner's endpoint and private helper. Environment-free
hooks require exactly one matching live ancestor across the inventory. GUI editor
close and Markdown preview sockets also resolve through the session owner.

Activation, creation admission, cwd updates and worktree mutations share a file
lock. A draining owner redirects requests before execution; clients retry only
that explicit rejection, never a timeout or uncertain creation response. Worktree
removal checks every owner and refuses when an unavailable owner prevents proof
of safety. Maintenance runs outside the socket accept loop. The active service
coordinates the global history budget by asking live owners to prune their own
files; retired history remains readable and is pruned by the active service.
Shared settings refresh without restarting any owned process.

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

macOS builds use Cocoa/native dialogs and a locally signed `.app` bundle. Linux builds use native windowing with X11/Wayland and portal file dialogs. The daemon's OS-notification callback worker is separate from terminal handling. macOS pumps its native notification run loop on the daemon thread; Linux uses the desktop notification service. Desktop banners may include a platform sound name when `notification_sound` is on (`notification-sound-v1`). In-app attention stays silent. No Electron, Chromium, CEF, or Servo is included. GUI-only OS webview tabs use WKWebView / WebKitGTK via wry and die with the GUI.


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
Only paths are persisted in version-3 image-bearing layouts. HTML and http(s) pages use GUI-only `Tab::Browser` (layout version 6) with an OS
webview child view. v4 `Html` tabs migrate to `Browser` file targets. Covered
panes hide the native view. Audio player tabs are GUI-only layout version 5: one
player per project, `rodio` on a worker for local files and HTTP(S) Icecast/Shoutcast
streams. Playback stops when the GUI exits or the player tab closes. Radio UI is
native egui. Unknown layout versions remain read-only. Explicit text/external
actions retain the editor paths. Open in browser still uses the system-browser worker.

`gui.sock` is a separate, mode-0600 authenticated GUI endpoint for explicit
presentation commands. It does not move PTY ownership into the GUI. The daemon's
existing protocol is unchanged; added requests are advertised through capabilities.
UI-control retries focus an already-visible session instead of duplicating it.
GUI Ping/Snapshot requests observe the GUI even while its daemon is unavailable;
presentation actions refresh daemon state before resolving their targets.

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

Worker/IPC responses, native exit requests, exit checkpoints and heartbeats run
in `eframe::App::logic`, which is called on repaint requests even when the window
is minimized or hidden. Rendering stays in `App::ui`. Heartbeats pause during the
exit checkpoint so they cannot keep its job drain busy. Queued GUI requests carry
the server's response deadline; expired requests are rejected before acting,
preventing a timed-out request from executing when an old UI queue resumes.

Window close, native Quit and Sparkle share the GUI's asynchronous exit
coordinator. A worker drain applies pending results; follow-up jobs trigger a
further drain before the final layout/focus/preference checkpoint. Successful
write acknowledgments permit exit; errors or a deadline restore interaction.
The macOS bridge intercepts `NSApplication.terminate:` while retaining winit's
delegate and calls its original implementation on the main queue after saving.

Generation-aware services advertise `daemon-generations-v1`. On launch, the app
stages the installed daemon/helper, starts a candidate with creation disabled,
and verifies authenticated generation/build/catalog identity, executable contents
and helper execution. Under the coordination lock it selects the candidate and
marks the previous owner draining. Existing PTYs remain with their original
owner, including unsaved editors. Draining services flush and retire when idle,
even without a GUI. Their executable/helper copies are removed; databases and
history remain. A failed candidate preserves the active service and exposes a
retry in Settings. A candidate that never spawned or whose child exited before
initialization is removed from the registry under coordination and its owner lock;
its private files are removed and its startup log is retained separately. Active
or uncertain owners cannot be discarded. A catalog-active owner that is still
listening is not treated as archived; a Retired mark on that owner is restored
while it remains the serving generation. Newer or incompatible services are not downgraded.

The first migration is different: a legacy daemon keeps serving all live sessions.
Once idle, the GUI checkpoints, requests acknowledged idle shutdown and acquires
the legacy lock. Migration backs up the database, copies historical records and
history, advances the legacy version guard and atomically publishes the catalog.
The original data and backup remain recoverable if import fails. If the legacy
service already exited, the exclusive legacy lock permits reconciling its stale
live records as interrupted in the imported copy. Recorded PIDs are neither
signalled nor adopted, and original records remain in the backup. Generation
services retain shared legacy-lock guards, preventing old executables from
starting a competing legacy daemon. Appearance and navigation paths are unchanged.
Unknown legacy versions and services without safe idle shutdown remain untouched.

An unavailable socket does not establish death. Recovery requires the owner lock
and an absent recorded process; a reused PID is conservatively unavailable.
Only that owner's records become interrupted, and no commands or editors are
rerun. Future sessions use a verified replacement.

**Stop all sessions and restart** remains explicit recovery. It freezes creation,
carries the exact generation and session IDs captured at the GUI confirmation
through the worker and detached helper, and rejects any newly live session before
closing the GUI or sending a stop. It checkpoints/closes the GUI, stops sessions across
owners without force-kill, waits for retirement, and reopens the installation.
Cancellation preserves every owner. Partial failure is reported. The freeze is
released on completion; an absent recovery process permits stale-freeze recovery.
Sparkle Install and Relaunch remains a GUI operation. See `docs/UPDATES.md` for
packaging and signed rollout boundaries.

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

For legacy services, Settings → Updates → Installation lists live sessions with ordinary navigation
back to their tabs. Explicit repair saves the workspace on the GUI worker,
rechecks the observed daemon generation/version/capability and sends only
`ShutdownIfIdle`. After the socket is removed and lock is released it starts the
installed daemon and verifies its new generation and private helper. Concurrent
creation refuses shutdown. Unknown/newer or unsupported daemons are preserved;
the oldest installations get a copyable manual shutdown command scoped to the
GUI's helper/data/runtime paths. Users must finish sessions and fully quit the
GUI before executing it in another terminal; the GUI never sends legacy shutdown.
**Restart session service** is the in-app equivalent for a replaceable live
daemon: it spawns the GUI's sibling helper, not the daemon's private copy.
Logout/login remains an alternative. Legacy missing-helper errors route here even when
the old daemon has no installation-health fields.

The bounded history queue applies backpressure on the dedicated PTY reader,
without holding GUI, parser or state locks. Output bursts are not discarded when
storage temporarily falls behind. Sustained slow storage can slow the producing
terminal process. A disconnected worker or write failure reports actual history
loss and marks the affected session truncated; it does not terminate that session.
A new daemon clears previous runtime health warnings and rechecks its installation;
per-session history-loss flags remain intact across recovery.

Explicit `terminator-hook ctl shutdown --stop-all` performs user-requested cleanup
using existing daemon requests. It requests normal GUI closure, holds `ui.lock`
after the checkpoint completes, records the daemon generation and session set,
then stops those sessions and waits for them to end. Unrelated concurrent sessions,
generation changes and timeouts abort the sequence. Idle shutdown flushes history
and persists state; completion waits for socket removal and daemon lock release.
The command refuses execution inside a managed session and does not force-kill
processes. Unsaved editor buffers are discarded only under the explicit stop-all
option. Normal daemon teardown explicitly removes its private helper directory
before releasing the lock, even when background worker Arcs outlive main.

## Verified idle-shell closure

`close-idle-sessions-v1` gates a daemon-generation-scoped batch request. The GUI
uses one asynchronous coordinator for existing pane/workspace close intents;
editors and mixed editor/terminal tabs retain their existing confirmation flow.
A pending workspace close captures its contents and preserves the view if they
change before the result arrives. Older daemons retain confirmation.

A dedicated terminal-operation lock serializes input admission, automatic terminal
reply admission, agent events, and final idle-close checks. Blocking PTY writes run
outside that lock; an in-flight counter makes idle closure ineligible until they
finish, so a full input buffer cannot block Stop or other panes. The daemon checks
the originally launched shell executable/PID/start time, actual thread waiting state
on macOS, foreground PTY process group, live children, and agent lifecycle. Idle
means the launched shell owns the foreground group and has no descendants; a live
agent, child, or non-shell foreground keeps the Keep-running/Terminate dialog.
Unverifiable inspections keep confirmation. Every target is preflighted before any
signal; each target is rechecked immediately before SIGHUP. Results distinguish
confirmed exits from signals whose exit could not be verified. Partial signalling
cannot be rolled back; the GUI preserves its view and reports outcomes. The GUI
never infers readiness from terminal text. Prompt-hook acknowledgments are not a
close gate; older generated shells may still emit them and the daemon ignores them.

DEC focus-reporting (1004) and alternate-scroll (1007) settings are retained through
vt100 extension callbacks and replayed after the formatted attachment snapshot.
A reset-boundary feeder clears extension modes in stream order on RIS; it does not
create another screen model or infer lifecycle from text. Long-running native
fixtures may opt into rendering while occluded through the test-support build;
normal application occlusion/minimization behavior is preserved.

Browser navigation uses persistent pane IDs (compatible with layout v6); legacy
browser entries receive IDs when loaded. Links, redirects and history navigation
use the same HTTP(S)/local-HTML policy as opening a tab. Closing a pane or its
containing top-level tab destroys its webview. WebKitGTK stores its profile below
the data directory. macOS 14+ uses a named WebKit store whose UUID is saved under
that directory; macOS 12–13 uses nonpersistent private browsing to avoid sharing
the default store. OS-managed macOS store contents are not in the data directory.

Playback has an explicit owning project and is polled from app logic, including
when the GUI is minimized. Switching projects does not change the current
playlist. Closing the Player window or a leftover Player tab does not stop
playback; the GUI exit
path does. Only one audio source plays at a time. Radio uses connection and
per-read timeouts without a total stream deadline; initial output honors saved
volume, including mute. Mini-controls bind to the owning project, not the
selected one. The full Player window is one themed floating window (never a workspace tab):
transport, optional EQ, playlist. Spectrum paints only while playing. Opening
the player closes leftover Player tabs. EQ sliders are visual only.
Playlist stacks match Webamp ADD/REM/SEL/MISC/LIST. Shuffle uses a remaining-track
bag; repeat wraps sequential play and reshuffles the bag.
