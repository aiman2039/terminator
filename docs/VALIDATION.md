# Validation evidence — 2026-09-08

## Notification sound and GUI player (2026-09-16)

- `cargo test --package terminator-core --package terminator-daemon notification_sound` / `os_banner_sound`: passed.
- `cargo test --package terminator --offline`: 217 passed (includes layout v5 player tab, audio open without Neovim, stream URL rejection, wav decoder).
- `cargo clippy --package terminator --package terminator-core --package terminator-daemon --all-targets -- -D warnings`: passed.
- Live OS banner sound and Icecast playback need a desktop audio device; not established by the unit suite.

## Dependency security workflows (2026-09-16)

Added Dependabot (Cargo only), Renovate (`github-actions` only), `deny.toml`,
`.cargo/audit.toml`, `.github/workflows/security.yaml`, and
`.github/workflows/scorecard.yaml`. README badges link CI, Security, MIT,
Dependabot, and OpenSSF Scorecard.

- `cargo audit` on this tree: 0 vulnerabilities, 5 unmaintained warnings (not
  treated as CI failures).
- License allowlist in `deny.toml` was taken from `cargo metadata --locked`
  SPDX strings. `cargo-husky` has no crates.io license field and is clarified
  as MIT (dev-only hook installer).
- OWASP Dependency-Check is `continue-on-error: true` because CPE matching is
  noisy on Cargo; flip to blocking after a suppression baseline exists.
  The job skips when `NVD_API_KEY` is unset (OWASP 13 fails NVD updates on an
  empty key). When set, NVD data is cached under `owasp-data/`.
- Snyk and Socket skip when `SNYK_TOKEN` / `SOCKET_SECURITY_API_KEY` are unset.
- OSV-Scanner uses `osv-scanner.toml` with the same unmaintained RUSTSEC ignores
  as `deny.toml` / `.cargo/audit.toml`.
- Hosted Security/Scorecard runs, secret-backed Snyk/Socket scans, Renovate App
  onboarding, and Dependabot alert enablement were not executed in this change.
  Scorecard badge 404s until the first `publish_results` run succeeds.

## Explicit session restart warning (2026-09-16)

The restart confirmation uses bold danger-colored text for session termination
and unsaved editor loss, with a “Stop all sessions and restart” button.

- Workspace formatting and test-support build passed.
- `cargo xtask gui installation --output /tmp/terminator-restart-warning-preview`
  passed using disposable sessions, including cancellation and restart/relaunch.
- Inspected native capture: `/tmp/terminator-restart-warning-preview/installation/restart-confirm.png`.
  No user sessions were restarted.

## Keyboard protocol and updater review fixes (2026-09-16)

Modified navigation keys and F1–F12 now use the kitty protocol's CSI letter/tilde
encodings; F13–F35 retain CSI u. macOS updater helpers are compiled only on macOS
or in tests, avoiding unused-code warnings in the Linux application target.

- `cargo test -p egui_term --locked --offline keyboard::tests`: 7 passed,
  including navigation keys, F1–F12, and F13/F35 boundaries.
- `cargo test -p terminator --locked --offline updater::`: 6 passed.
- Workspace Clippy with all targets/features and `-D warnings`, workspace
  formatting, and explicit formatting of the vendored keyboard module passed.
- Checks ran on macOS; Linux compilation and native Neovim input were not run.

## Unbound Command/Ctrl terminal chords (2026-09-16)

Unbound Command/Ctrl keys are forwarded as kitty CSI u (Cmd+E → `CSI 101;9 u`).
Ctrl+E stays ENQ. Copy chords are not forwarded. Settings `command+T` still
consumes and removes the key event so it is not also typed.

- Unit: `egui_term` keyboard encoding; app shortcut consumption.
- Native desktop typing of Cmd+E into a live Neovim mapping was not recaptured
  in this change.

## Stable latest-activity sidebar (2026-09-15)

Selecting a visible project no longer stamps `project_activity`. Latest activity
ranks session, agent, and notice times, plus restore/add/worktree. Click, palette,
go-session, and hide-next keep order.

- Unit tests: visible select/go-session keep rank; restore/add/worktree move to
  top; hide-next does not bump the remaining project.
- `cargo test --workspace --all-features --locked` passed. Clippy `-D warnings`
  and rustfmt passed.
- `cargo xtask gui project-sidebar` passed. Latest-activity capture clicks the
  non-top project and still waits for activity order (first project remains first
  from its later editor session), including after GUI restart.

## HTML Blitz preview (2026-09-15)

- Decision: no Chromium/CEF/webview. Local `.html`/`.htm`/`.xhtml` open as
  GUI-only `Tab::Html` rastered by Blitz CPU (`anyrender_vello_cpu`). Header
  always has Open in browser (`file://` / `Job::Browser`), plus Reload and
  Open as text. Fail closed with an error label; never a blank tab.
- DummyNet: no JS, no HTTP. Layout v4. Transitive `stylo` is MPL-2.0.
- Unit: extension gate, 2 MiB oversize, inline HTML raster, open creates no
  editor session. Native GUI screenshot was not recaptured in this change.

## Review regressions (2026-09-15)

- `cargo test --workspace --all-features --locked`: 233 passed with local
  socket/process access and the parent terminal environment intact.
  Workspace Clippy with `-D warnings`, rustfmt, and `git diff --check` passed.
- Native diff rows reserve missing split cells and clip each side. Document-wide
  cached widths retain horizontal scroll range when long lines leave the visible
  rows; refreshed documents invalidate the cache. Unified/Split keep independent
  scroll positions. Split rows now use the same font height as virtualization.
- Headless egui regressions cover insertion/deletion-only rows, long lines on both
  sides, the rightmost text, vertical plus horizontal scrolling, mode switching,
  width-cache invalidation, and split row pitch. These inspect layout/paint output;
  no new native desktop screenshot was captured.
- Shutdown socket tests run in isolated child test processes with
  `TERMINATOR_SESSION_ID` removed, without changing the production refusal to
  shut down from inside a managed session or mutating parallel tests' environment.
- `cargo xtask idle-close` passed for zsh, bash, unsupported sh, and the zsh
  prompt-framework case. Fish was unavailable. The new command-submission case
  uses an ordered attachment resize to acknowledge PTY input forwarding, then
  immediately requests idle close without a command-settling sleep and verifies
  that the shell remains alive. It does not prove ordering across unrelated
  sockets before the daemon receives their input.


## Native diff gutter chrome (2026-09-15)

Unified/split paint no longer stuffs `{:>4} {:>4} ±` into the same galley as
code or inherits the app's 8px `item_spacing`. Rows use the monospace line
height, stay left-aligned, and number/`+/-` columns are pixel-measured.

- `workspace_ui::tests`: short vs long equal lines share code x; delete/insert
  signs sit right of their numbers; split paints old on the left and new on the
  right; consecutive row pitch is font height (12–22px), not height+8.
