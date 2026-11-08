# Remote apps

Status: in development (branch `worktree-remote-apps`). Not shipped.

Remote GUI apps open as Terminator tabs, one tab for each app window, with no
remote desktop. X11 apps use a normal `DISPLAY`, as with vncserver.

## Architecture

```
Linux host (Terminator install)                 This computer
┌───────────────────────────────────────┐       ┌──────────────────────────────┐
│ X11 app ── DISPLAY=:N ── Xwayland      │       │ terminator-daemon            │
│               │                        │       │  owns the session, detach    │
│      xwayland-xdg-shell (wprs)         │       │        │ IPC                 │
│               │ Wayland                │  SSH  │ terminator GUI               │
│ Wayland app ── wprsd (wprs) ── bridge ─┼───────┼─ one tab per toplevel,       │
│                         rkyv   (ours)  │ stdio │  popups as overlays          │
└───────────────────────────────────────┘       └──────────────────────────────┘
```

- `wprsd` is a rootless Wayland compositor (vendored `vendor/wprs`). It keeps
  all app state, so apps survive a disconnect.
- `xwayland-xdg-shell` gives X11 apps a `DISPLAY` through Xwayland.
- The **bridge** (`crates/remote-apps`, Linux) is a wprs client. It composites
  each toplevel and its subsurfaces into one image and speaks the Terminator
  display protocol (`crates/remote-apps/src/protocol`).
- The wprs protocol (rkyv) never leaves the host. It has no stability promise,
  and smithay cannot link on macOS without `libxkbcommon`. The Terminator
  protocol is versioned, JSON headers with binary payloads.
- Transport: the stdio of an exec channel on the existing OpenSSH master. This
  does not need `AllowStreamLocalForwarding`.

## Host requirements

The host runs a Terminator install that includes `wprsd`,
`xwayland-xdg-shell` and the bridge. Terminator never copies or installs
anything on the host. If the host install is missing or too old, Terminator
tells the user to update Terminator on the host. Xwayland comes from the
host's packages (`xwayland`).

## Versions

- Both sides send `Hello` with `PROTOCOL_VERSION`, the Terminator version and
  capabilities. On a mismatch, the message names the side to update.
- New features are new capability strings, not version bumps.
- The local GUI gates the feature on the daemon capability `remote-apps-v1`.
- A running `wprsd` is never replaced. A new host install takes effect when
  the last remote window closes.

## Failures

| Failure | Behavior |
|---|---|
| `wprsd` stops | Bridge sends `Error{wprsd_stopped}` with the last log lines. The GUI shows a bar on the host. A shell hook prints a clear line when `DISPLAY` is dead. The display restarts with backoff and keeps its number. |
| Network drop or sleep | Apps keep running in `wprsd`; the tab shows "Reconnecting…" and reattaches. |
| Two Terminator instances | The newest takes the windows; the other shows "moved to another client". |
| Display number in use | Find a free one with lock files; keep it stable for each host. |
| Multi-user host | Sockets in a 0700 directory; xauth cookie; never `xhost +`. |
| No cookie, or proot (nix-portable on old kernels) | Xwayland refuses `xwayland-xdg-shell` ("Authorization required"). The bridge always starts Xwayland with `-auth` through a wrapper first in `PATH`, and picks the display with `--display=N` so the cookie matches. Proven by `host-smoke.sh`. |
| No `XDG_RUNTIME_DIR` | Private directory under `/tmp`. |
| Xwayland missing | `Error{xwayland_missing}`: "Install the `xwayland` package". |
| Unsupported CPU | `Error{unsupported}`. |
| Home on NFS | Sockets never on NFS. |
| Mosh terminals | Display traffic still uses the OpenSSH master. |
| `sudo`/`su` lose `DISPLAY` | Documented; shell hook hint. |
| Dialogs and menus | Popups are overlays, never tabs. |
| GL apps slow | Software rendering; documented. |
| Clipboard leaks | Clipboard sync is opt-in for each host. |
| Mac key mapping | Each host has a key mapping setting. |
| Orphaned `wprsd` | Stops after the idle timeout. |
| Old local daemon | Feature hidden without `remote-apps-v1`. |
| App starts before wprsd has an output | xwayland-xdg-shell never maps its window (upstream). The host launcher connects the bridge once (it sends the output) before it exports `DISPLAY`; wprsd keeps the output after the bridge leaves. |
| Configure sends an unknown window-state bit | wprsd panics. The bridge sends only `TILED` and `ACTIVATED`. |

