# macOS releases and session continuity

The release workflow builds native Apple Silicon and Intel slices in parallel
with Linux. It then runs `xtask universal` using the tooling binary uploaded by
the Apple Silicon job to assemble the three executables without rebuilding them.
macOS assembly waits only for the macOS matrix. Its package output lives under
`RUNNER_TEMP`, outside the native jobs' Cargo caches. The same prebuilt tool
creates the DMG, so assembly requires no Rust installation or compiler cache.
The daemon build verifies the architecture of its embedded CodeDiff library before
embedding it. Assembly checks every Mach-O executable in the app and Sparkle
framework for both `arm64` and `x86_64`, preserves symlinks, and includes Sparkle's
license. The minimum macOS version remains 12 and the bundle identifier remains
`dev.terminator.app`.

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

`https://github.com/ohaddahan/terminator/releases/latest/download/appcast.xml`

Enclosures use immutable `releases/download/vVERSION/` URLs. A draft receives the
universal DMG, both Linux archives, appcast, and SHA256SUMS before publication.
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
bundle on the main thread. The application menu has **Check for Updates…** and
Settings has **Updates**. Automatic daily checking and background downloading
are defaults; Sparkle stores the user's choices. Native Later, Skip, Install and
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

No GUI update stops the daemon, rotates authentication, replays commands, or
relaunches agents/editors. A later GUI launch may retire an older daemon, or a
same-version daemon with unavailable/unreported helper health or without
`stable-helper-v1`, only when it
advertises `shutdown-if-idle-v1` and has no live sessions. That request shares
the session-creation lock and persists before acknowledging shutdown; subsequent
creation requests fail. Daemons without that capability, unknown/newer versions,
and live daemons remain in place. RPC
failure with a held daemon lock is a connection error, never authority to replace
the daemon. Reboot/daemon-crash recovery is separate from GUI-update continuity.

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
notarized fixture versions with increasing build numbers on both Intel and Apple
Silicon. Retain the two DMGs, signed appcast, checksums, OS/architecture, daemon
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
[updater delegate](https://sparkle-project.org/documentation/api-reference/Protocols/SPUUpdaterDelegate.html),
[Apple universal binaries](https://developer.apple.com/documentation/apple-silicon/building-a-universal-macos-binary).

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
Daemons too old to advertise safe shutdown require a one-time OS logout/login
after saving work and closing sessions. Unknown or newer versions are preserved.

`cargo xtask gui installation --output /tmp/terminator-helper-recovery` checks the
native recovery screen, live-session protection and successful idle repair.
`cargo xtask integration` also removes an isolated running daemon's entire source
installation, then verifies helper execution and new session creation with the
same daemon and original shell PID. These fixtures do not exercise Gatekeeper or
a signed Sparkle installation.
