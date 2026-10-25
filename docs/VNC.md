# Native VNC viewer

The daemon owns VNC connections. The GUI owns display state, fullscreen,
monitor selection, local shortcut handling, and file pickers. Closing a VNC
tab disconnects that view's session; closing the GUI does not stop terminals.

## Modules

- `crates/core/src/vnc/options.rs`: bounded connection policy and shared settings.
- `crates/daemon/src/vnc_session/transport.rs`: TCP, OpenSSH, and TLS configuration.
- `crates/daemon/src/vnc_session/credentials.rs`: serialized OS credential operations.
- `crates/daemon/src/vnc_session/connection.rs`: one connection attempt and input loop.
- `crates/daemon/src/vnc_session/reconnect.rs`: bounded retry policy and cancellation.
- `crates/daemon/src/vnc_session/input.rs`: input, clipboard and resize validation.
- `crates/daemon/src/vnc_session/quality.rs`: encoding and refresh presets.
- `crates/daemon/src/vnc_session/transfer.rs`: explicit, bounded SFTP transfers.
- `crates/app/src/vnc_ui/`: frame decoding, view state, connection profiles/settings,
  keyboard, clipboard, display, and transfer controls in separate modules.
- `vendor/vnc-rs`: RFB framing and decoding. See its `UPSTREAM.md` for changes.

## Connections and passwords

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
or explicit removal of the old credential. Unsaved passwords are prompted for
again on a new connection. They remain in memory only for the current session.

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

Fit and 100% modes remain available. Fullscreen shows the VNC pane without
workspace chrome; F11 exits. The server must advertise and confirm extended
desktop support before remote resize is sent. Resize requests are debounced;
server rejection does not change the reported desktop geometry. Multiple
advertised monitors can be shown together or selected separately. Desktop
allocation is bounded to 8192 pixels per axis and 32 Mi pixels total. Remote
cursor images are limited to 256 pixels per axis.

Sharp text uses lossless encodings. Low bandwidth prefers Tight compression
with a lower JPEG quality and fewer refresh requests. Automatic mode adapts
the request interval to recent frame size. Hidden views stop requesting frames.

Reconnect is optional and bounded to at most ten attempts, with delays up to
30 seconds. Authentication, certificate, SSH and protocol failures stop retries.
Manual disconnect cancels pending retries. Queued input is not replayed after
a connection fails.

SFTP transfers require a writable SSH session and an explicit file selection.
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