- `cargo test --locked --features test-support --bin terminator workspace_ui::tests`
  and `diff::tests`: 11 passed. `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
  and `cargo fmt --all --check` passed.
- Live native GUI screenshot of a real dirty file was not re-captured in this
  change; rebuild the GUI to see the chrome.

## Native Git diff viewer (2026-09-15)

Git reviews default to a GUI `Tab::Diff` built with similar (line and word hunks)
and syntect (syntax), computed on the existing jobs worker. Settings → Terminal
& Editor → Diff viewer can select Neovim CodeDiff when `nvim-review-v1` is
present. Double-click on a git-dirty Explorer or Git-status file opens the
working-tree side when that side is dirty, otherwise staged; deleted files still
open a diff immediately. Click still opens the editor after a short delay on
dirty rows so the first click of a double-click does not also open the file.

- Unit tests cover snapshot sides, word-level inserts, untracked/binary/outside
  paths, Native vs Neovim routing, and `Change::default_staged`.
- `cargo xtask gui reviews` sets `review_mode=neovim`. Native default is covered
  by `legacy-diff` (no `CreateReview`) plus routing unit tests.
- Live native GUI paint, delay-click timing, and a real Neovim-mode review were
  not re-run in this change; record that follow-up in a later entry.

Review fixes use egui's input clock and configured double-click interval, with
pending opens flushed after the frame's click handlers. Delayed editor/image
opens retain the original project, cwd and tab target. Native diff paths reject
symlinks before resolving directory aliases, preserving the selected Git entry.

- Regressions cover a simulated 280 ms double-click without an editor launch,
  a configured 500 ms interval, project navigation before editor/image creation,
  internal/external/dangling symlinks on both diff sides, and deletion through a
  repository directory alias.
- `env -u TERMINATOR_SESSION_ID cargo test --workspace --all-features --locked --quiet`:
  all 208 tests passed. The inherited session marker otherwise trips two hook
  fixtures' outside-Terminator guard; the successful run allowed isolated Unix
  fixture sockets and filesystem watches outside the sandbox.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
  `cargo fmt --all --check`, and Git diff whitespace checks passed.
- Pointer regression coverage uses headless egui input; a live native GUI smoke
  test was not run for these fixes.

## Minimized GUI control and cleanup timeout (2026-09-14)

The running daemon remained healthy with both live shells intact while a read-only
GUI snapshot timed out after 5.05 seconds. A three-second process sample showed
the GUI in eframe's invisible/minimized-window path. Source inspection found IPC
response processing and exit checkpoints only in `App::ui`, which eframe skips
there. The earlier native cleanup test covered a visible window and missed this.

Extending the native fixture to minimize its disposable window reproduced the
unresponsive GUI status endpoint before the code change. Moving worker responses,
native exit handling, checkpoints and heartbeats to `App::logic` fixes the
dependency on rendering. Heartbeats remain suspended during exit. GUI requests
also carry a deadline and cannot perform late actions after expiring in the queue.

- All 155 workspace tests passed, including expired-request rejection without a
  late project/file action. Log: `/tmp/terminator-minimized-workspace-tests-final.log`.
- Workspace binaries/examples test-support build, strict workspace Clippy,
  formatting and whitespace checks passed.
- Native installation recovery passed with both visible and explicitly minimized
  GUIs. Each cleanup case used a shell and unsaved Neovim editor, verified normal
  GUI exit, retained on-disk file contents and ended records without relaunch.
  [Minimized cleanup result](screenshots/helper-recovery/cleanup-minimized.json).
  Captures/logs: `/tmp/terminator-minimized-close-final/`.
- Native GUI-update continuity also passed after moving checkpoint processing,
  including preserved sessions and unsaved editor state across GUI replacement.
  Artifacts: `/tmp/terminator-minimized-update-final/`.

No user sessions were stopped and the installed GUI was not replaced. Older
installed GUIs need their window restored and a complete normal quit before
retrying cleanup; the permanent minimized-window fix requires the rebuilt GUI.
The reproduction and native verification were on macOS; Linux native behavior
was not exercised in this follow-up.

## Version-mismatch restart (2026-09-15)

Detached `ctl shutdown --stop-all --relaunch` and in-app **Restart session service**
quit the GUI, stop live sessions without force-kill, drop `ui.lock`, and respawn
this installation with `TERMINATOR_DATA_DIR` / `TERMINATOR_RUNTIME_DIR`. Sparkle
Install and Relaunch is unchanged. Idle auto-repair is unchanged.

- Unit: session-leader `setsid`; missing exe / failed GUI close do not Stop;
  successful relaunch writes isolated env after lock release; stop timeout
  relaunches without `ShutdownIfIdle`; in-session still refused; GUI restart
  hidden for newer/idle daemons; confirm cancel does not enqueue a job; copied
  stop-all includes `--relaunch`.
- `cargo xtask integration`: two-session `--relaunch --exe stub`, history kept,
  ended records not revived, stubborn HUP keeps the daemon and still launches
  the stub. Large-snapshot fixture uses distinct invocation IDs so wait-supersede
  does not dismiss the 130-notice payload.
- `cargo xtask gui installation` passed. Evidence:
  `target/validation/native/installation/restart-cancel.png`,
  `restart-confirm.png`, `restart-relaunch.json`
  (`restarted`, `unsaved_buffer_not_written`, `history_not_restarted`,
  `helper_available`).

## Explicit stop-all cleanup (2026-09-14)

Added `terminator-hook ctl shutdown --stop-all` and a separate copyable command
under Installation → Stop all sessions and shut down. The command checkpoints and
closes the GUI, stops daemon-owned sessions, waits for their end, flushes history
through shutdown, and waits for socket removal and lock release. Saved records
remain. Unsaved buffers are explicitly discarded; refusal/timeouts are errors,
not force-kill fallbacks. The hook now reuses the workspace's existing fs2 package.

The first real-PTY run found that background worker Arcs could leave the private
helper directory behind on normal process exit. Teardown now removes that exact
directory explicitly while holding the daemon lock. No saved history is removed.

- All 154 workspace tests passed, including new failed-GUI-save and
  changed-generation/concurrent-session refusal tests. Shell quoting tests cover
  both displayed commands. Socket/watcher tests needed access outside the sandbox.
- Workspace binaries/examples build with test support, strict workspace Clippy,
  rustfmt and diff checks passed.
- `xtask integration` passed: default/self shutdown refusal, two-session cleanup,
  helper/socket teardown before return, retained history and ended records after
  restart, and an HUP-ignoring session that times out without retiring its daemon.
  The fixture then explicitly exits its stubborn shell. Existing integration
  regressions also passed. Log: `/tmp/terminator-stop-all-integration-final.log`.
- The extended native installation fixture passed. The cleanup command itself
  closed an isolated GUI normally, stopped a shell and an unsaved Neovim editor,
  preserved the on-disk file without silently saving its buffer, and retained
  historical records without relaunch. The existing manual/idle/reconnection
  cases passed, and the [stop-all explanation](screenshots/helper-recovery/stop-all-command.png)
  was visually inspected. Captures/logs: `/tmp/terminator-stop-all-ui/`.

The two observed live user sessions were not stopped. The installed app/helper
were not replaced; the new command is available from the locally built helper.
Linux native rendering and detached processes that escape terminal job control
were not exercised. Stop-all uses normal daemon termination and reports sessions
that refuse to exit; it is not an unconditional OS process-tree kill.

## Manual recovery instructions and reconnect banners (2026-09-14)

Settings → Updates → Installation now explains manual recovery for legacy
services and provides **Copy shutdown command** after live sessions finish.
The command uses the current GUI's absolute helper/data/runtime paths with shell
quoting. Instructions require fully quitting the GUI before running it in another
terminal, then reopening after shutdown. No legacy shutdown is executed by the UI.

Successful snapshots clear old connection errors while preserving unrelated
operation failures. Transport failures invalidate the worker's snapshot hint so
an unchanged daemon can restore connected status. GUI Ping/Snapshot requests now
observe the GUI directly without requiring or refreshing the daemon first;
presentation actions retain their daemon state refresh.

- All 152 workspace tests passed (101 app, 26 core, 15 daemon, 3 hook,
  4 integrations, 3 packaging), including shell metacharacters/custom endpoints
  and clearing connection errors without hiding failed operations. The initial
  full-workspace sandbox run blocked socket/watcher fixtures; rerunning with
  access to their isolated sockets and watchers passed.
- Workspace binaries/examples build with `terminator/test-support`, strict
  all-target/all-feature Clippy, formatting and whitespace checks passed.
- The extended native installation fixture passed: [manual instructions](screenshots/helper-recovery/manual-recovery.png),
  live-session protection, idle repair, and [automatic banner clearing](screenshots/helper-recovery/reconnected.png)
  after a temporary socket outage with identical daemon generation and revision.
  An initial outage test exposed the GUI status endpoint's daemon dependency;
  the final fixture verifies recovery without causing a state refresh itself.
  Captures are under `/tmp/terminator-manual-recovery-ui-verified/installation/`.
  Copyable shell text was executed only against a harmless argument-reporting
  fixture helper; native clipboard transfer was not exercised.

After the user's manual shutdown, a process check showed the original GUI still
open and no daemon. Starting the installed daemon restored version 0.16.0 with an
available private helper and no degraded warning. The inventory retained all
110 ended and 10 interrupted sessions, with no live sessions or historical
relaunch. The installed GUI was not replaced; its stale banner can be dismissed
or cleared by quitting and reopening now that the daemon is reachable. The UI
changes above are local builds, not an installed or published application update.

## Installed 0.16.0 helper and history diagnosis (2026-09-14)

A read-only process check and authenticated health snapshot confirmed the GUI
was running from `/Applications/Terminator.app`, while its older daemon still
referenced an `AppTranslocation` installation. That daemon reported no version,
neither `shutdown-if-idle-v1` nor `stable-helper-v1`, and the old queue-saturation
warning. Its inventory contained no live sessions at inspection. The disabled
repair action therefore reflected unsupported safe restart, not missing files
in the current installed app. Logout/login is one recovery option. Inspection of
the older daemon source also confirmed an existing manual `Shutdown` request
that rejects live sessions and flushes history. After saving work and quitting
the GUI to prevent new session creation, users can invoke the installed helper
with `rpc '"Shutdown"'`, wait for daemon exit, then reopen the installed app.
This legacy check is not serialized against concurrent creation and is therefore
not a substitute for the capability-gated automatic repair path. The manual
shutdown command was inspected, not executed against the user daemon.

Copies of the installed 0.16.0 build 15 daemon/helper passed the existing real-PTY
integration suite through `TERMINATOR_TEST_BIN_DIR`. The history regression saved
all 6,291,456 payload bytes without truncation; the helper regression preserved
the private executable and original shell after removing its source installation.
Logs and binary hashes are in `/tmp/terminator-installed-check-cel6zhj9/`.
Fixtures used temporary state and required local socket/PTY access outside the
sandbox. The installed application and user daemon were not changed or restarted;
post-recovery production health and native rendering were not tested. Previously
dropped history remains lost. No additional application-code change was needed.

## Release cache collision and build scheduling (2026-09-14)

Run [34851813492](https://github.com/aiman2039/terminator/actions/runs/34851813492)
at `10f9f39516875a3cf6eb6918e8ec4070501471aa` failed in universal assembly because
the compiler cache restored `target/package/Terminator.app`. The four native
builds succeeded concurrently; the Intel macOS job took 11m56s, including 8m03s
for packaging/building, 54s for cache restore and 1m58s for cache cleanup/save.

Native release builds now share a composite action across separate macOS/Linux
matrices. Assembly waits only for macOS and consumes the Apple Silicon job's
archived `xtask` executable. Packages and extracted slices stay under
`RUNNER_TEMP`; the old generated package directory is removed after cache restore
without removing compiler artifacts. Packaging builds only the three shipped
executables and their dependencies. Both tooling and application builds emit
Cargo HTML timing artifacts, including available reports after build failure.

Validation on Apple Silicon, macOS 26.6.2, Rust 1.97.1:

- All 150 workspace tests passed (99 app, 26 core, 15 daemon, 3 hook,
  4 integrations, 3 packaging). The packaging regressions preserve existing
  outputs, reject a dangling app symlink, and reject a restored empty app
  directory before reading assembly inputs. Existing socket/watcher tests needed
  execution outside the sandbox; their initial permission failures were not
  application regressions.
- Strict all-target/all-feature workspace Clippy, rustfmt, and diff checks passed.
- Actionlint 1.7.12 checked the release workflow and the composite steps through
  a synthetic workflow wrapper. YAML parsing and all 16 shell blocks passed
  `bash -n`. Dependency checks verified four native targets, no Linux dependency
  for macOS assembly, both platforms required for draft upload, and no Cargo or
  Rust-cache action in assembly.
- Executing the actual legacy-cache cleanup body in a disposable checkout
  removed only `target/package`, retaining a marker in `target/release/deps`.
- The first local release package build took 34.24s with existing dependency
  artifacts; this was not a cold build. After the final source edit, identical
  packaging commands took 2.80s then 1.91s. The latter rebuilt no crates:
  development and optimized Cargo phases finished in 0.09s and 0.16s respectively.
  The logs confirm that no optimized `xtask` was built. Strict deep ad-hoc signature
  verification of the resulting native app passed.
- The actual tooling archive/restore and assembly shell bodies were exercised
  using the freshly built ARM app, the successful Intel slice from the failed
  run (artifact `10351139933`), and Sparkle 2.10.0 with its pinned SHA-256 verified.
  The already verified SDK archive was copied locally instead of downloading it
  again. A dummy public update key was used; no private keys were involved.
  Assembly succeeded with an unrelated cached app directory still present and
  a `cargo` shim that fails on invocation. `lipo` verified both architectures in
  all three assembled executables. The archived tool retained its executable
  permission and passed its ad-hoc signature check after extraction.

Logs, assembly fixtures and local packages are under
`/tmp/terminator-ci-fix-validation/`; Cargo reports are in `target/cargo-timings/`.
These timings are local verification, not measured GitHub speedups. No workflow
was dispatched, cache deleted remotely, tag moved, release published, signing
credential used, or installed application changed. Hosted execution and final
Developer ID signing/notarization remain to be checked on the next release run.

## Private daemon helpers and installation recovery (2026-09-14)

Each new daemon now pins its bundled helper in private persistent app data before
serving requests. Shell hooks and supported GUI attachments use that copy.
Settings → Updates → Installation handles older/missing helpers with live-session
navigation and a checkpointed, capability-gated idle repair.

Local validation on Apple Silicon, macOS 26.6.2, Rust 1.97.1:

- Workspace build with binaries/examples and `terminator/test-support`, formatting,
  strict all-target/all-feature Clippy, and `git diff --check` passed.
- All 147 workspace tests passed (99 app, 26 core, 15 daemon, 3 hook, 4 integrations).
  Regressions cover independent helper copies, missing source/cleanup, GUI helper
  capability fallback, legacy errors without health metadata, duplicate repair,
  live/unsupported/newer-daemon protection, generation changes, refused concurrent
  shutdown, and an unresponsive daemon whose lock remains held.
- `cargo xtask integration` passed. The new real-PTY regression overwrote and
  removed the running daemon's entire source installation, executed the pinned
  helper inside the original shell, and created another terminal with the same
  daemon generation. Removing executable permission from the private copy then
  invalidated conditional health; missing-helper failure preserved live sessions.
  All existing transport, hook, history, worktree and idle-shutdown race checks passed.
- `cargo xtask gui installation --output /tmp/terminator-helper-recovery/final`
  passed. Native screenshots show [repair blocked by a live session](screenshots/helper-recovery/live-sessions-preserved.png)
  and [successful idle repair](screenshots/helper-recovery/repaired.png).
  The fixture verified a new healthy daemon, ended-session history without relaunch,
  and new terminal creation. [Result record](screenshots/helper-recovery/installation.json).
  Fixture executables are frozen before captures so another Cargo build cannot
  replace test-support binaries during the run.
- `cargo xtask gui updates --output /tmp/terminator-helper-recovery` passed,
  retaining the daemon/session identities, workspace layouts and an unsaved
  editor across GUI replacement, with continuing input/output and new sessions.
- A local development app was packaged at
  `/tmp/terminator-helper-recovery/package/Terminator.app`; strict deep ad-hoc
  code-signature verification passed. Package creation does not install it.

PTY/native fixtures required execution outside the sandbox to bind their isolated
Unix sockets. Production sessions, agent configurations, privacy grants and the
installed app were untouched. Linux native rendering, Developer ID notarization,
Gatekeeper and signed Sparkle installation were not exercised. Daemons too old to
advertise safe idle shutdown receive logout/login guidance after saving work and
closing sessions; they are never killed or sent unsupported requests.

## Universal macOS packaging and session-preserving GUI updates (2026-09-14)

Implemented universal assembly, the dynamically loaded Sparkle 2.10.0 controller,
Updates settings/native menu action, acknowledged asynchronous exit checkpoints,
and capability-gated retirement of older idle daemons. Production publication is
gated on signed-update validation; no release, signing secret, or production
installation was changed in this work.

Local evidence on Apple Silicon, macOS 26.6.2, Rust 1.97.1:

- Workspace build with binaries/examples and `terminator/test-support`, formatting,
  strict all-target/all-feature workspace Clippy and `git diff --check` passed.
- All 134 workspace unit tests passed (91 app, 24 core, 12 daemon, 3 hook,
  4 integrations). New regressions cover failed persistence, pending creation and
  pickers, follow-up draining, duplicate exit requests, timeouts, stale success
  replies, modeled installation cancellation/retry, unknown layouts and
  legacy/newer daemon preservation. Socket/PTY tests
  require execution outside the filesystem/network sandbox.
- `cargo xtask integration` passed, including eight simultaneous creation versus
  idle-shutdown races, existing real-PTY reconnect/same-PID checks, hook delivery,
  legacy-client behavior, daemon recovery without relaunch, worktree controls,
  oversized snapshots, stalled attachment cleanup and terminal-editor operands.
- `cargo xtask gui updates` passed with the pinned Sparkle framework loaded into
  an isolated fixture app. The fixture's explicit controller-availability check
  passed. It exercised window close and native Quit, replaced all three installed
  executable copies, and retained daemon generation/PID, four session IDs/PIDs,
  project/top-level-tab identities, split trees/fractions/focus (excluding derived
  screen rectangles), hidden-project state and Markdown mode. Output and input
  continued; an unsaved Neovim buffer remained unsaved on disk and modified in
  memory; new session creation and an event delivered through the replaced hook
  worked. Foundation preferences were redirected to a temporary fixture home.
- [Continuity inventory](screenshots/updates/continuity.json),
  [before replacement](screenshots/updates/before-replacement.png),
  [after replacement](screenshots/updates/after-replacement.png), and
  [Updates settings](screenshots/updates/settings.png) are retained. IDs, PIDs and
  temporary paths in this evidence identify the completed isolated run.
- Downloaded Sparkle 2.10.0 and verified its archive SHA-256 against the release
  asset digest. Universal assembly passed with small native C executable fixtures
  compiled for Intel and Apple Silicon, plus the actual pinned Sparkle framework;
  all bundled Mach-O files passed both-architecture checks and framework symlinks
  remained intact. This validates the assembly path, not a universal Rust release.
- Release workflow YAML parsed; all 12 shell blocks passed `bash -n` and embedded
  Python blocks compiled. Hosted GitHub jobs, the pinned CI Rust 1.95 toolchain,
  Intel-native GUI execution and Linux execution were not run here.

The native fixture exposed and fixed two AppKit integration failures: invoking
termination inside winit's active callback re-entered its event handler, while
`NSTerminateLater` blocked GUI checkpoint progress. The final bridge intercepts
`terminate:` without replacing winit's delegate, then invokes its original
implementation in a later main-queue callback. Both native exit paths passed
with this implementation.

Remaining rollout gates: two genuinely Developer-ID-signed and notarized versions
through Sparkle on **both Intel and Apple Silicon**, including Install and
Relaunch, installation on quit without reopening, Later/Skip, offline checks,
invalid feed/archive signatures, interrupted downloads, authorization or
installation cancellation, and read-only/translocated installs. The local fixture
uses an unreachable loopback feed and dummy public key and does not install via
Sparkle. Signing-key setup remains operator-owned. Keep
`SIGNED_UPDATE_VALIDATED` unset until that matrix passes; see
[UPDATES.md](UPDATES.md). Session continuity assumes the daemon survives; crashes
and reboots remain separate recovery cases.


## Clipboard image paste (2026-09-14)

- Reviewed Orca's [terminal clipboard routing](https://github.com/stablyai/orca/blob/main/src/renderer/src/components/terminal-pane/terminal-clipboard-paste.ts)
  and [temporary PNG writer](https://github.com/stablyai/orca/blob/main/src/main/window/clipboard-image-temp-file.ts):
  nonempty text takes priority; image-only clipboard contents become a temporary
  PNG whose path is pasted into the terminal. Terminator implements this locally
  using arboard and its existing image encoder, with shell-quoted paths.
- Cmd+V on macOS, Ctrl+Shift+V on Linux, and the terminal's Paste context menu
  support image-only clipboard contents. The egui-winit patch preserves empty
  text paste intent; the GUI routes image reads/encoding/writes to its worker.
  Results carry the originating session ID and never use the later active tab.
- PNGs use exclusive random filenames with Unix mode 0600 in the OS temp
  directory. Files survive GUI exit for daemon-owned agents; eventual cleanup
  is left to the OS. Invalid RGBA sizes and images above 128 MiB are rejected.
  Paths refer to this machine; no SSH upload is performed.
- All 81 app tests passed, including three clipboard regressions covering pixel
  round trips, unique/private retained files, quoted paths, malformed input, and
  one-time event consumption with text preservation. Locked all-target app
  checking, workspace strict Clippy, formatting, diff checks, and
  `cargo build --workspace --locked` passed.
- Native macOS follow-up passed using the real NSPasteboard, a normal GUI,
  Orca-generated Cmd+V, and two daemon-owned raw PTY receivers that record input
  without executing it. Image-only paste delivered exactly one quoted PNG path;
  text-only and mixed text/image paste delivered the expected text exactly once;
  an empty clipboard delivered nothing. Switching to the second project routed
  the next image only to its terminal. Each 32x24 PNG matched every decoded
  source pixel and had mode 0600; generated files survived GUI exit.
- The rendered context-menu Paste action passed through the existing native
  fixture mouse-event driver, using the real image clipboard and PTY receiver.
  Orca's OS right-click delivery could not be verified; this menu result is a
  GUI fixture result, distinct from the successful native OS keyboard tests.
- Evidence: `target/validation/clipboard-e2e/results.json`, `source.png`,
  `menu.png`, `menu.log`, and `REPORT.md`. Clipboard formats were retained in
  memory and restored; an earlier attempt preserved externally changed clipboard
  contents. Cleanup stopped only the isolated fixture GUI/daemon/sessions.
- Orca's focus check rejected the always-on-top screenshot fixture; the normal
  layer-0 GUI allowed native keyboard testing. No application fix was needed.
  Linux X11/Wayland runtime behavior, SSH upload, live-agent image recognition,
  and switching focus while image encoding is still in flight remain unverified.

## Project terminal indentation (2026-09-14)

- Expanded project terminals indent one standard spacing step beyond the actual
  project row, accounting for its separate expand/collapse button.
- Added and passed `expanded_project_terminals_stay_indented_at_all_sidebar_sizes`:
  measured parent/child row rectangles at widths 180, 280, and 420 points,
  scales 1 and 2, both selected projects, and initial/settled layout frames.
  The existing project-list/history regression also passed.
- Built the workspace binaries and examples with `terminator/test-support`.
  Native `project-sidebar` fixtures passed at normal 1x and narrow 2x sizing;
  inspected both restored-project screenshots and confirmed the indentation.
  Removal, restoration, and GUI restarts preserved fixture shell/editor PIDs,
  unsaved editor content, saved file bytes, and project layouts.
- Captures: `/tmp/terminator-indent-native-1x/project-sidebar/` and
  `/tmp/terminator-indent-native-2x/project-sidebar/` (five PNGs each).
  Fixtures used isolated temporary state; the sandbox initially blocked daemon
  socket startup, and both native runs passed outside the sandbox.
- `cargo fmt --all --check` and `git diff --check` passed. Linux native rendering
  was not tested.

## Cargo-husky and master CI (2026-09-14)

- Added cargo-husky 1.5.0 with a tracked pre-commit hook and shared
  `scripts/check.sh` commands for fmt, compiler checks, build, and strict Clippy.
  The installed `.git/hooks/pre-commit` passed end to end on macOS with Rust
  1.97.1, checking all workspace targets and features with the lockfile.
- All 121 workspace tests passed with isolated temporary application state.
  The initial sandboxed run failed socket/notification tests; the unrestricted
  rerun passed without Rust source changes.
- `actionlint` 1.7.12, shell syntax checks, and `git diff --check` passed.
- New CI runs checks and tests on macOS and Linux for master pushes, pull
  requests targeting master, and manual dispatch, using Rust 1.95.0. Hosted
  execution and the pinned CI toolchain were not validated locally. No native
  GUI smoke tests, release, signing, or deployment was performed.

## Release dependency caching (2026-09-10)

- Replaced download-only caching with `Swatinem/rust-cache@v2`, including
  compiled dependencies and separating runner/target combinations. Toolchain,
  manifests, lockfiles, and build configuration contribute automatic cache keys.
- Default-branch pushes run the same four-platform packaging builds to populate
  caches accessible to release tags. Other branch pushes skip preparation;
  default-branch builds do not create tags, upload artifacts, or publish releases.
  This adds four native builds per default-branch push.
- `actionlint`, Ruby YAML parsing, all five multiline shell blocks checked with
  `bash -n`, and `git diff --check` passed. Executing branch preparation locally
  emitted only the current commit SHA and exited before any tag/API operations.
- No hosted workflow was triggered. Cache hits, archive sizes, and build-time
  improvements remain unmeasured; local Rust builds do not verify hosted caches.

## Native release workflow (2026-09-10)

- `actionlint` 1.7.12 passed for `.github/workflows/release.yaml`.
- YAML parsing, `bash -n` for every shell block, and `git diff --check` passed.
- Six isolated temporary-Git scenarios exercised the workflow's preparation
  script: matching annotated tag, version mismatch rejection, manual retry,
  existing tag at a different commit rejection, missing push tag rejection, and
  manual tag creation with an asserted API request (remote mutation stubbed).
- No tag, release, or workflow run was created on GitHub. The four native release
  builds, archive checks, signing checks, and uploads await their first hosted
  run. No GUI, PTY, or live-provider validation was performed for this CI change.

## Project sidebar removal and flat Markdown header (2026-09-10)

Projects now expose **Remove project from sidebar** on right-click. Removal is
saved in navigation preferences and preserves project/session records, layouts,
files, running PTYs and unsaved buffers. **Removed** restores an entry; reopening
the folder or explicitly navigating to a session also reveals it. Folder reopening
recognizes saved path aliases and reuses the original project identity. Removing
the last visible project leaves an empty selection, including after GUI restart.

Markdown headers now put the filename, flat Edit / Preview / Split tabs, refresh
icon and pane X on one row. Tab selection uses the existing flat fill/underline
style. The filename retains its rename/context actions; other controls have
separate hit targets. Long filenames yield space to the controls.

- All **121 workspace tests** passed with all features and the lockfile. Added
  tests cover hide/restore persistence, late folder-open results, path aliases,
  one-row geometry and independent mode/refresh/close clicks.
- `cargo xtask gui project-sidebar` passed with two projects, two shells and an
  unsaved Neovim editor. Active-project removal, removal of every entry, GUI
  restart and restoration preserved all three original PIDs, file bytes, dirty
  state and both original tabs. The final empty-state text was also captured.
- `cargo xtask gui markdown` passed at **1x** and **2x with `--narrow`**. The
  fixture checks the header's shared row and nonoverlapping controls, along with
  mode switching, refresh, unsaved preview, buffer identity and safe close.
- Formatting, all-target/all-feature Clippy with warnings denied, and locked
  workspace/test-support builds passed. Inspected captures:
  [Markdown header](screenshots/markdown-header.png),
  [narrow Retina header](screenshots/markdown-header-2x.png),
  [removed projects after restart](screenshots/projects-removed.png).

## Markdown prompt timeout and Preview by default (2026-09-10)

A read-only fast-mode query of the two live README editors found one in normal
mode and the other in `rm` with `blocking: true`: Neovim's pager prompt. Ordinary
`--remote-expr` evaluation waits behind that prompt, causing the reported two-second
timeout. The existing prompt was inspected without sending it input.

The GUI now talks directly to the existing Neovim socket with MessagePack. It
checks fast `nvim_get_mode` before `nvim_exec_lua`, applies a 400 ms response
deadline and byte/depth limits, and never queues buffer evaluation behind a known
blocking prompt. While updates are paused, it preserves the last live/unsaved
snapshot or renders a clearly labeled saved-file preview. Refresh retains that
snapshot. Polling resumes live content once the editor is ready. The preview does
not answer prompts, save files, or restart editor processes.

New Markdown tabs now default to **Preview**. Explicit Edit and Split choices
are persisted, including an explicit Edit choice across GUI restarts.

- All **116 workspace tests** passed, including new coverage for blocking-editor
  fallback, unsaved-cache preservation through Refresh, Preview defaults, explicit
  Edit persistence, RPC response identity, deadlines and response size limits.
- `gui markdown-busy` passed with a real Neovim pager prompt, rendered saved-file
  fallback, and resumed live preview on the same editor PID. The preserved
  previous GUI reproduced the exact red two-second timeout with this fixture.
- The updated `gui markdown` passed initial-click Preview, Edit restoration,
  live unsaved rendering, switching modes, paused unsaved Refresh, and the existing
  buffer-identity, input-focus and cancel/save-close checks. The real pager is
  answered only by the isolated fixture after checking the paused preview.
- Formatting, all-target/all-feature Clippy with warnings denied, and locked
  workspace/test-support builds passed. Inspected captures:
  [readable preview during a prompt](screenshots/markdown-prompt-preview.png),
  [unsaved preview after Refresh](screenshots/markdown-prompt-unsaved.png).

Native checks used isolated macOS sessions. No live user editor prompt, buffer,
daemon or configuration was changed. A fresh GUI build is sufficient; the running
daemon does not need replacement. Linux was not exercised.

## Native Markdown Edit / Preview / Split (2026-09-10)

Markdown file sessions now provide three presentation modes using the same
Neovim process. The GUI renders previews with `egui_commonmark` 0.25.0, reads live
unsaved text through the existing Neovim socket, and stores per-session mode
preferences without changing dock identities or daemon IPC. Edit remains the
default. Split has a resizable divider; Preview releases only the GUI attachment.
The regular Neovim configuration and existing close/save lifecycle are retained.

- All **111 workspace tests** passed (`--workspace --all-features --locked`),
  including seven new Markdown/image tests for relative references, unsupported
  URL schemes, file changes/errors/size bounds, stale updates, refresh recovery,
  preview keyboard focus, and background image reload/release. Preference tests
  also verify old settings default safely and independent modes round-trip.
- `cargo xtask gui markdown` passed at **1x** and at **2x with `--narrow`**.
  Real GUI typing updated the live unsaved preview without changing disk bytes.
  Clicking and typing in the preview did not edit the buffer, both while split
  and with the editor hidden. Mode switching and GUI restart retained the exact
  original editor/shell PIDs and created no duplicate session.
- The native fixture changed the original file buffer while selecting a different
  scratch buffer in Neovim. The preview updated from the correct file. The path
  includes spaces, an apostrophe and Unicode. It also verified dirty Preview
  close → Cancel preserves edits, and Save and close writes them before ending
  only the editor. Standard size uses the top-level X; narrow size uses the pane X.
- `cargo xtask gui workspace-tabs` passed after extracting the shared terminal
  renderer, including ordinary Rust-file opening, splits, restoration and close.
- Formatting, all-target/all-feature Clippy with warnings denied, and locked
  workspace and test-support builds passed. Captures were inspected:
  [live split](screenshots/markdown-split.png),
  [preview](screenshots/markdown-preview.png),
  [narrow Retina split](screenshots/markdown-split-2x.png).

Fixtures used isolated data/config directories and real Neovim on macOS. No live
user session, daemon, configuration or hook installation was replaced. Linux was
not exercised. Custom terminal editors use a saved-file preview when no Neovim
socket is available; this fallback has focused file-reader coverage. Preview text
is bounded to 1 MiB, and local images reuse the existing bounded decoder. Remote
images show alt text; browser HTML, Mermaid and typeset math are outside this
implementation. The feature does not require restarting a running older daemon.

## Split/file opening after a checkout move and brighter icons (2026-09-10)

The fresh `workspace-tabs` native fixture passed before changes. Read-only
inspection of the user's live daemon and saved state instead found the project,
session working directories, and daemon executable still addressed the removed
`RustroverProjects/my-ai/terminator` checkout. Both shell and editor creation
require an accessible working directory and the helper beside the running daemon.

The local repair is a compatibility symlink from that missing path to
`RustroverProjects/terminator`. It restores access for the running daemon and
existing shell hooks without replacing the daemon or rewriting session records.
A subsequent read-only snapshot confirmed all 17 original live sessions retained
their PIDs and lifecycle, all live working directories resolved, and the old helper
path was executable. Keep this alias while those sessions use the old location.
No live session was created or stopped for validation.

New builds identify the exact missing working-directory/helper path in creation
errors and explain recovery after a move. Navigation, menu, project, file, tab,
and close icons now use near-white `#F2F4F8`; Git/status text and badges keep their
meaningful colors. [Split and bright icons](screenshots/split-file-opening.png),
[file opened from its menu at 2x](screenshots/split-file-opening-2x.png).

