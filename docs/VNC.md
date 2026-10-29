# Native VNC viewer

The daemon owns VNC connections. The GUI owns display state, fullscreen,
monitor selection, local shortcut handling, and file pickers. Closing a VNC
tab detaches its session, like a terminal: the daemon keeps it connected and
the VNC sidebar marks it Detached; clicking it reattaches. Disconnect is an
explicit action. A session no viewer has watched for Settings → History →
Detached VNC sessions (default 30 minutes, 0 = never, `vnc-detach-timeout-v1`)
is disconnected; older daemons use a fixed 30 minutes. Quitting with live
VNC sessions asks once: Quit, Disconnect all and quit, or Cancel. An app
restart (update or repair) does not ask. Connecting a host that already has a live session reattaches
it instead of opening a second connection. VNC records of retired or
unreachable daemons are not shown; they cannot be live.

## Modules

- `crates/core/src/vnc/options.rs`: bounded connection policy and shared settings.
- `crates/daemon/src/vnc_session/transport.rs`: TCP, OpenSSH, and TLS configuration.
- `crates/daemon/src/vnc_session/credentials.rs`: serialized OS credential operations.
- `crates/daemon/src/vnc_session/connection.rs`: one connection attempt and input loop.
- `crates/daemon/src/vnc_session/reconnect.rs`: bounded retry policy and cancellation.
- `crates/daemon/src/vnc_session/input.rs`: input, clipboard and resize validation.
- `crates/daemon/src/vnc_session/quality.rs`: encoding and refresh presets.
- `crates/daemon/src/vnc_session/transfer.rs`: explicit, bounded SFTP transfers.
- `crates/daemon/src/vnc_session/shm.rs`, `crates/sys/src/shm.rs`: shared-memory frames.
- `crates/daemon/src/vnc_session/stats.rs`, `thumbnail.rs`: speed numbers and saved
  host thumbnails.
- `crates/daemon/src/vnc_session/ssh_steps.rs`, `status.rs`: start command and
  typed session status.
- `crates/app/src/vnc_ui/`: frame decoding, view state, connection profiles/settings,
  keyboard, clipboard, display, and transfer controls in separate modules.
  `page.rs`, `address_book.rs`, `card.rs`, `book_keys.rs` and `import.rs` are the
  address book; `profiles.rs` is host Properties; `ssh_probe.rs` reads SSH config.
- `vendor/vnc-rs`: RFB framing and decoding. See its `UPSTREAM.md` for changes.

## Address book and Properties

The VNC tool opens the VNC page in the main pane (like Settings and Player).
It is the one place to add and edit hosts; the VNC sidebar only lists
sessions and hosts. The address bar filters cards and connects to
`host:display` or `host::port` (TigerVNC style); an unknown address opens a
new host's Properties. Cards show the live screen, or the last screen the
daemon saved (`data/vnc-thumbnails/<profile>.png`, every 30 s while
connected). Double-click or Enter connects; arrows move the selection;
Delete removes; Command/Ctrl+I opens Properties. Labels filter cards; a
Grid/List switch changes the layout. Import… adds hosts from concrete
`~/.ssh/config` names (as SSH tunnels) or RealVNC `.vnc` files.

Properties has General, Security and Options pages. A saved password shows
as a mask; Forget password removes it. For a new host, `ssh -G` runs in the
background: when `~/.ssh/config` configures the address, the SSH tunnel is
selected and a notice names what was found. A blank SSH host tunnels to the
address. Reconnect turns on for SSH hosts. Start command on host (optional,
`vnc-start-command-v1`) runs a user-entered command over SSH before each
connection, for example a script that starts the VNC server; it is not
part of Terminator and is never generated.

Errors carry a kind (`vnc-errors-v1`): wrong or missing password, SSH key or
host key, unreachable, TLS, start command, protocol. The status line offers
one fix: Enter password, Open Hosts, Properties or Retry. The daemon never
sends an empty password; a server that needs one opens the password prompt.
A saved password is read from the OS keychain on the session thread, so a
keychain permission prompt cannot time out the connect request.

## Connections and passwords

Enter a bare host name or IP address in Host. Port and Display are linked:
port = 5900 + display. New forms start at port 5966 / display 66; saved hosts
keep their ports. Ports below 5900 have no display number. Host values with
an embedded port or URL scheme are rejected so they cannot override the fields.
VNC views can belong to local or SSH projects.

Connection options are saved per host. Direct VNC does not encrypt traffic.
TLS uses VeNCrypt 0.2 with X509Vnc or X509Plain and verifies the certificate
against OS trust or the explicitly selected CA file. The certificate name
must match the VNC host or the configured TLS server name. A TLS failure
never retries through an unencrypted transport.

SSH uses system OpenSSH with batch authentication, strict host-key checks,
SSH config and agent/key authentication. Unknown or changed host keys fail.
SFTP accesses the configured SSH host's filesystem, which can differ from
the VNC target reached through that host. SSH errors do not fall back to TCP.

