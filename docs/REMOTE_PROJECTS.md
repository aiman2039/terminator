# SSH projects

An SSH project has a name, OpenSSH connection settings, and an absolute POSIX
root path. It lives on the SSH host. The local application keeps only settings,
layouts, scrollback, and bounded read-only preview snapshots.

New terminals are daemon-owned local PTYs running OpenSSH with a remote PTY.
The remote command changes to the configured project directory and starts that
host's login shell. Splits create another SSH terminal in the project root.
Closing the GUI leaves those SSH processes running. A network disconnect ends
the SSH process; this first implementation does not restart remote shells or
add tmux automatically. Historical sessions are never silently restarted.

Remote projects have their own catalog collection and table. Local project
services never receive their remote root. The GUI uses a non-filesystem
identity to share existing docking code; it is not a local checkout. Remote
Explorer and Git use explicit daemon requests and never fall back to local
filesystem or Git operations. Older active daemons cannot create SSH projects.
The separate table preserves remote records if older code saves local settings.

## Authentication and commands

OpenSSH config aliases (with their `User`, `Port`, `ProxyJump` and
`IdentityFile`), explicit users/ports, identity files, and an SSH agent are
supported. A blank port uses the SSH config. Every OpenSSH call for a host
shares one multiplexing master through a control socket in the runtime
directory (`ControlMaster=auto`, `ControlPersist`), so Explorer, Git,
editor control and transfers ride an existing connection.

Settings → SSH sets the keep-alive interval (`ServerAliveInterval`, default
15 s, three missed replies end the connection), how long the shared connection
stays open after its last terminal (`ControlPersist`, default 10 min) and the
connect timeout (default 10 s). They apply to OpenSSH processes the daemon
starts after the change. The section needs `ssh-settings-v1`; older daemons
keep the defaults.

SSH terminals are interactive: OpenSSH prompts in the terminal for passwords,
key passphrases and 2FA codes, and asks about an unknown host key there. The
signed-in terminal becomes the shared master, so files and Git then work
without a second login. Other calls stay noninteractive (`BatchMode=yes`).
When one fails authentication with no live master, the view says so and offers
"Open SSH terminal" to sign in there.

When a project probe, Explorer, Git or transfer call meets an unknown host key,
the daemon scans the host (`ssh -G`, `ssh-keyscan`) and the GUI shows the
SHA256 fingerprints for confirmation. "Trust and connect" rescans and appends
the key to the configured known-hosts file (or `~/.ssh/known_hosts`) only if
the host still presents exactly the keys shown. A changed host key is never
trusted automatically: the dialog shows the new fingerprints and the
`ssh-keygen -R` command to run after you have verified the change. The daemon
also refuses `TrustHostKey` while the known-hosts file holds a different key of
the same type for that host, since OpenSSH would accept either line. This needs
a daemon advertising `ssh-hostkeys-v1`.

Agent/X11 forwarding, local commands, and port forwards from SSH configuration
are disabled; only the project's own forwards (below) are set up. Local
Terminator session tokens and routing variables are removed from SSH child
environments. They are not forwarded or installed on the host. Remote agent
lifecycle hooks are not provided by this feature.

Remote text editing runs Neovim on the SSH host, with that host's configuration.
Save/close checks use a private per-session remote Neovim socket over SSH. An
unavailable editor state remains unknown; it does not permit a silent discard.
The remote host needs a POSIX shell, Git for the Git pane, and Neovim for editing.

## Files and Git

Explorer lists files through SFTP, with a 1000-entry limit. Preview reads are
limited to regular files of 2 MiB or less. SFTP paths are canonicalized and
must remain inside the configured project root. Interactive SSH shells retain
the SSH account's normal permissions; the project root is not a shell sandbox.

Images, HTML, audio and Markdown previews use explicitly downloaded read-only
snapshots in the app's private data directory. Markdown snapshots are read-only;
choose Open in SSH editor to edit the remote file. HTML dependencies are not
mirrored. Remote Markdown file links and local images are disabled in snapshots;
use Explorer to open linked files. Open as text runs the remote editor instead of a special viewer.

Git status and diffs run on the host. Diffs compare HEAD/index/worktree snapshots
in the existing native viewer. Remote diffs cannot stage, discard, or write
files. Use the SSH terminal for Git mutations. Remote filesystem operations,
Git reads, and editor control have bounded process/output deadlines.