- All **104 workspace tests** passed with all features and the lockfile. The new
  icon test inspects painted SVGs with no hover and muted text. Existing request,
  click, asynchronous ownership and editor lifecycle tests also passed.
- The new `gui split-file-opening` fixture passed at **1x and 2x** with real PTYs
  and Neovim. It checks missing-directory failures leave inventory/PIDs intact,
  then verifies four native split-menu actions with correct tree orientation and
  pane placement, a double-click creates one editor, normal file-menu opening
  creates its own top-level tab, and editor-split opening shares the shell tab.
- The new fixture failed against the previous daemon binary because its error
  omitted the missing path and recovery instruction. The initial sandboxed full
  suite timed out in the existing macOS watcher test; the full run with native
  filesystem notifications passed.
- Locked workspace/test-support builds, formatting, and all-target/all-feature
  Clippy with warnings denied passed. Native captures were visually inspected.

Native actions were exercised in isolated fixtures on macOS, not in the user's
live sessions. Linux and live-provider behavior were not exercised. The running
older daemon was preserved; the improved errors apply when a new daemon is
started normally after its live sessions are no longer needed.

## Diff compatibility with a running older daemon (2026-09-09)

The reported “failed to fill whole buffer” came from sending `CreateReview` to a
pre-feature daemon. Read-only inspection found daemon processes started before
the feature build; the live default daemon's snapshot also omitted the review
session field. Older servers deserialize the request before dispatch and close
unknown variants without replying, leaving the GUI with a raw EOF error.

The running daemon now advertises `nvim-review-v1` in its snapshot capabilities.
Missing capabilities deserialize as empty. New GUIs use the existing local diff
renderer with older daemons and an explanatory status message, without sending
`CreateReview`, starting an editor, or restarting anything. Daemon startup replaces
persisted capability data with the features that binary actually supports.

Formatting, Clippy, the locked build, and all 61 workspace tests passed. Added
regressions cover legacy/new snapshot decoding and both staged/working diff
requests without the capability. `review_smoke.py` passed against the updated
daemon, including capability advertisement and both Neovim review modes.
`legacy_diff_smoke.py` passed native Git-menu clicks through an isolated proxy
that strips capabilities and closes unsupported requests like the old daemon:
zero unsupported requests, rendered diff content, no error, and the original
shell PID remained alive. [Inspected screenshot](screenshots/legacy-daemon-diff.png).

The full `integration.py` PTY and conditional-snapshot compatibility checks also
passed. The local debug macOS package was rebuilt and its deep/strict code
signature verified.

This compatibility behavior was tested on macOS. No live daemon or user session
was restarted, and no hooks were changed.


## Bundled Neovim Git reviews (2026-09-09)

New Git diff actions launch pinned CodeDiff in a separate Neovim PTY/top-level
tab. The daemon builds and embeds the Lua runtime and native diff library; no
user plugin install or first-use download is needed. Review sessions use a
separate profile and two read-only snapshots. The ordinary editor setting is
unchanged. Old persisted native diff tabs retain their existing renderer.

Validation on macOS arm64 with Neovim 0.12.3:

- Formatting, Clippy (`--workspace --all-targets --all-features -D warnings`),
  locked workspace build, and all **59 workspace tests** passed. New regressions
  cover partial staging, rename/deletion, unborn HEAD, untracked files, binary
  rejection/size limits, linked-worktree indexes, explicit conflict rejection,
  and distinct review tabs whose asynchronous creation retains project ownership.
- `scripts/integration.py` passed real-PTY lifecycle/reconnect/hooks/recovery
  checks and conditional snapshot compatibility checks.
- `scripts/review_smoke.py --gui` passed real Neovim PTY checks: exact staged and
  working-tree sides, literal filenames containing spaces/quotes/Unicode/`|`,
  both buffers read-only after layout switching, no user init loaded, operation
  with an unavailable ordinary editor setting, q exit, snapshot cleanup, and
  unchanged Git index/status/file bytes and original shell PID.
- Native menu actions opened reviews in distinct top-level tabs at 1x and 2x.
  Clicking each review tab X closed it without a session prompt and preserved
  the original shell/layout. Captures were inspected:
  [1x](screenshots/nvim-review-1x.png), [2x](screenshots/nvim-review-2x.png).

The local debug macOS package was built with `scripts/package.py --debug`.
`codesign --verify --deep --strict` passed, and the package contains CodeDiff,
VSCode and utf8proc attribution. The packaged daemon also passed the standalone
review PTY fixture using its embedded runtime. No package was installed.

The first PTY probe exposed CodeDiff resetting the right buffer's editability;
post-setup and layout-event protection corrected it, and regression checks pass.
A sandboxed workspace run timed out in the existing native watcher test; the
complete suite passed with native notifications available. The initial native
fixture incorrectly used the six-pane helper for one pane; the helper now
supports both fixture sizes, and the final native runs passed.

Reviews are snapshots, refreshed by reopening. This is a two-way review feature,
not a merge editor: conflicts, binary/non-UTF-8 data, symlinks/submodules and files
over 1 MiB are rejected. Linux, other Neovim versions, cross-architecture
Neovim, and a new load comparison were not exercised. All test daemon/config/Git
state was isolated; user hooks and the live daemon were not replaced.


## File-only close behavior (2026-09-08)

File-only tabs and editor-caption X buttons now check buffers off the render
thread. Clean editors receive a normal Neovim `:qa`, which also protects against
edits arriving after the status check. Unsaved buffers show file-specific Save
and close / Discard changes / Cancel choices. Shell or active-agent tabs retain
the session-termination prompt. Unknown editor status never silently discards
changes. Unrelated terminals stay interactive during editor-close checks.

Double-click file opening ignores the second click, avoiding duplicate editor
processes and swap-file prompts. The classification regression and Clippy passed.
The full workspace suite passed (55 tests). The native `file_close_smoke.py`
verified a double-click creates one editor, clean files close directly, dirty
files can cancel then save/close, and every original shell PID remains running.
An initial idle screenshot timed out after the close itself completed; the final
fixture checks editor/layout completion directly and passed. All test state was
isolated, and no live user session or hook configuration was changed.

## In-file unsaved close bar (2026-09-14)

Dirty file close no longer uses a floating `"Close file"` window. A red in-pane
bar offers Save and close / Discard changes / Cancel; other terminals keep
keyboard focus. Extra close while the bar is up does not start another Check.
Discard treats `Stopping` as closed so the prompt does not return. Clippy
passed with warnings denied. Terminator unit tests passed (105). Native
`cargo xtask gui file-close` passed, including an `unsaved-close-bar` capture
of the bar over a dirty buffer. Isolated harness only; live daemon untouched.

## Flat tab strip and inline naming (2026-09-08)

The top-level strip now uses flat 32-point tabs, a high-contrast terminal icon,
visible X controls, and a muted active underline. Removed the separator widget
and vertical item spacing between the tab strip and pane area, leaving a 1-point
boundary. Pane-caption spacing is also reduced.

