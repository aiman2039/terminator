# macOS releases and session continuity

The release workflow builds the Apple Silicon app in parallel with Linux. It then
runs `xtask assemble` using the tooling binary uploaded by the macOS job to embed
Sparkle without rebuilding the executables. macOS assembly waits only for that
Apple Silicon job. Its package output lives under `RUNNER_TEMP`, outside the
native jobs' Cargo caches. The same prebuilt tool creates the DMG, so assembly
requires no Rust installation or compiler cache. The daemon build verifies the
architecture of its embedded CodeDiff library before embedding it. Assembly
checks every Mach-O executable in the app and Sparkle framework for `arm64`,
preserves symlinks, and includes Sparkle's license. The pinned Sparkle framework
may still contain an unused Intel slice. The minimum macOS version remains 12
and the bundle identifier remains `dev.terminator.app`.

Sparkle is pinned to 2.10.0, archive SHA-256
`c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c`.
The workflow verifies that checksum before extraction. The public key is provided
by the repository variable `SPARKLE_PUBLIC_ED_KEY`; packaging rejects missing or
malformed keys. `CFBundleVersion` is the positive Release workflow run number,
which must exceed the latest published appcast's build number. The Cargo version
remains the display version. The first migration compares against the previous
fixed build number, 1.

The workflow signs nested executable files and bundles before the containing app,
notarizes/staples the app, creates a DMG with fixed icon positions, an Applications shortcut and installation instructions, signs and
notarizes/staples that DMG, then generates the signed appcast from those final
bytes. Deltas are disabled. The feed is:

`https://github.com/aiman2039/terminator/releases/latest/download/appcast.xml`

Enclosures use immutable `releases/download/vVERSION/` URLs. A draft receives the
macOS DMG, both Linux archives, appcast, and SHA256SUMS before publication.
Published assets cannot be replaced by a rerun; bump the version instead.
Publication requires `SIGNED_UPDATE_VALIDATED=true` and the
`production-release` GitHub environment. Keep that variable unset until the
signed-update matrix below has passed. Configure the environment's reviewers
according to the repository's production approval policy.

## Signing-key setup (operator only)

Use the verified Sparkle archive's `bin/generate_keys` on your own Mac to create
an Ed25519 key. Copy only the public key to `SPARKLE_PUBLIC_ED_KEY`. To provision
Actions, export the private key with `generate_keys -x PRIVATE_FILE` and set the
`SPARKLE_PRIVATE_ED_KEY` Actions secret from that file using GitHub's secret UI or
`gh secret set SPARKLE_PRIVATE_ED_KEY < PRIVATE_FILE`. Never paste private key
material into a conversation, commit it, or print it in a workflow. Protect the
export and retain an offline backup under your signing-key policy. Existing Apple
Developer ID and notarization secrets are still required. No key is generated or
uploaded by packaging.

## Native updates and exit

Sparkle's `SPUStandardUpdaterController` is loaded from the app's own framework
bundle on the main thread. The application menu always has **Check for Updates…**,
including development launches that cannot load Sparkle; those explain why instead
of querying the production feed. Settings has **Updates**. The GUI probes
immediately once Sparkle reports it can check, then every 60 seconds using
`checkForUpdateInformation`, following AppDock's app-owned schedule. Automatic
checks default on when unset. Sparkle is not asked for update-check permission.
The existing automatic-check choice is migrated once to
`TerminatorAutomaticUpdateChecks` before disabling Sparkle's separate timer.
Polling pauses during GUI exit and active Sparkle sessions, with no catch-up burst
after sleep. A successful probe hands each new version to Sparkle's background
update flow once per GUI launch; manual checks remain available after a failure
or deferral. Background downloading retains its existing Sparkle preference.
Native Later, Skip, Install and
Relaunch, and install-on-quit behavior remain under Sparkle's control.
`SUShowReleaseNotes=false` disables embedded release notes; Settings links to
GitHub. Signed-feed verification and verification before extraction are enabled.

Window close, native Quit, and Sparkle termination share an asynchronous exit
checkpoint. The GUI suspends actions, waits for an existing picker, drains its
worker and applies the results, and repeats draining if those results enqueue
follow-up work. It then serializes final layouts and writes selection, focus and
preferences, requiring successful RPC and filesystem acknowledgments. Unknown
layout versions and unreadable preference formats remain read-only. Failure or a
15-second deadline cancels that attempt and restores interaction with a retryable
error. Attempt IDs reject delayed acknowledgments from cancelled attempts.