## Phases

0. Spike: vendor wprs, protocol crate, Linux dev image. **Done.**
1. Bridge: wprs client on the host that composites windows and speaks the
   protocol over stdio. Test in the dev image with `xclock`. **Done**
   (`terminator-remote-bridge`; frames are full-window zstd BGRA for now,
   cursor and clipboard come later).
2. Daemon session (`remote-apps-v1`): start the bridge through the SSH master,
   own the stream, detach and reattach. Host side done:
   `terminator-remote-bridge --ensure` finds or starts this user's wprsd
   (private dir `$XDG_RUNTIME_DIR/terminator-remote` or
   `/tmp/terminator-remote-<uid>`, own Xauthority cookie, `-auth` shim for
   Xwayland, free display from `:100`, detached process group) and writes
   `session.json` after wprsd has an output. Tools come from beside the
   bridge, else from `PATH`. Test: `dev/bridge-ensure-e2e.sh`.
   Daemon side done, tested end to end by `dev/daemon-e2e.sh` (local
   isolated daemon → SSH → Docker host from `dev/sshd.Dockerfile` → xclock:
   connect, shared-memory frame, resize to 400x300, close, disconnect):
   capability
   `remote-apps-v1`; requests `RemoteAppsConnect{project}` (one live session
   per project, reused), `RemoteAppsState{session, since, wait_ms}` (long
   poll), `RemoteAppsInput` and `RemoteAppsDisconnect`. The daemon runs
   `exec sh -c 'exec <apps_command>'` over the project's SSH master (no
   login shell: profile output would corrupt the stream), with
   `RemoteProject::apps_command` defaulting to
   `terminator-remote-bridge --ensure`. Each window's pixels are
   premultiplied RGBA in a `terminator_sys` shared-frame file
   (`remote-app-*.fb` in the runtime directory); a new size gets a new file.
   Exit reasons: exit 127 or "not found" → `wprsd_missing` ("bridge is not
   installed on the host"), "server disconnected" → `wprsd_stopped`, SSH
   exit 255 → the classified SSH error. A bridge `Error` message wins over
   the exit reason. Removing an SSH project is refused while its remote apps
   are connected. Still to do: reconnect with backoff and the detached
   timeout.
3. GUI: tab per toplevel, popups as overlays, input, cursor, resize, HiDPI.
4. Launch UX: `DISPLAY` and `WAYLAND_DISPLAY` in host terminals, shell hook.
5. Clipboard, packaging of the Linux host binaries, `docs/VALIDATION.md`.

## Development

```sh
docker build -t terminator-remote-apps-dev crates/remote-apps/dev
docker volume create terminator-remote-apps-build
docker run --rm -v "$PWD":/src:ro -v terminator-remote-apps-build:/build \
  -e RUSTUP_TOOLCHAIN=1.97.1 -w /src/vendor/wprs \
  terminator-remote-apps-dev cargo build --release --bins
```

Bridge end-to-end test (wprsd + Xwayland + xclock; the probe acts as the
local daemon: reattach, first frame, resize to 400x300, close). Inside the dev
image, from `/src`:

```sh
cargo build -p terminator-remote-apps --features bridge --bins --examples
sh crates/remote-apps/dev/bridge-e2e.sh
```

Set `TERMINATOR_BRIDGE_TRACE=1` to print each wprsd request on the bridge's
stderr, and `LOG_LEVEL=debug` for wprsd logs.