The rename popup is removed. Renaming starts inline in the workspace-tab label,
pane caption, or Projects row that invoked it. Existing text is selected on entry;
Enter saves, Escape cancels, and valid edits save on focus loss/navigation.
Terminal input remains suppressed while editing a title.

The inline rename regression passed for Enter/save and Escape/cancel on all three
surfaces. The full workspace suite (54 tests), formatting and Clippy passed.
`scripts/inline_rename_smoke.py` verified native renaming on all three surfaces
without changing shell PIDs. An initial native capture timed out; the repeated
run completed and produced the inspected [flat layout](screenshots/inline-tabs/renamed.png)
and [inline editing](screenshots/inline-tabs/editing-inline.png) captures.
No live sessions, user hook configurations, or other projects were changed.


## Focus fade and editor close button (2026-09-08)

The focused pane keeps its full 2-point accent outline for 200 ms, then fades over
one second to a 1-point outline at approximately 27% opacity. Repainting is
requested during the transition; steady focus does not restart the animation.
File-editor captions now include an X button with a Close editor tooltip, using
the existing close/save choices. Historical editor views close directly.

The fade regression and Clippy passed. `scripts/focus_editor_close_smoke.py`
clicked the native editor X and confirmed termination, verified removal of only
that editor view, and verified every original shell remained running with the
same PID. Captures show [initial emphasis and X](screenshots/focus-and-close/strong-border-editor-x.png)
and the [soft steady border after editor close](screenshots/focus-and-close/soft-border-editor-closed.png).
All fixture sessions and configuration were isolated.


## Top-level workspace tabs (2026-09-08)

Legacy layouts are wrapped intact into a single initial tab. The versioned layout
JSON includes the selected top-level tab and all split trees, and unknown versions
are never overwritten. SQLite and the daemon request protocol are unchanged.
Closing a tab can background its sessions; sidebar navigation restores the same
process and selects the correct tab. Editor exit only removes its own view and
returns to the other tab's latest pane focus.

- **52 workspace tests passed**, covering migration, independent split/focus
  persistence, sidebar routing, delayed split ownership, editor exit, close
  behavior and unknown-layout protection, alongside existing regressions.
- Formatting, Clippy with all targets/features and warnings denied, and native
  test-support builds passed.
- `scripts/workspace_tabs_smoke.py` passed twice with actual Neovim and PTYs:
  Explorer creates a separate top-level editor tab; splitting the original tab
  leaves the editor layout unchanged; restart restores both tabs; closing a tab
  with Keep running preserves its PID; sidebar reopening reuses that editor PID;
  `:q` preserves all original shell PIDs and their split layout.
- The 50-session/six-pane native smoke passed after legacy-layout migration,
  including Neovim input/save, font/navigation restart preferences, and unchanged
  original shell/editor PIDs after GUI exit.

Captures: [editor tab](screenshots/workspace-tabs/independent-editor.png),
[restored terminal splits](screenshots/workspace-tabs/restored-shell-layout.png),
[after editor exit](screenshots/workspace-tabs/editor-exit-preserves-workspace.png),
[50-session smoke](screenshots/workspace-tabs-50/native-six-panes.png).

The local debug macOS package was rebuilt. No installation, live-session
termination, hook installation, Linux desktop run, or fresh load benchmark was
performed for this UI change. Earlier pane-level-tab fixtures describe the older
UI; `workspace_tabs_smoke.py` is the current tab-hierarchy regression.


## Pane border titles (2026-09-08)

Lower panes now reserve an 18-point caption row inside the top border. It shows
the current terminal title, uses the accent color for the selected pane, and
truncates long titles with a full-title tooltip. The caption focuses its pane;
its context menu and double-click retain the existing rename actions. Top-row
panes retain their existing tab titles without duplication.

Clippy passed. An isolated six-shell native capture verified the renamed title
and its placement above terminal content:
[border title capture](screenshots/pane-border-title.png).


## Focus, rename, editor isolation and split cleanup (2026-09-08)

- Focused terminal panes have a 2-point accent border (blue by default).
- A shared rename dialog is accessible from terminal, tab and Projects-row menus,
  plus a double-click on a top tab. It uses the existing persistent Rename RPC.
- Editor creation captures the originating pane and never reuses a shell process.
  Lower panes receive a separate editor split; top panes receive a new tab.
- Newly ended sessions are removed from the dock, collapsing empty split nodes.
  Editor exit restores its originating live tab without stealing another project's
  focus. Startup also removes stale ended panes. Explicitly reopened History views
  remain available and no historical records are deleted.

Validation: 46 workspace tests passed, including editor origin restoration,
independent project focus, all four split-close directions, stale-pane cleanup,
and rename targeting. Formatting and Clippy with warnings denied passed.
`scripts/editor_lifecycle_smoke.py` opened an actual Neovim editor from Explorer,
verified a distinct PID and seven dock tabs, sent `:q`, verified restoration to
six tabs with every original shell PID intact, and renamed a terminal through its
Projects row. Some early runs exceeded intermediate layout-save waits; the final
instrumented fixture passed three consecutive runs. The [native capture](screenshots/editor-lifecycle.png) shows the renamed
terminal and focus border. Tests used isolated state; live sessions and user hooks
were untouched. Linux native behavior was not rerun for this change.


## Explorer file click regression (2026-09-08)

Explorer file rows now open an editor tab on a single primary click rather than
requiring a double-click. The shared row owns the icon, label and empty space;
right-click retains the existing file action menu. A headless pointer regression
failed before the change and passes afterward at all three row positions, checking
that each click queues exactly one editor creation with no split. Clippy passed.

## Terminal row click regression (2026-09-08)

The selectable label painted over a sidebar row intercepted clicks on its text.
The label is now painted directly, so the full row owns icon, text and empty-space
clicks and invokes the existing terminal/project navigation. A headless egui pointer
regression reproduced the failure on the label before the fix and passes at all
three positions after it. Navigation regressions and Clippy also pass. This is
input-routing evidence; no new native screenshot was needed for the unchanged layout.


## Flat UI follow-up (2026-09-08)


Top-edge panes retain tabs, +, dropdown and directory/editor controls. Lower
panes have no header strip and retain right-click actions, including tab
selection when multiple tabs share a pane. The new ancestry regression covers
nested horizontal and vertical splits. Clicking an existing Git file opens its
editor; explicit staged/working-tree diff actions remain available. Deleted
entries still open their diff.

The attention bar checks managed-hook status off the render thread at startup.
When no hooks or agent events are present it offers **Set up hooks** instead of
claiming all work is caught up. No user hooks were installed in this run.

Validation on macOS/arm64:

- **39 workspace tests passed** and **three terminal-widget tests passed**,
  including copying offscreen history while retaining newlines.
- Formatting, all-target/all-feature Clippy with warnings denied, and builds
  passed. Existing real-PTY coverage remains applicable to unchanged daemon
  behavior; the native smoke exercised the changed widget against real PTYs.
- `scripts/ui_flat_smoke.py` passed at 1x, 2x and narrow width: +/dropdown creation,
  persisted History, settings, Git single-click editor opening, terminal hover
  editor split, and splitting a lower pane through its right-click menu.
- The **50-session/six-pane** smoke passed, including native Neovim input/save,
  original PIDs after GUI exit, and persisted user font/navigation preferences.
- Fixture windows now have a test-only repaint clock, explicit pixel-size setup,
  and visibility handling. This resolved intermittent idle native captures and
  lets timed raw pointer input run without physical mouse movement. Normal builds
  exclude this test support.

Captures: [workspace](screenshots/flat-100/pane-controls.png),
[settings](screenshots/flat-100/settings.png),
[terminal menu](screenshots/flat-100/context-menu.png),
[hover actions](screenshots/flat-100/hover-menu.png),
[Git opening](screenshots/flat-100/git-open.png),
[2x Git](screenshots/flat-200/git.png),
[narrow settings](screenshots/flat-narrow/settings.png),
[50-session smoke](screenshots/flat-50-sessions/native-six-panes.png).

A local debug macOS package was rebuilt. No installation, live-daemon replacement,
Linux desktop run, or live-provider event verification was performed.

## Plan 3 implementation evidence (2026-09-08)

This section covers the current UI/configuration/performance changes. Earlier
sections below describe previous revisions and do not establish current Linux or
live-provider behavior.

- `cargo fmt --all --check`, workspace Clippy with all targets/features and
  warnings denied, workspace build, and the binaries/examples test-support build
  passed on macOS/arm64 with Rust 1.97.1.
- **38 workspace tests passed**, including TOML defaults/comment preservation,
  external-edit conflicts and invalid values, preview/cancel, persisted History
  expansion, pane removal during asynchronous creation, stale refresh results,
  partially staged diffs, subprocess overflow/deadlines/inherited pipes,
  legacy process identity, snapshot envelopes/generation, and history
  rotation/retention/clear/remove ordering.
- **Two additional vendored terminal tests passed** for quoted spaces,
  punctuation, wrapped logical lines, scrolling and wide-cell hit testing.
- The native filesystem test passed for atomic save/rename, deletion, suspended
  hidden panels and immediate refresh on reopening. macOS FSEvents were
  suppressed by the execution sandbox; this test and the complete test suite
  passed with native event access outside it. Error fallback remains covered by
  code inspection; no Linux watcher failure was injected.
- The real-PTY integration suite passed: reconnect keeps shell PID, cwd preserves
  ownership, hooks/dismissal/authentication, blocked-input Stop, retained history,
  restart without relaunch, conditional `Unchanged`, and legacy full snapshots.
  The archived old daemon also accepted and ignored the new envelope hint.
- The existing **50-session/six-pane** native smoke passed, including Neovim
  input/save, original PIDs after GUI exit, font preference persistence, and
  restart navigation preferences.
- Extended native raw-pointer fixtures passed at **1x, 2x, and 900-point narrow
  width**: click pane +, click dropdown then Split right, expand History and check
  persistence, inspect Settings and Git, hover a printed relative path, and click
  Open in editor split to open the originating project's file. Narrow settings
  height is bounded to leave Apply/Cancel visible. These are native renderer and
  synthetic input results, not physical-device/accessibility certification.

Screenshots (actual native captures):

- [50 sessions / six panes](screenshots/plan3-50-sessions/native-six-panes.png)
- [Pane controls](screenshots/plan3-100/pane-controls.png),
  [History](screenshots/plan3-100/history.png),
  [Settings](screenshots/plan3-100/settings.png),
  [hover actions](screenshots/plan3-100/hover-menu.png)
- [Git at 2x](screenshots/plan3-200/git.png),
  [narrow settings](screenshots/plan3-narrow/settings.png)

### Bounded before/after comparison

Baseline source: archived Git commit `61d4cb890c1360f74495525ca42583283dc6523d`.
Both builds ran sequentially for 30 seconds with isolated daemon state, 50
sessions, six subscribers, twelve output-producing shells and 100 ms snapshot
requests. No GUI rendered during this load comparison.

| Measurement | Baseline | Updated |
| --- | ---: | ---: |
| Snapshot requests | 290 | 290 |
| Snapshot response frame bytes | 5,614,690 | 23,746 |
| Unchanged responses | 0 | 289 |
| Snapshot p50 | 7.256 ms | 6.379 ms |
| Snapshot p95 | 12.763 ms | 11.180 ms |
| Peak daemon RSS | 42,544 KiB | 43,376 KiB |
| History files observed | 413 | 50 |
| History metadata changes observed | 2,948 | 1,104 |

Response bytes decreased by **99.6%**. Peak RSS was approximately unchanged
(832 KiB higher). History activity is 100 ms filesystem metadata sampling, not a
kernel file-open/write trace. These measurements do not establish GUI latency,
long-term memory behavior, or production load. The shutdown flush and later GUI
polish do not change the measured steady-state daemon paths.

A separate short GUI fixture counted actual helper invocations through isolated
PATH wrappers. Each visibility case lasted six seconds:

| Helper calls | Baseline | Updated |
| --- | ---: | ---: |
| Git with Explorer visible | 12 | 6 |
| Git with sidebar hidden | 10 | 0 |
| ps for one hook ancestry lookup | 5 | 1 |

Machine-readable evidence: [before](plan3-load-before.json),
[after](plan3-load-after.json), [helper counts](plan3-command-counts.json).
Reproduction scripts: `scripts/compare_load.py`, `scripts/command_counts.py`,
`scripts/ui_plan3_smoke.py`. Helper counts are from their own fixtures, not
estimated kernel process counts during the 30-second workload.

### Implementation and remaining platform limits

Revision increments were checked across session creation/exit, cwd, rename,
layouts, selection, settings, hook/notification state, truncation and degraded
storage. The missing history-queue saturation increment was fixed. Terminal
bytes stay on the PTY stream and do not force full metadata snapshots.

History retains the existing segment format and reads legacy files. Its worker
owns buffered writers, ordered clears/removals and flush barriers; it rotates at
4 MiB/five minutes, flushes every 250 ms and checks age every 30 seconds. A
removed-session tombstone rejects late producer messages. Shutdown waits for a
history flush before exiting.

No installation or live-daemon replacement was performed. The package is a
local macOS development bundle. Current Linux desktop/Wayland behavior,
provider-specific live hooks, linked-worktree watcher events and physical
Cmd/Ctrl-click input have not been exercised in this run. Linked-worktree Git
metadata roots and modifier handling were inspected in code. Persistent state
subscriptions, parser consolidation and which/crossterm replacements remain
deferred as planned.

## Environments

- macOS 26.6.2 (25G83), Apple Silicon/arm64, Rust 1.97.1.
- Linux: official `rust:1.97-bookworm` container, Debian 12/arm64, native X11 rendering under Xvfb. The image digest used was `sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97`.

## Passing checks

- Workspace formatting and Clippy with all targets/features and warnings denied.
- 14 focused Rust tests: layout round trips, path/line parsing, zsh/bash/sh executable preference and fallback, notification/state separation, duplicate/late events, recovery, frame-size rejection, shell quoting, SQLite, safe replay, and non-destructive JSON/TOML hook installation.
- Real-PTY integration on macOS and Linux: same shell PID after detach/reattach, directory changes without project reassignment, hook delivery, dismissed notification retaining waiting state, invalid authentication rejection, persisted layouts/output, and daemon restart without automatic process launch.
- A regression check verifies that stopping a session remains responsive when a large paste blocks its PTY writer.
- macOS native renderer capture with **50 live sessions and six visible panes**, including embedded Neovim. Test-only synthetic keyboard events travel through the actual widget → PTY bridge → daemon → Neovim and save an isolated fixture. GUI exit retains the original session PIDs.
- Linux native renderer capture under Xvfb with six visible panes and Neovim; the same input/save and GUI-exit preservation assertions pass.
- Muse 1.0.3-R2198.1 offline echo provider: real SessionStart/UserPromptSubmit/Stop/SessionEnd hooks remain one correlated invocation, deliver completion, and retain a resume template. Muse removes the terminal variables from hooks; the test exercises the verified-ancestor fallback.

The native captures are generated at `.artifacts/native-six-panes.png` on macOS and `target/linux/artifacts/native-six-panes.png` by the container run. They show actual rendered terminal/editor contents, not a mockup. Synthetic focus/input exercises the application path but is not proof of every physical keyboard or accessibility interaction.

## Bounded workload

An initial daemon revision completed **600.14 seconds** with 50 sessions, six attached streams, twelve output-producing shells, and no model/network calls:

| Measurement | Result |
| --- | --- |
| Snapshot RPC p95 | 13.17 ms |
| Peak daemon RSS | 208,576 KiB (about 204 MiB) |
| Bytes delivered to the six subscribers | 1,177,618 |

The report is `.artifacts/load.json`. This run preceded later terminal-query, native-dialog, and shutdown refinements; the final code is covered by the subsequent functional/native regressions. It is transport/resource evidence, not a ten-minute GUI benchmark or indefinite leak proof.

