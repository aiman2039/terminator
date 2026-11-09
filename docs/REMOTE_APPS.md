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

## Set up a host

1. Check it: `ssh HOST sh -s < crates/remote-apps/dev/host-check.sh`.
2. Build the host tools for x86_64 hosts (static, any glibc):
   `sh crates/remote-apps/dev/build-host.sh`, then copy all of
   `target/host-x86_64/` (`terminator-remote-bridge`, `wprsd`,
   `xwayland-xdg-shell`) into one directory on the host, for example
   `~/bin/`. The bridge uses the wprsd beside it first, so both speak the
   same wprs protocol.
3. Xwayland:
   - From packages (`xwayland`) on `PATH`: the default apps command,
     `terminator-remote-bridge --ensure`, works if the bridge is on the
     non-login `PATH`; else give its full path.
   - No root (for example CentOS 7): install
     [nix-portable](https://github.com/DavHau/nix-portable) in `~/bin`, set
     `NP_LOCATION` to a local (non-NFS) disk, and set the host's
     Properties → **Remote apps command** to:

     ```sh
     env NP_LOCATION=/local/disk TERMINATOR_WPRS_WRAPPER='~/bin/nix-portable nix shell nixpkgs#xwayland -c' ~/bin/terminator-remote-bridge --ensure
     ```

     The wrapper starts wprsd (from beside the bridge) in its own
     nix-portable process with Xwayland on its `PATH`, so the apps keep
     running when the bridge (the viewer) stops. The first start downloads
     Xwayland. Without the tools beside the bridge, add `nixpkgs#wprs` to
     the wrapper (nixpkgs builds it with AVX2; it has no `ReplayDone`, so
     `Ready` comes after 3 s).
4. In Terminator: Hosts sidebar → the host's menu → **Show remote apps**.
   New terminals on the host get `DISPLAY`; run an X11 app there (for
   example `xclock`) and it opens as a tab.

### Real-host test

On this computer: build Terminator (`cargo build --workspace --bins
--locked`) and the host tools (step 2). On the host, then in Terminator:

1. `ldd ~/bin/terminator-remote-bridge` says "not a dynamic executable",
   and `~/bin/wprsd --help` runs.
2. Show remote apps: the status becomes connected; `/tmp/terminator-remote-$(id -u)/`
   holds `wprsd.json`, `session.json`, `bridge.pid`, `wprsd.log`.
3. New SSH terminal: `echo $DISPLAY` shows the display; `xclock &` opens a
   tab; `xterm &` opens another.
4. Type in xterm; click and scroll; open a menu (popup over the tab); resize
   the tab; copy and paste both ways.
5. Close Terminator and open it again: the tabs come back with the apps.
6. Drop the network (sleep the laptop or `kill` the ssh master): the tab
   says Reconnecting, then the apps come back.
7. Show remote apps from a second computer (or a second Terminator data
   dir): the first one says another Terminator shows them.
8. Close a window from the app and with the tab's close button (twice
   within 5 s closes an app that ignores it).

On a failure, send `/tmp/terminator-remote-$(id -u)/wprsd.log` and the
host's error text from the tab.

Keys: a focused remote tab takes every key (Cmd works as Ctrl); F8 or a
click elsewhere gives the keys back to Terminator. Copy and paste work for
text; text an app copies reaches this computer only while one of that host's
windows has focus.

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
| Two Terminator instances | wprsd serves one client. A new bridge writes `bridge.pid`, waits 3 s for the old bridge to see it and leave, then kills it. The old one ends with `Error{taken_over}` (no retry); its tabs say "Another Terminator shows these remote apps now". |
| Daemon restarted | `RemoteAppsState` for an unknown session fails with "Unknown remote-apps session" (`UNKNOWN_SESSION`, the same text in every daemon version); the GUI connects again and keeps the tabs. |
| Old bridge or old daemon | `PROTOCOL_VERSION` 2 (`Ready`, damage rectangles, new clipboard messages): a version 1 peer gets the version mismatch error that names the install to update. An unknown `ErrorKind` reads as `other`. |
| Display number in use | Find a free one with lock files; keep it stable for each host. |
| Multi-user host | Sockets in a 0700 directory; xauth cookie; never `xhost +`. |
| No cookie, or proot (nix-portable on old kernels) | Xwayland refuses `xwayland-xdg-shell` ("Authorization required"). The bridge always starts Xwayland with `-auth` through a wrapper first in `PATH`, and picks the display with `--display=N` so the cookie matches. Proven by `host-smoke.sh`. |
| `XDG_RUNTIME_DIR` removed at logout (systemd) | Never used: the private directory is `/tmp/terminator-remote-<uid>` (0700), or `$TERMINATOR_REMOTE_DIR`. |
| Xwayland missing | `Error{xwayland_missing}`: "Install the `xwayland` package". |
| Bad private directory, no free display, wprsd does not start | `Error{host_setup}`; no retry. |
| Home on NFS | Sockets never on NFS. |
| Mosh terminals | Display traffic still uses the OpenSSH master. |
| `sudo`/`su` lose `DISPLAY` | Documented; shell hook hint. |
| Dialogs and menus | Popups are overlays, never tabs. |
| GL apps slow | Software rendering; documented. |
| Clipboard leaks | Clipboard sync is opt-in for each host. |
| Mac key mapping | Each host has a key mapping setting. |
| Orphaned `wprsd` | `wprsd.json` records it as soon as it answers, so a bridge that dies before `session.json` does not start a second one. |
| Old local daemon | Feature hidden without `remote-apps-v1`. |
| App starts before wprsd has an output | xwayland-xdg-shell never maps its window (upstream). The host launcher connects the bridge once (it sends the output) before it exports `DISPLAY`; wprsd keeps the output after the bridge leaves. |
| Configure sends an unknown window-state bit | wprsd panics. The bridge sends only `TILED` and `ACTIVATED`. |