Saved passwords use the OS keychain/credential service. New passwords are not
saved in SQLite or returned in snapshots. Archived pre-upgrade stores remain
unchanged; migration does not erase historical database pages or backups. Existing plaintext passwords are
migrated only after a verified credential-store write; if migration fails,
startup stops before saved state is overwritten. Prepared generations do not
write the shared catalog; migration completes under the workspace lock after
activation. Unlock the credential store
and retry. Changing a saved authentication endpoint requires a new password
or explicit removal of the old credential. A verified replacement removes the
previous endpoint's keychain entry. If removal fails, the new entry is removed
and the previous profile is retained. Entries left by older versions cannot
be identified from the current profile alone. Unsaved passwords are prompted for
again on a new connection. They remain in memory only for the current session.

Full frames that exceed the 8 MiB IPC message limit use the existing opt-in
chunk stream. Each message remains bounded; assembled responses are limited
to 256 MiB. Updated clients accept chunked VNC responses. Older clients still
receive ordinary frames or a size error; older daemons retain their size limit.
Queued input from a lost connection is discarded during retry. Disconnect
still cancels retry immediately.

## Input and clipboard

Unicode and committed IME input use X11 Unicode keysyms. The Keys menu sends
remote Alt+Tab, Command+Tab, Super, and Escape. Capture app shortcuts prevents
Terminator shortcuts from taking focused remote input; F8 releases capture.
This does not install an OS-wide keyboard interceptor. System-reserved keys
can be sent through the Keys menu.

Clipboard sharing can send, receive, do both, or be disabled. The Clipboard
menu selects the remote paste shortcut and the corresponding copy/cut
shortcuts. Baseline RFB clipboard data is Latin-1, limited to 256 KiB. Paste
as keystrokes is an explicit fallback for Unicode and login fields, limited
to 4096 characters. It can send Return and Tab; choose the remote field first.
Files, images and rich clipboard formats are not part of baseline RFB text
clipboard transfer. Use SFTP for files.

## Display, recovery and transfers

The scale menu offers Fit, Fit width, 100% and 200%, and the Resize remote to
pane toggle (enabled when the host allows remote resize). Only Fit sends the
mouse wheel to the remote; the other modes scroll. Fullscreen shows the VNC
pane without workspace chrome; F11 enters it from a focused remote screen and
exits it. Fullscreen turns on shortcut capture and restores the previous
choice on exit; the toolbar shows "Capturing shortcuts · F8 to release".
Ctrl+Alt+Del from the toolbar asks for confirmation in the pane. View-only
hosts show an eye glyph on their card. The server must advertise and confirm extended
desktop support before remote resize is sent. Resize requests are debounced;
server rejection does not change the reported desktop geometry. Multiple
advertised monitors can be shown together or selected separately. Desktop
allocation is bounded to 8192 pixels per axis and 32 Mi pixels total. Remote
cursor images are limited to 256 pixels per axis.

Sharp text uses lossless encodings. Low bandwidth prefers Tight compression
with a lower JPEG quality and fewer refresh requests. Automatic mode switches
to the JPEG set after updates stay slower than 80 ms for 3 s, and back below
30 ms. Hidden views stop requesting frames.

With `vnc-shm-v1`, the daemon keeps each framebuffer in a private
memory-mapped file in the runtime directory and the GUI copies only the
changed rectangles (up to 32); a sequence counter discards torn reads.
Frame requests long-poll for a new revision, and the next update request is
sent as soon as an update ends. Older daemons and GUIs keep the compressed
path. The cursor shape is always requested and drawn at the local pointer,
so it never lags. 100% shows exact pixels; Fit is smoothed. The toolbar
speed chip shows viewer frames per second and how long an update takes to
arrive. Queued pointer moves collapse to the newest; presses keep their place.

Reconnect is optional and bounded to at most ten attempts, with delays up to
30 seconds. Authentication, certificate, SSH and protocol failures stop retries.
Manual disconnect cancels pending retries. Queued input is not replayed after
a connection fails.

SFTP transfers require a writable SSH session and an explicit file selection.
Files dropped on the screen of a writable SSH session upload to the Upload
folder set in the Files panel (an absolute path; `~` cannot be expanded on
the client). One file uploads per drop.
They are limited to 1 GiB and five minutes, with at most two active transfers
per session. Existing destinations are not overwritten. Upload publication
uses the server's hard-link and fsync extensions; servers without them are
rejected before upload. Downloads use a local temporary file and atomic
non-overwriting publication. A failed or timed-out upload can leave a named
`.terminator-<transfer-id>.partial` file; it is never reported as complete or
automatically retried. The Files panel reports progress and errors.

Advanced operations require the owning daemon's `vnc-features-v1` capability.
New GUI code does not upgrade a running daemon automatically.

A transfer timeout leaves publication status uncertain. Check the destination
before retrying; transfers are never automatically replayed.