## Limits of this evidence

- Input-to-paint p95, project-switch p95, hardware GPU frame rate, and the proposed aggregate memory ceiling have not been established by instrumented measurements.
- Native picker calls use `rfd` with the current directory and parent window, and both platform builds pass. Automated selection/cancellation through real macOS or Linux desktop picker UI was not completed.
- OS notification permission prompts and click activation need a real logged-in desktop validation matrix. In-app hook notifications are covered by tests.
- Linux testing used X11/Xvfb; a real Wayland desktop and additional CPU/OS distributions are not covered.
- Claude/Codex/OpenCode/Grok installed versions and public schemas were inspected, and installer/normalizer fixtures pass. No paid provider model runs were used. Their complete live event combinations remain provider/version-specific validation work.
- Custom shell startup arrangements, every user's Neovim plugin set, and vendor-specific terminal extensions are not exhaustively tested. The app deliberately loads the user's editor configuration; a user-configured `Lexplore` startup action can add an editor-side file tree.

No agent accounts/configurations were changed during validation. Hook installation tests used isolated fixture directories. No deployment, publication, notarization, or external message sending was performed.

## Project navigation / Islands implementation — 2026-09-08

This section supersedes the earlier 14-test count for this change. Current checks:

- Formatting and warnings-denied all-target/all-feature Clippy pass.
- **21 Rust tests** pass on macOS and Linux, including cancellation/error,
  delayed selection result, A→B→A split/focus, hidden-session ownership/context,
  sidebar/scope/expansion restart, bounded width, unsupported preference schema,
  and migration acknowledgment regressions.
- `python3 scripts/integration.py` passes on both platforms. Added real-daemon
  checks open B then A via a symlink and the canonical path without duplicating
  projects or changing session IDs/PIDs; invalid directory/file paths preserve
  selection. Existing real-PTY, hook, history and recovery assertions still pass.
- Native fixtures at **1 and 2 pixels per logical point**, with **50 live sessions,
  six panes and embedded Neovim**, pass on Linux/X11 under Xvfb. macOS captures
  at both scales also passed before the final shutdown flush; final-run results
  are recorded below. Fixtures verify saved Neovim input, unchanged shell/editor
  PIDs and six persisted tabs after GUI exit.
- Native restart checks set an existing font size to 16, verify migration to 13
  and its persisted marker, change to 18, and reopen the GUI to verify 18 remains.
  Git/collapsed sidebar, width 370, All projects and collapsed project expansion
  also survive reopening. Settings use isolated shell/editor fixtures.
- The first native run exposed a nested font lock in bold glyph rendering; it was
  fixed before successful captures. Initial 200% captures used undersized physical
  windows; the fixture now scales its initial window and Linux uses a 3200×2000
  Xvfb display. macOS limits the physical window height to the available desktop.

Implementation captures (native renderer, no personal project/session data):

| Platform | 100% | 200% |
| --- | --- | --- |
| macOS | [Six panes](screenshots/macos-100/native-six-panes.png) | [Six panes](screenshots/macos-200/native-six-panes.png) |
| Linux X11 | [Six panes](screenshots/linux-100/native-six-panes.png) | [Six panes](screenshots/linux-200/native-six-panes.png) |

Restart captures are alongside each image as `native-restart.png`. Visual inspection
of native captures finds legible regular/bold glyphs, bounded tab clipping and aligned
block cursors. Terminal text wraps/clips at its PTY boundary; Neovim retains its own
colors. The test intentionally edits only its temporary source file.

Coverage limits: picker cancellation/errors and delayed results are exercised at the
GUI update/state boundary, not by driving OS picker dialogs. Existing-daemon support
uses unchanged Snapshot/SelectProject IPC; an older daemon binary was not separately
exercised. Sidebar state/scope and hidden-session navigation have Rust/serialized
restart coverage, not a complete physical pointer/accessibility interaction matrix.
Mouse reporting and selection share the measured integer geometry in code; exhaustive
mouse coordinate, selection-drag, Unicode/wide-glyph and physical keyboard coverage
is not established. No Wayland or paid live-provider checks were run for this change.
The earlier bounded load report predates this styling work; no new soak is claimed.

Unrelated local `LICENSE`, `AGENTS.md` and planning edits were preserved. No live user
daemon was restarted, and no personal Neovim configuration or agent credentials were
modified. Font release provenance and licenses ship with the package.

Final-run outcome: Linux passes both scales after the shutdown queue flush. macOS
passed both scales before that final flush, but subsequent final-code attempts timed
out before capture. Test-only stage logging confirms the first frame initialized;
a two-second sample of the isolated GUI found the main thread waiting in the AppKit
native event loop, with the IPC worker alive (not the earlier font-lock deadlock).
This desktop/event-delivery limitation remains unresolved; final macOS smoke after
shutdown-flush changes is therefore **not claimed passing**. Existing macOS images
are the successful pre-flush captures: 1440×900 at 100%, 2880×1304 at 200% (desktop
height constrained). Final Linux images are 1440×900 and 2880×1800.

`python3 scripts/package.py` rebuilt the optimized macOS bundle at
`target/package/Terminator.app`, signed ad hoc with bundled font licenses/provenance.
This is a local bundle build, not an installed-app launch, notarization or publication.

## Tab bar / Explorer follow-up — 2026-09-08

Added a native dock-tab **+** action targeting the clicked pane, persistent
**Show ignored files** (off by default), and horizontal folder/agents/branch icons
at the top of the right sidebar. Git ignore classification uses batched,
NUL-delimited `git check-ignore --stdin -z` outside rendering. Git metadata is
hidden with ignored entries; ordinary dotfiles and the `.gitignore` file remain
visible. The toggle is applied to cached entries immediately.

Formatting, warnings-denied workspace Clippy and all **22 Rust tests** pass.
The new Git fixture covers ignored directories/files, negation, tracked files,
ordinary dotfiles and `.git` metadata. No daemon changes or live-session restarts
were needed. Previous vertical-strip captures above describe the earlier revision.

The follow-up macOS native six-pane smoke and restart fixture passes, including
Neovim input/save, unchanged session PIDs and preference persistence. Inspected
[current native capture](screenshots/sidebar-horizontal/native-six-panes.png):
all six tab bars expose +, and the horizontal icon row and ignore toggle are visible.
Physical clicking of every new control and Linux native rendering were not rerun.
The optimized app bundle was rebuilt and signed ad hoc.

## UI cleanup and external editors — 2026-09-09

Implemented the ten-item root plan: project tabs share a 40-point window header,
with project/window controls on the left and Explorer/Agents/Git/Settings on the
right. The toolbar is removed; `+`, existing split menus/shortcuts, Explorer,
and configurable `command+O` provide the actions. Connection state occupies the
bottom status row. Attention defaults to the right sidebar and migrates once,
only after the existing Settings request is acknowledged. Failed updates retry
with a delay; in-flight requests do not duplicate. Later placement choices survive
restart. IPC, SQLite, workspace layout versions, and daemon ownership are unchanged.

Explorer retains its tree and ignored-file toggle, removes its headings/directory
label/bottom separator, and shows Git colors on filenames, icons, and badges.
Its icon tooltip carries the effective directory and confirmation status; the Git
view retains its directory heading and separators. Git
refresh builds deterministic file/folder lookups, including individual untracked
descendants. Conflicts use `!`, untracked files `U`, and tracked files their status
letters. Project selection now highlights; project/session hierarchy and separate
expansion/selection targets remain intact.

External settings offer System default, VS Code, Cursor, RustRover, Zed, and Custom.
Custom arguments remain individual strings and the absolute path is appended
without shell interpretation. The draft test action uses the native file picker
without saving. Launchers resolve through existing GUI search paths; spawn errors
and unsuccessful exits reach the status bar. Stderr storage is bounded to 4 KiB
and displayed diagnostics to 2,048 characters. Process waiting and pipe draining
run off the GUI/settings workers, without timing out or killing a running editor.

Validation performed on the final behavior:

- `cargo fmt --all --check`, locked workspace build, and warnings-denied Clippy
  with both default and all features passed on macOS. All **73 Rust tests** passed
  on macOS and Linux. New tests cover decoration precedence, deterministic folder
  aggregation, untracked descendants, literal argument/path handling, preset
  round trips, missing/spawn/exit failures, bounded diagnostics, long-running
  launch responsiveness, and migration acknowledgement/retry/persistence.
- `python3 scripts/integration.py` passed on macOS and Linux: original PID on
  reconnect, directory/project identity, hook delivery, dedup/dismissal, legacy
  snapshot compatibility, and no automatic launch after daemon restart.
- The 50-session native smoke passed on macOS and on Linux/X11 under Xvfb at 1×
  and 2×, including Neovim input/save and unchanged original shell PIDs on GUI
  restart. Linux used the repository's `scripts/linux-ci.sh` in an isolated
  `rust:1.97-bookworm` container with read-only source and dedicated build cache.
- macOS `workspace_tabs_smoke.py`, `inline_rename_smoke.py`,
  `editor_lifecycle_smoke.py`, `file_close_smoke.py`, and
  `focus_editor_close_smoke.py` passed. These exercise tab/split ownership,
  selection/close/rename, clean and dirty editor-close behavior, and original
  shell PID preservation. Their pre-existing screenshots were restored afterward.
- `ui_cleanup_smoke.py` passed at normal and narrow widths at both 1× and 2× on
  macOS. It verifies an overflowing eight-tab header with a usable `+`, sidebar
  navigation back to the first tab, Settings independent of the selected tool,
  an edited but unsaved custom-editor draft, and one-time Attention migration
  followed by a user placement change and restart.
- `review_smoke.py --gui` and `legacy_diff_smoke.py` passed on macOS: staged
  and working-tree CodeDiff snapshots remained read-only with Git unchanged,
  native review actions worked at both scales, and a simulated older daemon
  used the built-in renderer with zero unsupported requests. Existing screenshot
  artifacts were restored afterward.
- `external_editor_smoke.py` passed: an isolated controlled launcher received a
  literal argument containing spaces and shell syntax plus the absolute file
  path. Settings opened while it was still running, and its exit status 7 and
  stderr appeared in the GUI. This reproduces the formerly discarded failure
  class; it does not establish the cause of the originally reported user launch.

Captured and inspected the original committed UI (`f1f8555`) from a temporary
source copy and the updated native renderer:

| Capture | Evidence |
| --- | --- |
| Before | [Original macOS UI](screenshots/ui-cleanup/before/macos.png) |
| After, same 50-session fixture | [Updated macOS UI](screenshots/ui-cleanup/macos-50-sessions/native-six-panes.png) |
| macOS 1× | [Explorer](screenshots/ui-cleanup/macos-1x/explorer.png), [editor draft](screenshots/ui-cleanup/macos-1x/editor-settings.png) |
| macOS 2× | [Explorer](screenshots/ui-cleanup/macos-2x/explorer.png), [overflow](screenshots/ui-cleanup/macos-2x/overflow.png) |
| Narrow macOS | [1× editor settings](screenshots/ui-cleanup/macos-narrow-1x/editor-settings.png), [2× overflow](screenshots/ui-cleanup/macos-narrow-2x/overflow.png) |
| Linux X11 | [1× six panes](screenshots/ui-cleanup/linux-x11-1x/native-six-panes.png), [2× six panes](screenshots/ui-cleanup/linux-x11-2x/native-six-panes.png) |
| External failure | [Responsive Settings and exit diagnostic](screenshots/ui-cleanup/external-editor/external-failure.png) |

Limits: these renderer captures and synthetic-input fixtures do not verify physical
window-manager dragging, maximize/minimize/resize interactions, macOS traffic-light
pixels (native chrome is outside the renderer capture), Wayland, or actual native
file-picker selection. Preset mappings are tested, but every installed third-party
editor was not launched.
The first before-capture attempt also timed out; rebuilding the committed source with
`test-support` subsequently produced the before capture successfully. No production
daemon was restarted, user hook configuration changed, package installed, or
release published.

The final passive capture exposed a partially painted frame. Test-support now
requests screenshots after UI layout, skips discarded passes, and filters real
desktop keyboard/clipboard/pointer input out of isolated fixtures. Two focused
regressions cover final-pass capture and desktop-input isolation; the corrected
50-session capture above was inspected and its restart/PID assertions passed.
Concurrent macOS suite runs intermittently timed out in the existing filesystem-watch
test; standalone workspace runs passed. Native fixtures and the Rust suite should
be run separately on this desktop.

## Rust tooling, native media, worktrees, and controls — 2026-09-09

The follow-up review is implemented. Project-owned Python automation has been
replaced by `crates/xtask`; the old-to-new command mapping is in
[`scripts/README.md`](../scripts/README.md). The application, daemon, hook CLI,
package task, integration/load runners, native fixtures and native-input drivers
are Rust. Existing native dependencies (SQLite, Neovim/CodeDiff, OS APIs) remain.
No Swift source, Electron, Chromium, or webview was added to the application.
The external browser used below is a disposable test dependency, not bundled code.

Changes and compatibility:

- Image clicks open GUI-owned preview tabs. `image`, egui and resvg handle decoding,
  rendering, zoom/pan and SVG rasterization. Files are limited to 32 MiB, raster
  previews to 16 megapixels, and cached textures to 128 MiB. Decode work runs on a
  bounded worker queue; stale generations and closed views discard late results.
  SVG embedded raster images share a pixel budget; external/unsupported references
  produce an explicit error. Animated formats show their first frame. Preview,
  restart, close, corrupt-image handling and explicit text opening allocate no
  editor except when text opening is explicitly selected.
- Version 3 is used only for layouts containing the new Image tab type. Version 2
  and legacy layouts still load; unknown versions are not overwritten. Daemon
  schema and protocol versions remain unchanged.
- The daemon's duplicate Alacritty screen/parser has been removed. `vt100` provides
  the screen and callback API for cursor/device/color replies and OSC 9/777/99
  title/body notifications. Primary DA is VT420+color (`?64;1;2;6;22c`), not VT102
  (`?6c`). DECRQM (`CSI ? … $ p`) reports alt-screen and mouse modes as set/reset.
  OSC 4 palette queries return distinct ANSI colors. The GUI still uses its
  Alacritty-backed widget.
  Notification payloads/queues are bounded and do not create or transition agents.
  Terminal messages are separate from authenticated hook lifecycle events;
  historical replay emits neither notifications nor query replies.
- Explicit worktree CLI operations delegate to Git. HEAD resolves in the selected
  source checkout, not accidentally in the main worktree. A small registry links
  checkouts to projects. Removal is serialized against session creation, rejects
  live sessions, relies on Git's dirty/locked protections, and keeps branch refs
  and session history.
- The private `gui.sock` endpoint enables explicit tab, split, focus, file and
  window operations without changing PTY ownership. Both native socket servers
  clear inherited nonblocking mode before reading frames (required on macOS).
  Repeated ShowSession requests focus the existing view rather than duplicate it.
  New daemon operations are capability gated; old-daemon fallback remains intact.
- Metadata uses Git, optional gh, sysinfo and lsof. PR data is opt-in and cached;
  port discovery is restricted to the session PID/start time and descendants.
  External-browser commands use a local CDP page endpoint through tungstenite.
  Navigation, HTML/CSS snapshots, literal selector/text actions and PNG capture
  are explicit commands; the application does not launch an embedded browser.

Evidence recorded in this run:

- Formatting and warnings-denied workspace Clippy passed. **92 Rust tests** passed
  on macOS with native event access. Model tests now inject worker responses rather
  than start unrelated config watchers; the dedicated filesystem-watch regression
  still exercises actual notifications. macOS FSEvents is not reliably delivered
  inside the coding sandbox, so its native test run used normal host event access.
- `xtask integration` passed: real PTYs, reconnect PID preservation, project/cwd
  identity, hook delivery/dedup/dismissal, blocked-input Stop responsiveness,
  conditional and legacy snapshots, and no relaunch after daemon restart.
  Additional CLI fixtures passed worktree registration/removal guards, branch
  preservation, send/read-screen, OSC/CLI notifications and mocked gh PR metadata.