## Phases

0. Spike: vendor wprs, protocol crate, Linux dev image. **Done.**
1. Bridge: wprs client on the host that composites windows and speaks the
   protocol over stdio. Test in the dev image with `xclock`. **Done**
   (`terminator-remote-bridge`). A `Frame` carries the damaged rectangles of
   the window (zstd BGRA). `compose::Damage` keeps up to 8 separate
   rectangles and merges two only when their bounding box adds little area,
   so a caret and a clock far apart do not resend the whole window; a new
   size is one whole-window rectangle. Each rectangle is composited into a
   canvas of its own size; the bridge keeps no window copies. Frames are
   rendered only while the stdout writer is idle, so a slow link skips
   frames instead of queueing them. After `Hello` and the window replay the
   bridge sends `Ready`: wprsd marks the end of its replay with
   `Request::ReplayDone` (a local change, see `vendor/wprs/UPSTREAM.md`); a
   wprsd without it gets `Ready` after 3 s.
2. Daemon session (`remote-apps-v1`): start the bridge through the SSH master,
   own the stream, detach and reattach. Host side done:
   `terminator-remote-bridge --ensure` finds or starts this user's wprsd
   (private dir `/tmp/terminator-remote-<uid>` or `$TERMINATOR_REMOTE_DIR`,
   own Xauthority cookie, `-auth` shim for Xwayland, free display from
   `:100`, detached process group), records it in `wprsd.json`, and writes
   `session.json` for shells after `Ready` (wprsd then has an output). Tools come from beside the
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
   `RemoteWindow.regions` lists what changed after the poll's `since`
   (empty: read the whole window); the GUI patches only those areas of its
   texture. `cursor` and `clipboard` come only when they changed after
   `since`. The status is `connected` only after `Ready`, so the GUI closes
   tabs of missing windows only against a full window list.
   Exit reasons: exit 127 or "not found" → `wprsd_missing` ("bridge is not
   installed on the host"), "server disconnected" → `wprsd_stopped`, SSH
   exit 255 → the classified SSH error. A bridge `Error` message wins over
   the exit reason. Removing an SSH project is refused while its remote apps
   are connected. Window IDs are wprsd surface IDs, so a new bridge keeps
   them. Temporary failures (SSH, network, wprsd stopped) retry after 1, 2,
   4 … 60 s while the status is `connecting` with `error`, `retry_attempt`
   and `retry_in_seconds`; the backoff resets only on `Ready`. A missing
   bridge or Xwayland, host setup errors, a version mismatch, a takeover or
   no `ssh` end the session (`ErrorKind::permanent`). With no viewer (no `RemoteAppsState` or
   connect) for Settings → VNC detached timeout (default 30 min, 0 is
   never), the daemon stops the bridge; the apps stay in wprsd and the next
   connect shows them. **Phase 2 done.**
3. GUI: tab per toplevel, popups as overlays, input, cursor, resize, HiDPI.
   First pass done: `Tab::RemoteApp { project, window, title }` (layout
   v12, key `remote-app:<project>:<window>`). SSH host menu → Show remote
   apps / Disconnect remote apps; restored tabs reconnect at start. New
   toplevels open tabs, titles follow, windows that close close their tabs
   (not while reconnecting). Pixels 1:1 with physical pixels; the tab size
   in physical pixels is the window size (200 ms debounce); popups draw over
   their parent. Pointer, wheel (Wayland axis signs) and evdev keys by
   physical position (Cmd acts as Ctrl). Closing a tab asks the app to close;
   a second close within 5 s closes the tab, and that window gets no new tab
   while it exists. A focused tab that another tab covers releases its keys
   and tells the app it lost focus; the pointer leaving sends a leave. When
   one window shows in two panes, only the first sends input.
   Live check: `dev/gui-e2e.sh` (xclock fills its tab). A focused remote tab
   takes every key (F8 gives them back). The cursor comes from the app (CSS
   name → local cursor; image → drawn at the pointer). One app pixel is one
   point, so X11 apps are readable on HiDPI screens. **Done.**
4. Launch UX: new SSH and Mosh terminals export `DISPLAY`, `XAUTHORITY` and
   `WAYLAND_DISPLAY` from `session.json` when its X socket is live (else one
   line says remote apps are not running); host Properties → Remote apps
   command. **Done.**
5. Clipboard (text, selection; local text goes with the paste key in one
   request; remote text reaches the local clipboard only while one of that
   host's windows has focus), static x86_64 bridge build
   (`dev/build-host.sh`), `TERMINATOR_WPRS_WRAPPER` for nix-portable hosts,
   `docs/VALIDATION.md`. **Done; the real-host test is next.**

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
