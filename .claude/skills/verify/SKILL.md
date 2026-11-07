---
name: verify
description: Verify Terminator end to end — SSH/Mosh remote terminals against a local sshd, or GUI changes against an isolated daemon with seeded projects/layouts (snapshot RPC + window screenshot).
---

# Verify Terminator

## SSH / Mosh terminals

1. `cargo build --workspace --locked`.
2. Test host: user-level `/usr/sbin/sshd -D -e -f <config>` on 127.0.0.1:2299 with temp host/client keys
   (`UsePAM no`, `StrictModes no`, `AuthorizedKeysFile <temp>`). Write `[127.0.0.1]:2299 <host.pub>` to a temp known_hosts.
3. Isolated daemon: `env TERMINATOR_DATA_DIR=<short /tmp dir> TERMINATOR_NO_NOTIFICATIONS=1 target/debug/terminator-daemon &`.
   Poll `terminator-hook ctl --data-dir <dir> list` until it answers.
4. `ctl add-remote-project --host 127.0.0.1 --port 2299 --root $HOME --name t --identity <key> --known-hosts <kh>`.
   Mosh: no ctl flag; send `{"UpdateRemoteProject":{"project":<json with "mosh":true>}}` via `terminator-hook rpc` (with `TERMINATOR_DATA_DIR`).
5. `ctl create --background t` → session `id`; `ctl send --enter <id> 'echo X'`; `ctl read --screen <id>`; `pgrep -fl mosh-client`.

SSH gotchas:
- Inside a Terminator terminal, unset every inherited `TERMINATOR_*` var first, or the daemon refuses to start
  ("Inherited generation/catalog routing does not match").
- The agent's Bash `ssh` may be refused by permission rules; give the user a script to run with `! sh <script>`.
- Keep the data dir under /tmp (Unix socket path length).

## GUI changes (isolated daemon + seeded layouts)

1. Build: `cargo build --workspace --bins --locked` (+ `cargo build -p terminator --example layout` for a sample DockState JSON: `target/debug/examples/layout <session-id>`).
2. Isolated env (fresh dir under /tmp, keep socket path short): strip inherited `TERMINATOR_*` vars, then set `TERMINATOR_DATA_DIR=$ROOT`, `TERMINATOR_CONFIG_DIR=$ROOT/appearance`, `XDG_{CONFIG,DATA,STATE}_HOME=$ROOT/...`, `TERMINATOR_NO_NOTIFICATIONS=1`, `TERMINATOR_RENDERER=glow`. Write `$ROOT/ui-preferences.json` = `{"version":2,"ide_mode":false}`.
3. Start `target/debug/terminator-daemon` with that env. RPC wire: unix socket `$ROOT/run/daemon.sock`, frame = 4-byte big-endian length + JSON `{"version":1,"auth":<$ROOT/run/auth>,"request":...}`. Useful requests: `"Snapshot"`, `{"Settings":...}` (set `shell` to `/bin/sh`), `{"AddProject":{"path":..}}`, `{"Create":{"project":id,"cwd":null,"file":null,"line":null,"editor":false}}`, `{"SaveLayout":{"project":id,"layout":..}}` (stored raw, unvalidated).
4. Launch `target/debug/terminator` with the same env. GUI state: same framing on `$ROOT/run/gui.sock`, request `"Snapshot"` → `result.workspaces` (per-project loaded layouts), `selected_project`, `active_session`. Load failures show as a status-bar banner "<project>: … Layout will not be overwritten." and the daemon copy stays untouched.
5. Screenshot only the app window: find the window titled `Terminator` for the GUI pid via `CGWindowListCopyWindowInfo` (a small Swift script), then `screencapture -x -o -l <id>`. Other layer-0 windows of the pid are blank helpers.

GUI gotchas:
- Daemon `SelectProject` does not change the GUI's selection.
- Synthetic CGEvent clicks only hover sidebar rows; making the window frontmost lets the user's own keystrokes land in it. Prefer RPC/snapshot over driving the desktop.
- Shutdown RPC refuses while sessions run; SIGTERM the GUI, then the daemon pid you started.