- Ported macOS fixtures passed smoke (50 sessions), workspace tabs, inline rename,
  editor lifecycle, clean/dirty file close, focus/close, UI cleanup, external editor,
  images, CLI control and terminal actions. CodeDiff reviews passed; the legacy
  proxy initially exposed inherited nonblocking socket mode and passed after that
  transport fix. Original shell PIDs remained unchanged.
- Linux/X11 under Xvfb/Openbox passed 1×/2× smoke and image fixtures, both narrow
  UI-cleanup scales, and focused workspace/editor/rename/review/control fixtures.
  The native input driver verified header dragging, edge resizing, maximize and
  restore, minimize and restore, file selection/cancel, native folder selection,
  and GUI close with the original shell still running. The resize check exposed
  a window-manager pointer handoff issue: consumed mouse-up left egui dragging;
  the handoff now clears that state and places edge hit regions above panels.
- Headless Weston/Wayland passed 1×/2× smoke, image and narrow UI-cleanup fixtures,
  plus CLI GUI-control tests. This establishes Wayland rendering/PTY/input-fixture
  behavior, not every physical compositor-specific window-management gesture.
- A fresh headless external Chromium profile and a local HTTP fixture passed live
  browser navigation, HTML/CSS capture, fill/click with injection-like literal text,
  evaluation and screenshot. No existing browser profile or authenticated site
  was used. PR discovery used a Rust gh fixture; no live PR/provider claim is made.
- The Rust conditional load task passed a short 50-session transport workload and
  produced a JSON report; its comparison task read the report successfully. These
  numbers are not GUI FPS or a before/after terminal-performance benchmark.

Representative captures:

| Behavior | Capture |
| --- | --- |
| Native PNG preview | [macOS preview](screenshots/reuse/image-preview-macos.png) |
| Native SVG preview | [macOS SVG](screenshots/reuse/svg-preview-macos.png) |
| Wayland media at 2× | [Wayland preview](screenshots/reuse/image-preview-wayland-2x.png) |
| macOS native window fixture | [macOS window](screenshots/reuse/native-window-macos.png) |
| Linux native window | [X11 window](screenshots/reuse/native-window-x11.png) |
| Native directory chooser | [GTK chooser](screenshots/reuse/native-picker-x11.png) |
| External browser fixture | [Browser capture](screenshots/reuse/external-browser.png) |

The older validation sections above intentionally retain their historical Python
command names. Current commands are all in the Rust task mapping. No live daemon
was replaced or user hook configuration modified during these isolated checks.

Additional task verification: the Rust command-count runner passed visible/hidden
GUI measurements and ancestry-helper counting. Its log writer now locks and writes
complete records, preventing concurrent helpers from interleaving JSON. Captures
are removed before each fixture and must be freshly produced, so stale files cannot
satisfy validation. The native macOS driver ultimately passed drag, resize, maximize/restore,
minimize/restore, file/folder selection, cancellation and native close while
retaining the original PTY. Its high-DPI conversion uses egui/native scale ratios;
native gestures go through WindowServer and native minimize/close use accessibility
buttons. The fixture filters auxiliary windows and accepts its always-on-top layer.
It uses a non-executing, non-echoing PTY during foreground native-input validation.
Linux native file/folder selection and cancellation are also verified.

The Rust `package --debug` task produced an isolated development `.app`;
`codesign --verify --deep --strict` passed. The packaged GUI contains no
test-support diagnostic code. This was a local ad-hoc-signed artifact only.
A focused SVG regression also passed for a bounded embedded PNG and rejected
external image references; resvg's raster-image feature is explicitly enabled.

Final native runs were serialized on macOS: overlapping fixture windows could
stall initial AppKit rendering. The successful standalone run produced a fresh
capture and verified every native window/picker assertion. No physical-input
coverage on other macOS versions or Wayland compositors is implied.

### Click-positioned dialogs (2026-09-10)

App-owned egui dialogs now open eight logical points from the initiating click,
constrained by egui to the content viewport. They keep their position when the
pointer moves and use a fresh origin on reopening. Asynchronous editor-close
checks retain the initiating position. Native OS file pickers are unaffected.
The initial sizing passes temporarily disable window dragging because egui 0.36
otherwise restores the old title-bar position over `current_pos`; normal dragging
resumes after placement. No daemon ownership or session lifecycle changed.

Validation: 56 GUI unit tests passed, including new reopen/stability and delayed
editor-close/bounds regressions. App and xtask Clippy passed with warnings denied;
formatting and locked offline workspace binary/example build passed. macOS native
`file-close` and normal-size `workspace-tabs` passed, preserving fixture shell PIDs.
The dedicated `popup` fixture passed at narrow 2x scale. Screenshots were inspected:
[normal](screenshots/popups/normal.png), [narrow 2x](screenshots/popups/narrow-2x.png).

The broader narrow-2x workspace-tabs fixture timed out before the popup check:
its inactive source.rs tab close target was clipped by horizontal overflow.
This supports plan-2.md's request for visible tab scroll arrows; the arrow proposal
was reviewed, not implemented in this popup change. Linux desktop behavior was not
rerun for this change. No installed app or running user daemon was replaced.

### Tab overflow and global History (2026-09-10)

Added left/right overflow arrows using egui ScrollArea state; they scroll 80% of
the visible strip and disable at the ends. Active-tab reveal and wheel scrolling
remain available. Project lists and their counts now exclude editor sessions and
ended sessions. A History icon beside Git opens all ended sessions grouped by
project, using the existing session actions and saved-output view. Its selection
persists in UI preferences; old per-project expansion data is retained but unused.
No sessions or history records are deleted by this navigation change.

GUI tests passed (56 existing tests plus the new project/editor/global-scope
rendering regression). Formatting, app/xtask Clippy with warnings denied, and the
locked offline workspace binary/example build passed. The macOS narrow 2x
workspace-tabs fixture now uses both arrows and passes the previously clipped
editor-close check, tab selection, split ownership, GUI restart, and original
shell PID checks. Screenshots inspected:
[arrows and editor-free projects](screenshots/navigation/tab-arrows-2x.png),
[global History](screenshots/navigation/global-history.png).
Linux desktop checks were not rerun; the installed app and user daemon were not
replaced.
The macOS terminal-actions fixture also passed, including persisted global History
selection, Git file opening, terminal context actions, and shell PID preservation.

### Wheel scrolling, scrollbar styling, and collapsible History (2026-09-10)

Vertical wheel input over an overflowing tab strip now scrolls horizontally;
horizontal trackpad input is preserved and modifier-based zoom is not remapped.
App-owned egui scroll areas share slim overlay handles (4-point resting width,
8-point interaction width), subdued opacity, and minimal tracks. OS file pickers
and terminal applications' own text UI remain controlled by those applications.
History projects have clickable chevrons and counts with independent persisted
expansion, reusing the existing preference map.

Passed: locked offline workspace binary/example build, app/xtask Clippy with
warnings denied, formatting, and the project/global-History rendering regression.
macOS native workspace-tabs passed at narrow 2x using vertical wheel events to
reveal both tabs, with selection/split/editor-close and shell PID checks. Native
terminal-actions passed with collapse and expansion checked across GUI restarts.
The 50-session native smoke fixture passed and its scrollbar screenshot was
inspected, along with collapsed History. Linux was not rerun. No user daemon or
installed application was replaced. Icon candidates in ../docs/icon-options are
preview-only, sourced from official repositories with original licenses retained.

### Close buttons on terminal panes (2026-09-10)

Every terminal pane now exposes the existing caption X, with a generic Close pane
tooltip. The existing close-session flow still offers Keep running / Terminate /
Cancel; editor close retains its existing unsaved-buffer checks. Inline renaming
reserves space for the close target.

Formatting, app/xtask Clippy with warnings denied, and the locked offline workspace
binary/example build passed. The macOS `pane-close` native fixture passed: the X
opens confirmation, Keep running removes only the chosen pane, and all original
shell PIDs survive both GUI restarts and pane removal. The confirmation screenshot
was inspected and saved at screenshots/pane-close/confirmation.png. Linux desktop
checks were not rerun; installed application and user daemon remain untouched.


### Maintenance extraction, runtime panic audit, and locked worktrees (2026-09-10)

Moved rendering into `settings_ui`, `sidebar_ui`, `dialogs_ui`, and `workspace_ui`.
`App` retains state, asynchronous updates, lifecycle handlers and startup; the
`workspace` module retains layout models. Rendering method bodies, widget IDs,
action ordering and originating project/tab handling are preserved. Native window
gesture helpers remain beside the application entry point.

The [runtime audit](PANIC_AUDIT.md) reproduced two GUI crashes from malformed saved
layouts: an out-of-range focused node and a missing main surface. Loading now
rejects those layouts through the existing status/read-only preservation path.
Regressions first failed with actual out-of-bounds panics, then passed with the
handling; the app test also checks that no SaveLayout request overwrites the
original JSON. Legacy and version-2/3 layout handling is covered. No daemon panic
was demonstrated; the audit records retained assertions and poisoning risks.

Passed on the local macOS desktop:

- Initial locked all-feature workspace baseline: 95 tests, including 57 app tests.
  App tests ran after each extraction. Settings/sidebar sandbox runs each passed
  56 tests but timed out waiting for filesystem watcher events; the sidebar rerun
  with native access passed all 57, as did the dialog and workspace stages.
- `cargo fmt --all --check` and `git diff --check`.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- `cargo test --workspace --all-features --locked`: 99 tests passed (60 app,
  21 core, 11 daemon, 3 hook, 4 integrations). The final strengthened no-write
  assertion was also rerun independently and passed.
- `cargo build --workspace --bins --examples --features terminator/test-support --locked`.
- `cargo xtask integration`: real PTYs, original-PID reconnect, lifecycle/hooks,
  restart without session relaunch, CLI send/read, metadata and worktree checks.
  The new clean/no-live-session lock refusal retains tracked content, Git
  registration and lock, branch commit and the false application removal marker;
  a subsequent state request succeeds. Unlock/removal then succeeds and retains
  the branch commit. The core helper regression independently covers the same
  lock/unlock preservation boundary.
- `cargo xtask gui all --output /tmp/terminator-maintenance-after`: all 13 cases
  passed serially (smoke with 50 sessions, workspace-tabs, inline-rename,
  editor-lifecycle, file-close, focus-editor-close, ui-cleanup, external-editor,
  images, control, terminal-actions, reviews, legacy-diff).
- `cargo xtask gui ui-cleanup --output /tmp/terminator-maintenance-2x --scale 2`.

Before extraction, the original test-support binary ran ui-cleanup at 1x with
captures under `/tmp/terminator-maintenance-before`. After extraction, the
[Explorer/sidebar](screenshots/maintenance/explorer.png),
[settings](screenshots/maintenance/editor-settings.png), and
[workspace overflow](screenshots/maintenance/overflow.png) PNGs were visually
inspected and each was byte-for-byte identical to its pre-refactor counterpart.
The [2x settings capture](screenshots/maintenance/editor-settings-2x.png) was also
inspected. These are representative captures, not a claim that every temporal
frame is pixel-identical.

The first sandboxed native invocation could not start its isolated daemon
(`Operation not permitted`); the native suite passed with the required local
execution permission. All daemon/GUI/hook fixtures used temporary state, without
changing user hook configurations, installed binaries or live user sessions.
Linux/X11/Wayland, native OS notification permissions/actions, window-controls
and live-provider checks were not run in this macOS maintenance pass. Packaging,
installation, publishing, tagging and daemon replacement were not performed.
Historical evidence above and the former-command mapping in scripts/README.md
are retained; current CodeDiff instructions use cargo xtask.

### Review fixes: worktree use, large snapshots, attachments and editors (2026-09-10)

The preceding review reproduced four failures against isolated daemons: removing
another project's live cwd, exceeding the snapshot frame limit with 130 valid
64 KiB notification details, continuing to accept input after an attachment's
output writer timed out, and passing Neovim arguments to macOS Nano/Pico.

The fixes retain daemon PTY ownership and the database/layout formats. Worktree
removal checks recorded paths and current directories of session processes and
descendants, including symlink aliases; uncertain process inspection refuses the
operation. Snapshot chunking is opt-in on the existing envelope, leaves each
frame below 8 MiB and retains every pending detail. Legacy clients receive the
ordinary frame or an explicit size/update error. Attachment cleanup shuts down
both directions on errors. Terminal editors receive supported arguments and an
absolute file operand; embedded Neovim keeps its RPC/configuration behavior.

Passed locally on macOS:

- `cargo test --workspace --all-features --locked --offline`: 103 tests (60 app,
  24 core, 12 daemon, 3 hook, 4 integrations). The 24 core tests passed again after
  the final response-error wording adjustment.
- `cargo fmt --all --check`, `git diff --check`, and
  `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`.
- `cargo build --workspace --bins --examples --features terminator/test-support --locked --offline`.
- `cargo xtask integration`: existing PTY/auth/hook/recovery/CLI coverage plus
  rejection for recorded cwd, an unreported shell `cd` through a symlink, and a
  descendant's different cwd. Refusals preserve files and registry; an unrelated
  live shell remains running through successful unlocked removal. Dirty/locked
  refusal and branch preservation still pass.
- The same integration command transfers a snapshot over 8 MiB, preserves all
  130 pending notifications and full details through daemon restart, checks
  conditional snapshots and the legacy error, verifies EOF and rejected input
  after a stalled output writer, and reattaches the original shell PID. Temporary
  Nano/Pico/custom-editor executables assert the exact file and position operands.
- Serial macOS native `editor-lifecycle`, `file-close`, `reviews`, and `legacy-diff`
  fixtures passed. Captures are under `/private/tmp/terminator-review-fixes`;
  these were interaction assertions, not a separate visual design review.

Two fixture issues were corrected during validation: interactive macOS sh history
expansion of `$!`, and Darwin rejecting a timeout change after peer shutdown.
The successful run uses `%1` for its sole temporary job and sets socket timeouts
before triggering shutdown.

All integration/native runs used temporary state with local PTY/socket/desktop
permission. Linux/X11/Wayland and live agent providers were not rerun. No user
hooks, installed binaries, live user sessions or running user daemon were changed.

## Red Eye app branding (2026-09-10)

Selected the Red Eye logo for the embedded GUI window icon and macOS/Linux
package icons. `cargo fmt --all --check` and
`cargo check -p terminator -p xtask --locked` passed. Verified the source PNG
matches the selected artwork byte-for-byte, all seven ICNS entries contain PNGs,
and macOS `sips` recognizes the ICNS as 1024×1024. Package creation and native
Dock/desktop appearance were not exercised; running apps and daemons were left
untouched. Linux desktop installations must install the bundled PNG in the icon
theme path or use its absolute path, as described in the branding README.

## Developer ID release signing workflow (2026-09-10)

The manual release workflow now imports the five configured Apple secrets into
a temporary macOS keychain, signs all three executables and the bundle with
hardened runtime and timestamps, requires Accepted notarization, and staples
and validates the ticket before archiving. Gatekeeper assessment must pass.
Credential cleanup runs on failure as well as success. Local packaging is unchanged.

Validation: parsed workflow YAML, checked all eight embedded shell blocks with
`bash -n`, asserted that `workflow_dispatch` remains the only trigger, checked
local `notarytool submit --help` options, and passed `git diff --check`.
No credentials were imported locally and no signing, notarization submission,
GitHub run, or release publication was performed. Apple credential validity,
notarization acceptance, and downloaded-app launch remain unverified until a
manual release and native launch check.

## Left sidebar agent bell (2026-09-14)