## Uploads and downloads

With a daemon advertising `ssh-transfer-v1`, Explorer's toolbar upload button
(and "Upload into this folder…" on a folder) picks a local file and uploads it
into the current folder under its own name. "Download…" on a file saves it to a
new local file. Files dropped on the window while a project's Explorer is shown
upload into its current folder. Native pickers run off the GUI thread.

Transfers run in the daemon as background jobs over SFTP through the project's
OpenSSH master; the request that starts one returns at once, and the GUI polls
progress every 500 ms while it runs (at most two per project, one hour each,
files up to 1 GiB). Remote paths are canonicalized and must remain inside the
project root. Uploads write a create-new temporary file beside the destination,
fsync it and publish it with a hard link, so an existing remote file is never
replaced (the server must support the hard-link and fsync extensions).
Downloads refuse an existing local destination before connecting and publish
with a no-clobber rename. Choosing "Replace" in the save dialog does not
overwrite: the transfer fails instead. SSH failures are classified like other
calls (sign-in, host-key trust). Progress lives in daemon memory; transfers
started by an earlier daemon generation are not shown. A project cannot be
removed while a transfer runs. VNC SFTP transfers share the same code.

## Hosts sidebar

The Hosts tool lists SSH projects in collapsible groups (state kept in
`ui-preferences.json`), each row showing `user@host:port · root · last
connection`. The History toggle sorts each group by last connection instead of
by name. The dot is green while `ssh -O check` finds a live OpenSSH master on
the project's control socket; the check runs off the GUI thread every 10
seconds, only while the sidebar is visible. The terminal icon (or a
double-click on the name) opens a terminal; the row menu holds Properties,
Duplicate, Set group, Port forwards and Remove.

The filter field also connects: Up/Down move a selection over the visible
hosts and config aliases, and Enter opens the selected or first match. With no
match, Enter quick-connects a typed `host`, `user@host`, `host:port` or
`user@host:port`. Quick connect probes the login directory with `pwd -P`, adds
a project rooted there and opens a terminal. A failed quick connect shows one
fix: "Set up…" (sign-in needed: opens Properties, where the user sets the path
and signs in through a terminal), "Review host key", or "Retry".

New, Properties and Duplicate open SSH host Properties in the main pane (like
VNC Properties): config alias picker, host, port and user, name, remote path
("Login directory" probes it), group, and folded key files (`~/` expands to the
home directory). For a host that is not a config alias, `ssh -G` runs off the
GUI thread and the page shows what `~/.ssh/config` sets for it. "Test
connection" probes without saving. Errors carry the same one fix button; for a
sign-in error it saves and opens a terminal. Saving an existing project sends
`UpdateRemoteProject` (`ssh-edit-v1`): name and group always change; a new
host, user, port, keys or root is refused while the project has live
terminals, VNC sessions, transfers or active forwards, and afterwards stops the
old OpenSSH master unless something else shares it. Layout, forwards and the
last-connection time are kept. Duplicate copies a project without id, layout or
forwards; saving a copy with the same host and root is refused. The daemon
records `last_connected` when an SSH shell starts (`ssh-edit-v1`).

The key icon in the Hosts header opens a main-pane import page that lists
`~/.ssh/config` aliases with no saved project. Chosen hosts are added one at a
time, either at their login directory (one batch connection each, so hosts
needing a password or 2FA fail and are listed) or at one path for all (no
connection), with an optional group.

Grouping, removal and quick connect need a daemon advertising `ssh-hosts-v1`. Removal is refused while the project has live terminals, VNC sessions or running transfers, and it stops the project's OpenSSH master unless another project or live session uses the same socket.

## Port forwarding

An SSH project can save up to 32 local forwards (`local port → remote host:remote
port`, the remote host as seen from the SSH host). Listeners bind `127.0.0.1`
only; wildcard or other bind addresses cannot be configured. Forwards are edited
and switched on or off per forward in the Hosts sidebar (globe icon) and in the
project's Info view. This needs a daemon advertising `ssh-forwarding-v1`.