The existing winit application delegate stays installed. The macOS bridge
intercepts `NSApplication.terminate:` before AppKit begins its termination loop.
After persistence, it invokes the saved original implementation on a subsequent
main-queue callback, outside winit's event handler. This avoids both AppKit's
modal deferred-quit loop (which prevents GUI checkpoint progress) and synchronous
termination notifications re-entering winit. Native installation cancellation
restores GUI interaction. Review this bridge when upgrading winit or Sparkle.

GUI updates do not stop existing session owners or relaunch their shells,
agents, or editors. Services advertising `daemon-generations-v1` can coexist:
the installed daemon/helper are staged privately, authenticated and checked before
atomic activation. New sessions use the active generation; existing sessions keep
their original PTYs, PIDs and unsaved buffers. Idle draining owners retire without
requiring an open GUI. Failed preparation preserves the previous owner and offers
a retry in Settings. Unknown/newer or incompatible storage/protocol versions are
not automatically downgraded or migrated past live-owner compatibility.

The initial transition from a legacy service waits for its sessions to finish.
An acknowledged `shutdown-if-idle-v1` shutdown and exclusive legacy lock precede
backup/import and atomic catalog publication. Live legacy services are preserved.
If the legacy service has already exited, stale live records are imported as
interrupted under its exclusive lock; no historical process is restarted.
The legacy database version guard and shared legacy locks prevent older binaries
from starting a competing service after migration.

Settings shows **Updated service ready**, earlier owners' live-session counts,
and navigation to those sessions. **Stop all sessions and restart** is separate,
explicit destructive recovery. Its confirmation counts all owners and pins their
exact session IDs and active generation. A newly live session invalidates that
approval before the helper closes the GUI or stops anything. The helper
freezes creation, checkpoints the GUI and stops the confirmed inventory without
force-kill. Unavailability is not evidence of process death, and partial recovery
failure is reported. See `docs/ARCHITECTURE.md` for storage and routing details.

## Fixtures and production rollout

Unbundled, debug, Linux, and explicitly isolated launches never use the production
feed. Test-support builds allow an isolated bundle with identifier
`dev.terminator.update-fixture`, its own baked-in feed/public key, and
`TERMINATOR_UPDATE_FIXTURE=1`. Environment variables cannot redirect the production
bundle's feed. Sign fixture bundles with this separate identifier and separate
fixture keys before exercising real installation.

After building workspace binaries/examples with `terminator/test-support`, run:

```sh
cargo xtask gui updates --output /tmp/terminator-update-continuity
```

On macOS, optionally set `TERMINATOR_TEST_SPARKLE_FRAMEWORK` to the extracted pinned
`Sparkle.framework`. This loads Sparkle in the isolated fixture with an unreachable
loopback HTTPS feed and a dummy public key; it cannot install an update. The
fixture replaces GUI/helper executable copies, reconnects to the same daemon,
checks multiple projects, splits, hidden sessions, image/Markdown tabs, an unsaved
Neovim buffer, identities/focus, continuing input/output, hook delivery and new
session creation. It exercises window close and native Quit and records captures
and `continuity.json`. This local replacement is not a signed Sparkle update.

Before enabling publication, exercise two genuinely Developer-ID-signed,
notarized fixture versions with increasing build numbers on Apple Silicon.
Retain the two DMGs, signed appcast, checksums, OS/architecture, daemon
generation, session/PID inventory, and before/after layout evidence. Cover:

- Install and Relaunch, and installation on normal quit without reopening.
- Later, Skip, and cancelled authorization/installation, with the GUI usable.
- Offline checks, invalid feed/archive signatures and interrupted downloads.
- Writable, read-only and translocated installations.
- Active output/input, unsaved editors, hidden sessions, restored split geometry
  and focus, hook delivery, and new session creation after replacement.
- Failed/timed-out persistence, duplicate restart requests and asynchronous
  creation completing during exit.

Record results in `docs/VALIDATION.md`. Existing installations need one manual
upgrade to the first Sparkle-enabled release; subsequent updates are in-app.