- Added a bell bar above Projects with waiting-agent and unread-notification counts. Clicking toggles the left sidebar between Projects and the shared Agents view; the choice persists in UI preferences.
- Passed `cargo fmt --all --check`, `cargo check -p terminator --all-targets --all-features --locked`, and the three `preferences::tests` with all features and the lockfile enforced, including legacy defaults and persistence of the left Agents view.
- The initial locked check was blocked by concurrent dependency edits; an offline check reconciled Cargo.lock with those manifests before the successful locked check.
- Native visual/click behavior was not exercised; no live daemon was restarted.

## Agent terminal wheel scrolling (2026-09-14)

- Fixed wheel routing for applications advertising terminal mouse reporting.
- All 12 vendored egui_term tests passed, including three scrolling regressions.
- `cargo check -p terminator --all-targets --all-features --locked --offline` passed.
- Live agent/native GUI scrolling was not exercised; no running sessions were restarted.
- Formatting and `git diff --check` passed. Final strict Clippy was blocked
  by concurrent clipboard dependency edits requiring a lockfile update; that
  unrelated lockfile was not regenerated for this fix.

### Native agent-bell follow-up (2026-09-14)

- Added `cargo xtask gui agent-sidebar` to the repeatable native suite. It uses an isolated daemon, two real shell PTYs, and synthetic hook events; no live provider account or user daemon is involved.
- Passed at scale 1 and at `--scale 2 --narrow`. Automated clicks verified opening Agents, selecting an agent in another project, returning to Projects, and retaining the sidebar choice across GUI restarts. Snapshot assertions verified waiting/unread counts and a waiting-to-running update delivered while the GUI was open. Original shell PIDs survived all GUI launches.
- Inspected screenshots and fixed a black-on-dark bell caused by SVG `currentColor`; the icon now uses the existing white-source tint convention. Final captures: `/tmp/terminator-agent-sidebar-validation/agent-sidebar/` and `/tmp/terminator-agent-sidebar-retina/agent-sidebar/`.
- Passed the workspace binaries/examples test-support build, all 82 app tests (`cargo test -p terminator --all-features --locked`), formatting, and strict Clippy for app/xtask across all targets/features.
- Native fixtures required execution outside the filesystem sandbox for local daemon sockets/PTYs. Live provider hook delivery and Linux rendering were not tested.

## Terminal separator focus flash (2026-09-14)

- All 13 vendored egui_term tests passed, including a headless focus regression for Tab, Shift+Tab, arrows, Escape, and repeated presses.
- `cargo check -p terminator --all-targets --all-features --locked --offline` passed.
- Vendored formatting and `git diff --check` passed.
- Live native rendering was not exercised; no running daemon or sessions were restarted.


## 2026-09-14 — Installation, daemon health, DMG layout and history bursts

Implemented a macOS installation preflight before state creation/daemon startup,
app-owned native startup alerts, executable-access checks and 0755 package modes.
Authenticated snapshots refresh optional daemon path/helper-health metadata before
conditional-cache checks. Capability-gated retirement now also covers idle
same-version daemons with broken or unreported helper health; live, unknown/newer
and unsupported daemons remain protected. A new daemon discards stale runtime
warnings while preserving per-session history-loss flags.

The bounded history queue now waits for capacity without holding parser/state
locks rather than dropping output. Disconnected workers and storage-write errors
have distinct loss messages. Slow storage can apply backpressure to noisy PTYs.

Evidence on local macOS:

- `cargo test --workspace --all-features --locked`: 138 unit tests passed. The
  initial sandbox run could not create Unix sockets or receive filesystem-watch
  events; the complete run passed outside that sandbox using isolated fixtures.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
  formatting and whitespace checks passed. Release workflow YAML parsed locally.
- `cargo xtask integration`: real PTY saved all 6,291,456 payload bytes without
  truncation. An isolated copied daemon detected helper chmod/removal, invalidated
  the conditional snapshot, preserved the live session PID and rejected idle
  shutdown until the session ended. Existing shutdown-race (8 runs), reconnect,
  snapshots, hooks, worktree and terminal-editor checks also passed.
- `cargo xtask gui updates --output /tmp/terminator-install-validation/updates`:
  native GUI replacement preserved shells, unsaved editor state, layouts and
  session identities. The fixture now models a user Applications directory in
  its isolated home rather than bypassing installation checks.
- Local ad-hoc `cargo xtask package --debug` and `cargo xtask dmg` completed.
  The mounted DMG had `.DS_Store`, its rendered background and the Applications
  symlink; every bundled executable was 0755 and strict deep signature checking
  passed. Finder was visually inspected at 640 × 440; screenshot:
  `/tmp/terminator-install-validation/dmg-finder.png`.
- The isolated non-installed bundle launch exited successfully without creating
  its configured data directory. A screenshot of the startup alert itself was
  not retained; the launch-location decision is covered by unit tests.

Final local package/DMG: `/tmp/terminator-install-validation/final/`.
Unit log: `/tmp/terminator-install-validation/unit-tests.log`.
The original 20-second generic command deadline was too short for DMG creation;
its dedicated task now allows five minutes and successfully completed. The first
attempt's temporary mount was detached before retrying.

No existing user daemon was restarted or terminated. No signing credentials,
privacy settings, installation in Applications, hosted CI, universal release,
Developer ID signing/notarization or publication were changed or verified by
these local checks. The validation package is a local development build.


## Pending-change review fixes (2026-09-14)

Inline attention selection no longer suppresses terminal keyboard focus. A
notification outside the visible inbox scope still opens its detail window.
Resolved notices remain available below unresolved events and are excluded from
the waiting count. Inbox buttons wrap at narrow sidebar widths. The unsaved-close
bar measures its content, with the message above wrapping actions so long errors
cannot push Save, Discard or Cancel beyond the pane.

Validation: all 174 workspace tests passed (117 app tests), including five new
regressions for focus/modal scope, resolved waiting counts and ordering, and
button containment at 170/220/320/640 px. Formatting, diff whitespace checks and
strict workspace Clippy passed. Socket/watcher tests required execution outside
the sandbox; the inherited TERMINATOR_SESSION_ID was removed for the test run.
The test-support build passed. Native `file-close` and `agent-sidebar --narrow`
fixtures passed in isolated data directories; captures of the close bar and
wrapped inbox actions were inspected. Captures are under
`/tmp/terminator-review-file-close/file-close` and
`/tmp/terminator-review-agents/agent-sidebar` for this run (temporary artifacts).
No live daemon sessions or provider hooks were changed.

The icon generator now reads `red-eye-source.png` from its own branding directory
and uses a unique temporary iconset. Running an isolated copy with only the script
and source artwork reproduced all three shipped icons byte-for-byte. Apple's
image conversion services required execution outside the sandbox. No release or
package was published.

## Interactive terminal colors (2026-09-14)

PTY creation now matches Orca's color-environment cleanup: remove `NO_COLOR`
unconditionally, and remove `FORCE_COLOR` / `CLICOLOR` only when exactly `0`.
This applies to shells, editors, and reviews before spawning; shell startup files
can still set user preferences. Other environment values remain intact.

Validation: all 19 daemon tests passed, including two regressions covering
suppression removal and preservation of other preferences. Daemon build, strict
all-target/all-feature daemon Clippy, formatting, and diff whitespace checks
passed. The installed live daemon was not replaced or restarted; existing
sessions and native agent rendering were not changed or revalidated.

## Pre-commit review menus and Hebrew glyphs (2026-09-14)

Git row menus now put the matching review first: Working tree diff for Changes
and Untracked, Staged diff for Staged Changes. Conflicts offer editor actions
instead of unsupported two-way reviews. Opening a file still opens its editor.
No staging or commit is required to review working-tree changes.

A new font coverage regression failed on Hebrew aleph (U+05D0) with the old
terminal font stack. Bundled, unmodified Noto Sans Hebrew now supplies Hebrew
letters to normal/bold terminal and UI families; its OFL and source/hash are
recorded with the font assets. The test now passes for all 27 Hebrew letters
across all four families. This fixes missing glyphs, not bidirectional terminal
layout; the native terminal still paints characters in application cell order.

Validation: all 120 app tests passed, including pre-commit/partially-staged menu
routing and Hebrew coverage. Workspace test-support build, strict workspace
Clippy, formatting, and diff whitespace checks passed. The isolated native
`reviews` fixture passed after sandbox PTY startup required elevated execution:
staged/working snapshots remained read-only, the native Working tree diff menu
opened an untracked file, and review close preserved the fixture shell. Hebrew
glyphs were visually inspected in `/tmp/terminator-hebrew-git-review/reviews/review.png`.
Actual Hebrew keyboard-layout input and provider-specific behavior were not
exercised. No installed application or live daemon was replaced.

## Orca-style context menus (2026-09-14)

Right-click menus, pane/sidebar dropdowns, and the terminal hover action popup
now use Orca's dark menu tokens from `ui/context-menu.tsx` and `.dark` in
`src/renderer/src/assets/main.css`: fill `#0a0a0a`, text `#fafafa`, muted
`#a1a1a1`, hover/border white/14, 11px corners, 12px labels. App chrome and
Settings windows keep the existing grey theme.

Validation: all 125 app tests passed, including `menu_style_matches_orca_dark_tokens`
and `context_menu_paints_orca_fill_and_text`. Strict terminator Clippy and
formatting passed. Native right-click appearance was not re-captured; no
installed application or live daemon was replaced.


## 2026-09-14: Idle upgrades and compact agent cards

- The GUI now attempts the existing safe installation repair when an eligible
  daemon becomes idle, once per generation. Live sessions and active GUI exit
  checkpoints prevent automatic repair; manual retry remains available.
- macOS release packaging defaults to hourly update checks. Agent cards show
  status and a truncated session label without response summaries/details.
- `cargo test -p terminator --bin terminator --locked`: 111 passed, including
  automatic-upgrade idle/exit guards and failure retry suppression.
- `cargo fmt --all --check` and `git diff --check` passed.
- Native visual rendering, real-PTY upgrade continuity, signed updater delivery,
  and installed user preferences were not exercised. No installation or live
  daemon was changed.

## 2026-09-14: App-owned one-minute Sparkle polling

Supersedes the hourly packaging interval above. Terminator follows AppDock's
immediate/60-second information-probe schedule and disables Sparkle's separate
timer after migrating the existing automatic-check preference. Active Sparkle
sessions and GUI exit suspend polling. A successful probe enters Sparkle's normal
background update flow once per discovered version per launch, preserving its
background-download preference and manual check controls.

- `cargo test -p terminator --bin terminator --locked`: 113 passed, including
  immediate/minute scheduling, busy/in-flight suppression, sleep without bursts,
  failed-probe suppression, disable handling, and once-per-version offers.
  The sandbox run had eight socket/watcher failures; the same full suite passed
  outside the sandbox with isolated test fixtures.
- `cargo clippy -p terminator -p xtask --all-targets --all-features --locked -- -D warnings`,
  `cargo fmt --all --check`, and `git diff --check` passed.
- Sparkle selectors were checked against its public SPUUpdater API documentation.
  Native delegate delivery, installed preference migration, signed feed delivery,
  download/install behavior, and live minute timing were not exercised. No user
  defaults, installed app, production feed, or live daemon was changed by validation.

## 2026-09-15: Scrolling, local DMGs, access recovery, idle close and launch kit

Implementation is local and separate from the running installation. Existing dirty
backlog changes/task deletions and concurrent appearance/external-editor edits were
preserved. No user agent configuration, installed application, or live daemon was
replaced; no provider, Store submission, release, or Product Hunt post was launched.

### Automated and native evidence

- Workspace tests passed: 133 app, 28 core, 19 daemon, 5 hook, 6 integrations and
  3 xtask tests (194 total). Run with `env -u TERMINATOR_SESSION_ID cargo test
  --workspace --all-features --locked --offline` outside the filesystem/socket
  sandbox. Strict workspace/all-target/all-feature Clippy and formatting passed.
- Separate vendored terminal suite: 15 passed, including mouse-report routing,
  fractional points/lines, viewport page units, Shift bypass under alternate-scroll
  modes, and retained-history anchoring during new output. Temporary standalone
  build products were kept outside the repository.
- `cargo xtask integration` passed functional transport, conditional snapshots,
  hook delivery, same-PID reconnect, restart-without-launch, worktree protections,
  explicit shutdown cleanup, eight creation/shutdown races, 6 MiB lossless history,
  stable helper removal/replacement, large snapshots and stalled attachment cleanup.
  Its large-input check exposed and verified the fix for holding the close lock
  during a blocking PTY write. Input admission/in-flight bookkeeping now excludes
  idle closure while blocking writes remain outside the shared lock. Fixture
  processes also clear inherited live session IDs/tokens.
- `cargo xtask idle-close` passed zsh and bash initial/completed prompts, waiting
  builtins, foreground commands, background/stopped jobs, partial input, active
  agent state, all-target preflight, and stale generations. Unsupported sh retained
  confirmation. Fish was not installed and was explicitly skipped.
- Native `scrolling` passed at scales 1 and 2. Final metadata-only evidence is
  `target/validation/native/scrolling/scroll-evidence.log`; the unfocused pane moved
  0 → 5 → 7 → 0 while the focused pane stayed at zero. New sample output arrived
  before the explicit downward scroll. Modes were 163969; captured build was 0.20.0.
  These synthetic wheel events are not physical trackpad or live Codex proof.
- Native `folder-access` passed: one successful cached entry survived a real Unix
  PermissionDenied refresh, permissions were restored, Retry was exercised, the
  file stayed intact and its shell PID stayed live. See
  `target/validation/native/folder-access/access-evidence.log` and `recovered.png`.
  This is temporary-directory chmod revocation, not a macOS TCC-policy test.
  The existing 1,000-entry bound is now an explicit incomplete-listing error rather
  than silently returning a truncated successful listing. First-run dismissal,
  loaded inventory/hidden projects, forced same-path retry and stale results have
  focused unit coverage; existing picker-cancellation coverage still passes.
- Native `idle-close` passed: the real pane button closed a verified idle zsh pane
  without confirmation while another pane remained blocked in a shell builtin.
  Native `agent-sidebar`, `markdown`, `launch`, and `smoke --sessions 50 --seconds 5`
  also passed. Captures live under `target/validation/native/`.
- Background screenshot fixtures previously raised always-on-top windows while
  filtering physical input, which could make scrolling in an obscured working
  window appear stuck. They now start inactive, behind ordinary windows, with
  mouse pass-through for synthetic input. Scrolling and later native captures
  passed with that behavior. No fixture processes remained in the process check.

### Packaging and launch artifacts

- Two consecutive plain `cargo xtask local-dmg` builds succeeded. Build/package/DMG
  seconds were 1.63/0.90/6.65 and 0.18/0.82/5.64; the second reused compilation.
  The final plain artifact is `target/local-dmg/build-eF5Tbk/Terminator.dmg`.
- Styled packaging used an existing temporary create-dmg 1.3.0 checkout, without
  system installation: build/package/DMG 0.18/0.83/25.24 seconds. Artifact:
  `target/local-dmg/build-15rSgH/Terminator.dmg`. Missing create-dmg reports an explicit
  prerequisite error. Earlier plain and styled builds also used an output path
  containing spaces under `/private/tmp/terminator local dmg/`.
- Both final images passed hdiutil verification and were mounted read-only for
  inspection, then detached. Each contained the Applications symlink, all three
  executable components with 0755 permissions, resources/licenses, privacy usage
  descriptions and a valid ad-hoc signature. The styled image contained its Finder
  layout and background. `target/validation/local-dmg-inspection.json` records this.
- An intentionally invalid build number failed packaging with `--timings` enabled.
  Temporary output directories were removed, previous artifact hashes were
  unchanged and a Cargo HTML timing report was produced. No installation occurred.
- `docs/MAC_APP_STORE_FEASIBILITY.md` separates current Apple requirements, source
  gaps and review questions; its roadmap is a proposal, not a submitted prototype.