Starting a forward sends `ssh -O forward -L 127.0.0.1:lp:host:rp` to the
project's OpenSSH master (`-O cancel` stops it). With no live master, the daemon
starts a batch master (`ssh -fN -oControlMaster=yes`), which needs key or agent
authentication; otherwise sign in through an SSH terminal first and retry. Such a
dedicated master has no idle timeout and is exited when its last forward stops,
unless a live terminal or VNC session joined it. A master started by a terminal
keeps its `ControlPersist` (Settings → SSH); idle forward listeners do not keep it
alive. Forwards end with their master, and status (rechecked with `ssh -O check`
every 10 seconds while forwards are shown) reflects that. A local port already
in use is reported before the forward is requested.

`ClearAllForwardings=yes` stays on every connecting client, masters included:
OpenSSH applies it once to forwards from configuration and the command line,
and later `-O forward` requests are handled by the master regardless. Only the
`-O forward` client omits it (and reads no configuration), since there it would
drop the `-L` before sending.

Forwards marked "Start when a terminal opens" are started after a terminal for
the project signs in (waiting up to two minutes); failures show under the
forward. Active-forward tracking is in memory: a forward started by an earlier
daemon generation still listens but shows as stopped until it is started again,
which adopts it. A server with `AllowTcpForwarding no` accepts the listener but
refuses each connection.

## Local fixture

```sh
bash scripts/ssh-project-fixture.sh start
bash scripts/ssh-project-fixture.sh status
bash scripts/ssh-project-fixture.sh stop
bash scripts/ssh-project-fixture.sh rekey [ssh-project|ssh-project-pw]
bash scripts/ssh-project-fixture.sh down
```

The start command prints the connection settings. Defaults:

- Host: `127.0.0.1`, SSH port: `15932`, user: `developer`.
- Root: `/workspace/project`, stored in a Docker named volume.
- Disposable client keys: `$SSH_PROJECT_FIXTURE_KEY_DIR`, or a temporary folder.
- Known-hosts data is copied from the controlled container, not blindly trusted
  from a network key scan.
- A second service, `ssh-project-pw` on port `15933`, sets
  `PasswordAuthentication yes` (user `pw`, password `pw`) and
  `AllowTcpForwarding local`. `developer` keeps key access there, and the
  container serves `marker.txt` on its own `127.0.0.1:8000` for forward checks.
- `SSH_PROJECT_FIXTURE_PROJECT`, `SSH_PROJECT_FIXTURE_PORT` and
  `SSH_PROJECT_FIXTURE_PW_PORT` select the Compose project and ports.

`rekey` replaces a service's host key and restarts it, leaving the printed
known-hosts file stale, so the next connection reports a changed host key.
`down` removes the containers and their volumes.

The container has Git, Neovim, test files, a modified tracked file and an
untracked file. It uses an unprivileged SSH account and publishes only localhost.
No host repository is mounted. The fixture is limited to one CPU, 512 MiB, and
128 processes. Stop preserves fixture data and host keys; no production server,
user SSH configuration, or installed app is modified.

## Testing

`cargo xtask ssh` is the opt-in live check. It needs Docker and OpenSSH; without
them it prints why and exits successfully (`--require` makes that a failure).
It is never part of `cargo test` or `cargo xtask integration`. Build the
workspace first (`cargo build --workspace --locked`). It starts its own fixture
copy on free ports under a unique Compose project and an isolated daemon, then
checks, over the daemon's IPC:

1. A probe with an empty known-hosts file fails with `UnknownHostKey`.
2. `ScanHostKey` reports the container's fingerprint; after `TrustHostKey` the
   probe succeeds.
3. Twenty list/read/Git calls open no new sshd connection (counted from sshd's
   `Connection from` log lines): they reuse one ControlMaster.
4. On `ssh-project-pw`, `StartForward` serves `marker.txt` through the local
   port; after `StopForward` the port no longer answers.
5. A 50 MiB upload and download keep the SHA-256 (also checked inside the
   container), and a repeated download never replaces the local file.
6. After removing the project (its master exits) and `rekey`, the probe fails
   with `ChangedHostKey` and `TrustHostKey` is refused.

Masters, the daemon, the containers, their volumes and temporary keys are
removed on success and failure.

`cargo xtask gui ssh-hostkey` (after the test-support build) starts the same
fixture, opens an SSH project with an empty known-hosts file, clicks "Trust and
connect" in the host-key dialog (`ssh-hostkey-trust`), and captures the listed
project to `target/validation/native/ssh-hostkey.png`. It skips the same way
without Docker. Password sign-in in a terminal and `~/.ssh/config` aliases with
`ProxyJump` remain manual checks.