Primary references: [Sparkle setup](https://sparkle-project.org/documentation/),
[programmatic setup](https://sparkle-project.org/documentation/programmatic-setup/),
[updater delegate](https://sparkle-project.org/documentation/api-reference/Protocols/SPUUpdaterDelegate.html).

## Local installer validation

`cargo xtask dmg --app PATH/Terminator.app --output PATH/Terminator.dmg` uses
[create-dmg](https://github.com/create-dmg/create-dmg) (`brew install create-dmg`)
and requires a macOS desktop for Finder layout generation. The SVG background
source lives in `crates/xtask/assets/dmg-background.svg`; Rust renders it to PNG.
The task preserves the app's existing signature with `ditto`, checks all three
executables and refuses to overwrite an existing DMG. The release workflow signs
and notarizes the resulting image as before; it never skips Finder styling.

First launch from the disk image now presents installation instructions and exits
before creating state or starting a daemon. Copy the complete app to Applications,
eject the image, then launch that installed copy. Reopening can retire an idle
older/broken daemon through its advertised atomic shutdown capability. If any
sessions remain live, they are preserved and the status area explains recovery.
No helper is substituted into a live daemon and no privacy grants are changed.

## Stable helpers and recovery from older installations

New daemons keep a private copy of their own bundled helper in application data.
It stays executable if the app is moved, replaced or removed. Shell hooks and
GUI attachments use the advertised private copy, so an app update cannot change
the helper underneath a running session. Its bytes and signature remain intact;
no signing, quarantine removal or external download occurs at startup.

If an older daemon has already lost its original helper, **Fix installation…**
opens **Settings → Updates → Installation**. That screen lists every live session
and links back to it. Save editors and close sessions normally; backgrounded
sessions still count. **Repair installation** becomes available when no sessions
remain and the daemon advertises safe idle shutdown. It checkpoints the workspace,
rechecks generation and compatibility, requests atomic idle shutdown, starts the
installed daemon and verifies the private helper. No saved terminal is relaunched.
For older daemons without safe in-app restart, the screen shows a **Copy shutdown
command** button and numbered manual instructions. Finish all live sessions,
copy the command, fully quit Terminator (⌘Q on macOS), then run it in Terminal.app
or another terminal application. After `"Ok"`, wait two seconds and reopen
Terminator. The command targets this installation's helper, data and runtime
directories, including custom installations. The GUI only copies the command;
it does not execute legacy shutdown. Logout/login remains an alternative after
saving work and closing sessions. Unknown or newer versions are never retired
automatically.

When live sessions remain, **Restart session service** confirms and runs a
detached `ctl shutdown --stop-all --relaunch`. **Stop all sessions and shut down**
still offers a separate **Copy stop-all command** (now including `--relaunch`)
with an unsaved-work warning. The new helper's
`ctl shutdown --stop-all` requests normal GUI closure and waits for its checkpoint
and process exit before stopping sessions. It then uses the daemon's existing
Stop requests, waits for all sessions to end and requests idle shutdown (or the
legacy Shutdown request when needed). It waits for socket removal and daemon
lock release before returning, so a following `open /Applications/Terminator.app`
does not merely reactivate the old GUI. Run this command in another terminal app.
It does not save editor buffers or force-kill jobs that ignore normal termination;
timeouts report failure with the daemon retained. The new CLI is included only
after rebuilding/updating the helper, even when the running daemon is compatible.

`cargo xtask gui installation --output /tmp/terminator-helper-recovery` checks the
native recovery screen, live-session protection and successful idle repair.
`cargo xtask integration` also removes an isolated running daemon's entire source
installation, then verifies helper execution and new session creation with the
same daemon and original shell PID. These fixtures do not exercise Gatekeeper or
a signed Sparkle installation.

## Generation fixtures

Build the workspace binaries and test-support GUI first. Run:

```sh
cargo test -p terminator --all-features three_generations_preserve --locked -- --ignored --nocapture
cargo xtask gui generations --output target/validation/seamless-upgrades
```

The first fixture stages three generations of the current build and exercises
real shells, a running job, an unsaved Neovim buffer, candidate failure,
unavailable/crashed owners, installation removal and retirement. The native
fixture covers initial migration, generation diagnostics, an unsaved Markdown
preview, GUI relaunch, the all-owner warning, cancellation and confirmed cleanup. These are isolated
local fixtures, not a signed Sparkle rollout or cross-release compatibility proof.