- `launch/product-hunt/` contains a 50-character tagline, 227-character description,
  maker comment, three suggested topics, FAQ/checklist, 240×240 thumbnail and four
  1270×760 gallery images. Native sample projects are atlas-web/atlas-api. The
  attention event is explicitly synthetic; no agent provider was used. Source and
  download links returned HTTP 200, with latest download resolving to v0.20.0.
  Copy/dimensions/provenance are in `launch/product-hunt/validation.json`.

### Runtime gates still open

Installed Codex is 0.154.0; its help and the current official CLI documentation
confirm `--no-alt-screen` as a per-launch override. No user Codex configuration was
changed. Actual retained Codex conversation navigation in both modes remains open:
repository instructions keep agent launches user-driven, and synthetic tests alone
do not close that issue. Record GUI build, focus, modes, offsets and delivery counts
without private message text when performing that check.

Physical trackpad momentum/overlays, real TCC revocation/reselection and separate
GUI-versus-shell/editor access, fish runtime, and Linux runtime remain unverified.
The launch kit is prepared for review; posting and claims about the downloadable
release's installation/signing still require the posting checklist. A rebuilt GUI
or DMG has not replaced the running daemon; old daemons retain close confirmation.

## 2026-09-15: User-approved live Codex history verification

This follow-up supersedes the unverified live-Codex gate above. The user explicitly
approved launching disposable Codex sessions. Tests used only fresh conversations
requesting numbered public TSAMPLE/TUPDATE lines, with tools forbidden by the prompt,
read-only sandboxing and hooks disabled for those invocations. No stored conversation
was resumed. The saved configuration predates the test fixture and contains no new
temporary trust entries. Disposable projects were under the already-trusted repository's
ignored target directory; saved trust configuration was not edited.

Installed Codex 0.154.0 passed **both normal launch and `--no-alt-screen`** against the
current 0.20.0 GUI/daemon build:

- A focused pane reached earlier transcript lines and returned to line 160.
- An unfocused, hovered pane reached earlier lines while typing focus stayed in
  the other pane, then returned to recent output.
- Lines 104–135 remained unchanged in the visible history while the second real
  model response streamed; later downward scrolling reached update line 100.
- Closing/reopening the GUI preserved the daemon-owned sessions. Scrolling after
  reconnect reached the earlier first conversation, not only the latest update.
- The routing mode mask is now identical before/after reconnect (166033 in both
  cases). Both tested invocations used inline terminal history in this environment;
  application-mouse and alternate-screen routing retain separate unit coverage.

Evidence: `target/validation/native/codex-live/summary.json`, `normal-evidence.json`,
`no-alt-screen-evidence.json`, and the corresponding `*-focused-evidence.json` files.
They contain mode/focus/offset metadata and public line numbers, not private message
bodies. Screenshots and logs in that ignored directory belong only to these public
fixtures. Physical trackpad input, TCC behavior, fish and Linux remain separate gates.

### Fix found by the live check

The daemon's vt100 snapshot did not preserve DEC focus-reporting mode 1004, and did
not track explicit alternate-scroll mode 1007. Runtime callbacks now retain these
bounded mode settings and append them to attachment snapshots. The same parser is
fed in order around terminal resets so RIS clears extension state, including across
byte chunks. Regression tests cover set/reset, combined parameters, mode queries,
replay and terminal reset ordering. Live reconnect subsequently retained mode 166033
instead of losing focus reporting and falling to 163985.

Long-running background fixtures also exposed eframe's normal occlusion behavior:
logic and IPC continue, but covered windows stop UI rendering. A compile-time
`test-support` extension plus explicit fixture environment flags now enables UI
passes while occluded. Test windows remain inactive, behind normal windows and
click-through. Normal packaging does not enable that feature. Control connections
also preserve the GUI's PTY dimensions, and Codex prompt submission uses bracketed
paste followed by a separate Enter event.

Validation after the parser fix: **196 workspace tests passed** (133 app, 28 core,
21 daemon, 5 hook, 6 integrations, 3 xtask); strict Clippy and formatting passed.
The full real-PTY integration suite also passed, including 6 MiB history bursts,
mode/screen snapshots, stalled attachments, helper lifetime and shutdown races.

### Final post-live builds

The native scrolling (scales 1/2), idle-pane close, folder-access recovery and
50-session smoke fixtures all passed again with occluded rendering enabled for
background fixtures. Final formatting and strict Clippy checks passed.

Rebuilt artifacts containing the reconnect fix:

- Plain: `target/local-dmg/build-vbR38L/Terminator.dmg`
  (build/package/DMG: 3.22/0.96/17.97 seconds).
- Styled: `target/local-dmg/build-DZR4Je/Terminator.dmg`
  (build/package/DMG: 0.17/0.87/24.27 seconds).

Both passed image/signature verification and a fresh read-only mounted-content
inspection; mounts were detached afterward. Inspection evidence is
`target/validation/local-dmg-live-verified.json`. A final process check found no
fixture GUIs/daemons; the original installed Terminator GUI and daemon remained
running with their original PIDs. No installation or live-daemon replacement was
performed.

## Restart review fixes (2026-09-15)

Restart commands pin the original appearance configuration directory before
setting the service data directory. Detached helper stderr is drained to
`restart.log`; EOF notifies a surviving GUI to clear its restart guard and
report the failed attempt. Unrelated worker errors do not unlock a pending restart.
Focused GUI restart/installation and hook shutdown tests passed, including an
actual detached failing helper and its preserved config environment. Native
window restart behavior was not re-run for these fixes.

## CI inbox follow-up (2026-09-15)

The Agents inbox now lists in-scope terminal notices with navigation and
capability-gated dismissal, excludes resolved agent notices, and opens resolved
notice details outside the inline inbox. All 251 workspace tests passed with
all features, including the three reported CI regressions. Workspace Clippy,
formatting and diff checks passed. Native desktop rendering and hosted CI were
not rerun.


## Attention, diff selection, and restart recovery (2026-09-15)

- Replaced the expanded attention strip with a bell and pending count. Resolved,
  dismissed, and snoozed agent notices do not contribute to its count. Clicking
  opens a bounded inbox popup. Cards use rounded borders, selection backgrounds,
  and plain-text Markdown previews with visual ellipsis instead of cutting words.
- Git menus expose Native and Neovim reviews independently of the default viewer.
  Explicit Neovim selection retains capability-gated native fallback on older
  services. The native viewer labels its existing split mode “Side by side”.
- Reproduced failed restart in the disposable native installation fixture. The
  helper's stderr was a pipe drained by the exiting GUI; its post-exit status
  write could abort cleanup and leave an empty restart log. It now writes directly
  to disk, with a separate silent stdin socket used only to observe helper exit.
  No helper output depends on a reader in the old GUI. A subprocess regression
  verifies that the helper continues and logs after its parent GUI process exits.
- Restart results persist before relaunch. The reopened GUI reports cleanup errors
  and checks the new generation, daemon path/version, and helper health. Idle
  automatic repair also verifies the replacement. Restart preflight checks all
  three sibling executables. Existing live-session confirmation remains required.
- `cargo test --workspace --all-features --locked`: 256 passed outside the sandbox
  for disposable sockets, processes, PTYs, and watchers. The final application
  diff-routing change also passed all 188 application tests. Workspace build,
  all-target/all-feature Clippy with `-D warnings`, rustfmt, and diff checks passed.
- Native fixtures passed: `agent-sidebar`, `reviews`, `legacy-diff`, and
  `installation`. They verify collapsed/open bell state on Git, navigation and
  resolved counts, explicit Neovim review while Native is the default, native
  side-by-side rendering without allocating a PTY, no unsupported legacy request,
  visible restart errors, successful confirmed restart, and safe idle recovery.
  The sidebar fixture uses explicit manual dismissal to keep its notification
  pending through navigation before testing resolution.
- Inspected native screenshots: [collapsed bell](screenshots/sidebar-recovery-2026-09-15/bell-collapsed.png),
  [popup and cards](screenshots/sidebar-recovery-2026-09-15/bell-popup.png),
  [side-by-side diff](screenshots/sidebar-recovery-2026-09-15/side-by-side.png), and
  [restart error](screenshots/sidebar-recovery-2026-09-15/restart-error.png).
- Validation used disposable state. Existing user edits were preserved; the live
  installation and its daemon sessions were not replaced, restarted, or stopped.


## Pending-change review fixes — 2026-09-15

- Palette and worktree dialogs now suspend terminal input as well as app shortcuts.
  A regression checks that input is restored after each dialog is dismissed.
- Shortcut regressions cover captured function/navigation keys, macOS Control-only
  event consumption, and modifier-order independence for Command+Control.
- An egui pointer-event regression selects Custom, verifies the existing executable
  remains editable, then selects a detected executable and verifies Custom exits.
- Passed `cargo test -p terminator --bin terminator --all-features --locked`
  (212 tests), `cargo clippy -p terminator --all-targets --all-features --locked -- -D warnings`,
  `cargo fmt --all --check`, and `git diff --check`. Tests ran outside the sandbox
  to permit isolated Unix sockets and filesystem watchers.
- These are unit/headless egui checks, not native desktop or real-PTY validation.
  No live GUI or daemon was restarted, and existing pending changes were preserved.

## Follow-up review fixes — 2026-09-15

- Explicit empty shortcut bindings remain disabled; missing bindings still receive
  defaults. Palette file filtering precedes the 40-file result limit.
- Worktree creation now completes through its own worker response after an
  acknowledged add and inventory lookup. The worker canonicalizes the destination;
  failed requests cannot leave pending navigation or terminal creation behind.
- Five focused regressions cover empty bindings, searching beyond the first 40
  files, failed worktree requests, symlink destination lookup, and opening the
  requested project's terminal only when selected in the creation options.
- Passed all 217 application tests with all features outside the sandbox, permitting
  isolated sockets and filesystem watchers. Application all-target/all-feature
  Clippy with `-D warnings`, rustfmt, and `git diff --check` passed.
- These are unit/headless checks, not native GUI or real-daemon worktree validation.
  Existing pending edits were preserved; no live sessions were restarted.

## Compact agent notifications — 2026-09-16

- Agent cards use smaller status/session labels, neutral backgrounds, and arrow,
  moon (10-minute snooze), and dismiss icons with tooltips. Hovering the header
  shows the message and session directory. The sidebar header omits the Agents
  label and centers its bell and count in a compact 28-point row.
- Passed application all-target/all-feature check and Clippy with `-D warnings`,
  rustfmt, and `git diff --check`. Nine existing `click_` tests passed, including
  Go, Snooze, and Dismiss behavior and sidebar row click targets.
- Validation is static/headless; native appearance and hover placement have not
  been visually verified. No live GUI or daemon sessions were restarted.


## Seamless daemon generations — 2026-09-16

Implemented the shared catalog, scoped owner databases/history, authenticated
owner routing, candidate verification/activation, idle retirement, crash
reconciliation, cross-owner worktree safeguards, global history coordination,
and generation diagnostics/recovery. Appearance/navigation locations and
`AGENTS.md` are unchanged. Existing unrelated edits were preserved. All process
and migration checks below used disposable data and fixture processes; no live
user daemon or sessions were restarted.

Passed on local macOS:

- `cargo fmt --all --check`, workspace all-target/all-feature Clippy with
  `-D warnings`, the workspace binary/example test-support build, and
  `git diff --check`.
- `cargo test --workspace --all-features --locked --offline`: 308 tests passed;
  the opt-in real-PTY test is excluded from that count and was run separately.
  Catalog regressions cover ownership, active-only workspace writes, activation
  serialization, freeze rollback, unavailable-owner preservation, interrupted
  migration/retry, incompatible candidates, and explicit redirect versus
  uncertain-response creation handling.
- `cargo test -p terminator --all-features three_generations_preserve --locked
  --offline -- --ignored --nocapture`: passed. Three staged generations of the
  current build preserve shell/editor PIDs, a running job and an unsaved Neovim
  buffer. New sessions use the active owner. Removing the fixture installation
  leaves owner bridges/helpers usable. History, notices, editor save/close and
  input reach old owners. A broken candidate preserves the active generation;
  an unavailable owner keeps ownership; a crashed owner alone becomes interrupted.
  Draining owners retire and historical output remains readable.
- `cargo xtask integration`: passed PTY reconnect, hooks/notifications,
  deduplication, cwd routing, CLI controls, worktree safety, shutdown/relaunch,
  eight creation/idle-shutdown races, 6 MiB history backpressure without truncation,
  stable-helper replacement/removal, oversized snapshots, stalled attachments,
  and terminal-editor argument checks.
- `cargo xtask gui generations --output target/validation/seamless-upgrades-final`:
  passed real initial migration, two-owner diagnostics, unsaved Markdown preview
  from the draining editor, GUI quit/relaunch with original PIDs, the red warning
  counting all three sessions, cancellation, confirmed all-owner cleanup, and
  reopening without rerunning historical sessions. The unsaved file stayed
  unchanged on disk through cancellation and destructive cleanup.

Evidence is under `target/validation/seamless-upgrades-final/`: workspace,
integration and real-PTY logs, plus `generations/generations.json` and five native
captures (`first-migration`, `updated-service-ready`,
`all-owners-restart-warning`, `restart-cancelled`, `after-confirmed-recovery`).
The update-status and warning captures were visually inspected. Sandbox-denied
socket/watcher attempts were rerun successfully outside the sandbox using the
same isolated fixtures.

Boundaries: these fixtures stage multiple generations of this build, not three
released binary versions or a signed Sparkle installation. Linux native behavior,
signed cross-release rollout, prolonged load across many retained generations,
and real provider hook execution across upgrades remain unverified. PID reuse
is handled conservatively as unavailable rather than proof of death. Mixed-owner
idle-close batches retain the existing confirmation flow. Migration retains the
original database/history and its backup for recovery.


### Review fixes — legacy crashes, confirmation scope and failed startup

- A legacy daemon's held lock still blocks migration, including an unresponsive
  service. Once the exclusive legacy lock establishes that it exited, migration
  imports stale running records as interrupted without adopting, signalling or
  rerunning their PIDs. Session IDs/owner identities and original backup records
  are preserved. A new native fixture crashes only its disposable legacy daemon
  and verifies GUI startup/migration succeeds without restarting its shell.
- Restart confirmation captures the active generation and exact live-session IDs
  in the GUI job and passes them as structured JSON to the detached helper.
  Validation rejects added/replacement sessions or a changed generation before
  any GUI-close or stop request. Ended approved sessions are permitted. Legacy
  cleanup also checks its original inventory after the GUI checkpoint instead of
  silently broadening the target set. Regression tests cover the queued job,
  helper argument transport, same-count identity changes, and absence of any
  close/stop request after invalid confirmation. The native fixture creates a new
  session after approval, confirms CLI cleanup refuses, and checks all four PIDs.
- Candidate registration now has a guard before executable spawn. Failed spawn
  and child exit before storage initialization remove only a matching Prepared
  registration under the coordination/owner locks. Active owners and uncertain
  running candidates are preserved. Private failed-candidate files are removed;
  diagnostic logs remain under `failed-candidate-<id>.log`. Real-PTY coverage
  injects an invalid executable and an immediately exiting daemon and verifies
  the active generation, registry count and private-directory count are unchanged.

Passed: formatting, workspace all-target/all-feature Clippy with `-D warnings`,
workspace binary/example build, 315 workspace tests, the separately invoked real
three-generation PTY/Neovim test, `cargo xtask integration`, and both native
`generations` and `installation` suites. The installation suite also exercises
confirmed detached GUI restart and relaunch. Its legacy connection fixture now
keeps a live legacy shell so automatic idle migration cannot remove the socket
under test; the idle-repair fixture follows the automatic migration path.

Evidence: `target/validation/seamless-review-fixes/` contains the test/integration
logs and native captures/reports, including `generations/legacy-crash-recovered.png`
and `installation/restart-relaunch.json`. All tests used isolated processes and
state. Linux native and signed cross-release validation remain outside this local
run; no live user service was restarted and `AGENTS.md` was unchanged.
