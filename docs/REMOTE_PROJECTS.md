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

OpenSSH config aliases, explicit users/ports, identity files, and an SSH agent
are supported. Noninteractive key/agent authentication is required. Host keys
must already be trusted in the configured known-hosts file. Agent/X11 forwarding,
local commands, and configured port forwards are disabled. Local Terminator
session tokens and routing variables are removed from SSH child environments.
They are not forwarded or installed on the host. Remote agent lifecycle hooks
are not provided by this feature.

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

## Local fixture

```sh
bash scripts/ssh-project-fixture.sh start
bash scripts/ssh-project-fixture.sh status
bash scripts/ssh-project-fixture.sh stop
```

The start command prints the connection settings. Defaults:

- Host: `127.0.0.1`, SSH port: `15932`, user: `developer`.
- Root: `/workspace/project`, stored in a Docker named volume.
- Disposable client keys: `$SSH_PROJECT_FIXTURE_KEY_DIR`, or a temporary folder.
- Known-hosts data is copied from the controlled container, not blindly trusted
  from a network key scan.

The container has Git, Neovim, test files, a modified tracked file and an
untracked file. It uses an unprivileged SSH account and publishes only localhost.
No host repository is mounted. The fixture is limited to one CPU, 512 MiB, and
128 processes. Stop preserves fixture data and host keys; no production server,
user SSH configuration, or installed app is modified.
