# Rust development tasks

## Local release checks

`cargo xtask release` validates an isolated source copy with the
next version before it changes the checkout. It includes staged, unstaged,
deleted, and untracked files that Git does not ignore. Check the files before
running it: a successful release still commits all of them, as before.

The Rust command requires host checks, Linux Docker execution, and a Windows
runner unless a platform is explicitly skipped. The existing
`scripts/git-release.sh` wrapper keeps its default `--skip-windows` setting.
Missing required runners stop the flow. To validate all configured platforms
without committing or pushing:

```sh
export TERMINATOR_WINDOWS_HOST=your-windows-ssh-alias
cargo xtask release --check-only
```

For an explicit partial run on this Mac:

```sh
cargo xtask release --check-only --skip-linux --skip-windows
```

Remove `--check-only` to commit and release from `master`. The script checks
that local `master` contains remote `origin/master`, rejects existing version
tags, and stops if source files change during validation. Failed validation
does not change source files or the real staging area. On success, it checks
the staged and committed trees against the tested tree, pushes `master` and
the version tag atomically without force, and dispatches Release from that tag.
The repository commit hook skips its duplicate checks only when its staged
tree equals `TERMINATOR_RELEASE_VALIDATED_TREE`; other hooks still run.

Host checks include the shared compiler, formatting, Clippy, workspace tests,
audit, license/source checks, real-PTY integration, idle-close, `gui all`, and
a release package build. macOS also cross-checks Windows with Clippy; this
requires zig 0.14.x and the installed `x86_64-pc-windows-msvc` Rust target.
Neovim, `cargo-audit`, and `cargo-deny` must be installed locally. The local
release flow does not require Python.
Native GUI checks require a desktop and any permissions required by the fixtures.
Live-provider tests, ignored tests, soak tests, signing, and notarization are
outside this local gate.

Linux uses an Ubuntu 24.04 image, the repository Rust version, Neovim v0.11.6,
Xvfb/Openbox, software rendering, the same checks and package build, and CI's
50% line-coverage threshold. It uses the Docker server's native architecture:
`linux/arm64` on Apple Silicon, or `linux/amd64` on x64 hosts. Emulation changes
the shell executable reported by the kernel and cannot pass the strict
idle-close identity checks. ARM64 local tests do not replace CI's x64 tests.
Docker must be running. Images and Cargo build volumes are separate for each
architecture; registry and Git source caches are shared. The image imports
the installed toolchain from the official Rust image while keeping Ubuntu as
its runtime. Required components come from `rust-toolchain.toml`, with LLVM
tools added for coverage, so fresh containers do not need to install missing
components. Audit, deny, coverage, and Neovim use pinned upstream binaries
with SHA-256 checksums for each architecture; the three Cargo tools have
independent build stages. Bash, zsh, and fish are installed for idle-close tests.
An empty build context keeps source edits from invalidating setup layers.
The local image tag includes a SHA-256 digest of the Dockerfile, Rust version,
required components, and platform. An exact local match skips `docker build`
and Docker Hub authentication. Setup changes create a new tag and run the
build, reusing unchanged layers. Use `--rebuild-linux-image` to run the image
build explicitly. The first build needs registry access; an older image with
a different configuration is never used as a fallback. The run records its
Linux platform in `results.json`. Other CPU architectures and host-specific
GPU/desktop behavior still need their own validation.

During a registry outage, explicitly select an image that is already installed:
`./scripts/git-release.sh --linux-image YOUR_NATIVE_LINUX_IMAGE`.
This overrides the Dockerfile configuration check; the release checks still
run inside the selected image. Missing images or an architecture mismatch stop
before host tests. The old x64 image cannot be used on Apple Silicon. Containers
use `--pull never`. This option conflicts with `--rebuild-linux-image`.

The built-in Windows runner uses `ssh` and `scp` with existing SSH key access.
Configure the Windows OpenSSH server and install Git Bash, Rust 1.97.1 with
rustfmt/Clippy, Neovim, `cargo-audit`, and `cargo-deny`. The remote SSH user
must be able to run these tools. `TERMINATOR_WINDOWS_BASH` can override
`C:/Program Files/Git/bin/bash.exe`. The runner copies the exact source,
runs shared checks, workspace tests, and packaging, and downloads an artifact
archive even after a test failure. Remote source/log directories remain under
`%USERPROFILE%/.terminator-release/`; Cargo output is reused there. Windows
PTY and native GUI fixtures are still unsupported. An ARM Windows VM does not
replace the x64 Windows Server CI environment.

For another VM or runner tool, set `TERMINATOR_WINDOWS_RUNNER` to an executable.
It receives `SOURCE_DIRECTORY ARTIFACT_DIRECTORY`, must copy the source
unchanged, run `cargo xtask release-check` on Windows with an isolated
`RUNNER_TEMP`, collect artifacts, and return a nonzero status on any failure.

Logs, screenshots, packages, and `results.json` remain under
`.artifacts/releases/run-*`; skips are explicit in the summary. The release
file lock prevents two local flows from running together and is released by
the OS when the process exits, including a forced stop. The file remains on
disk; do not delete it. For an empty legacy lock directory from the older
implementation, first confirm no release process is running, then remove
that directory with `rmdir .artifacts/release.lock` once.
A commit or push failure leaves local files, commits, and tags for inspection.
If dispatch fails after a successful push, retry only the printed
`gh workflow run release.yaml --ref vVERSION` command; do not bump again.
GitHub Actions retains its final checks and hosted security/signing steps.

The release commands are implemented in `crates/xtask/src/release/`:

- `cargo xtask release`: validate, commit, push, and dispatch.
- `cargo xtask release-check --output DIRECTORY`: check this operating system;
  `RUNNER_TEMP` can provide the output directory instead.
- `cargo xtask release-windows SOURCE ARTIFACTS --host HOST`: run the Windows
  SSH checks. `TERMINATOR_WINDOWS_HOST` can provide the host instead.
- `cargo xtask bump-version`: bump the workspace minor version and local
  lockfile entries without resolving dependencies.

The matching shell scripts remain wrappers. Offline Rust regressions:
`cargo test -p xtask --locked release::`. They use disposable Git repositories
and local bare remotes; they do not publish releases.

## Quality checks

Run `sh scripts/check.sh` for formatting, compiler checks (`lint`), build,
Clippy with warnings denied, `cargo audit`, and `cargo deny` (bans, licenses,
sources). Run `sh scripts/check.sh test` for workspace tests. Individual checks
accept `fmt`, `lint`, `build`, `clippy`, `audit`, or `deny`. `windows-check`
and `windows-clippy` cross-check `x86_64-pc-windows-msvc` from macOS/Linux via
zig 0.14.x (`TERMINATOR_ZIG` when not on PATH; check-only, linking stays on
windows-2025 CI). Checks use the
lockfile and do not rewrite files. Pre-commit uses the same `all` path; install
`cargo-audit` and `cargo-deny` (`cargo install cargo-audit cargo-deny --locked`).
`cargo deny check advisories` is CI-only (same RustSec DB as `cargo audit`).

[cargo-husky](https://github.com/rhysd/cargo-husky) installs the tracked
`.cargo-husky/hooks/pre-commit` when its development dependency is first built
(for example, `cargo test -p terminator --no-run --locked`). The hook runs
`sh scripts/check.sh` against the working tree; stage any fixes before committing.
Existing non-cargo-husky hooks are preserved. CI skips hook installation and runs
all checks plus tests on native macOS, Linux, and Windows through the reusable
CI workflow called by Release. Native `focus-editor-close` and `file-close`
fixtures must pass on macOS and Linux before release builds. The CI wrapper
(`bash scripts/native-gui-ci.sh CASE`) keeps logs and screenshots under
`$RUNNER_TEMP/terminator-native-gui`; CI uploads them even on failure. Linux uses
Xvfb and Openbox, and both runners explicitly select Glow. Local checks, CI, and releases use Rust 1.97.1, pinned locally by
`rust-toolchain.toml` with rustfmt and Clippy.

## Renderer comparison

Wgpu is the default renderer on macOS, Linux and Windows. Glow stays available
in Wgpu builds. `cargo build -p terminator --no-default-features` builds a GUI
that uses Glow without Wgpu.
`TERMINATOR_RENDERER=glow` selects the recovery renderer; `wgpu` selects Wgpu
when compiled. An invalid or unavailable override fails before starting the
daemon. There is no automatic renderer retry.

Compare both renderers with the same release executable:

```sh
cargo build --release --workspace --bins --examples --features terminator/test-support --locked
TERMINATOR_FIXTURE_RENDERER=glow TERMINATOR_TEST_BIN_DIR="$PWD/target/release" target/release/xtask gui renderer-perf --seconds 15 --output target/validation/renderer-glow
TERMINATOR_FIXTURE_RENDERER=wgpu TERMINATOR_TEST_BIN_DIR="$PWD/target/release" target/release/xtask gui renderer-perf --seconds 15 --output target/validation/renderer-wgpu
```

Run measurements serially after builds finish. Each command uses disposable
state and six visible shells, with three idle trials and three output trials.
Output is one line per shell approximately every 33 ms. Each trial warms up for
five seconds, then samples GUI cumulative CPU time and RSS every 500 ms. CPU
percentage uses one core as 100%. Screenshot polling stops after three seconds;
the final capture occurs after measurement. Captures and renderer logs verify
that the native GUI ran. `OUTPUT/renderer-perf/renderer-perf.json` contains raw trial results. Compare
medians and repeat in reverse order to check order effects.

These results do not measure GPU time, power use, startup, or input delay. The
fixture uses visible, inactive windows with mouse passthrough and a fixed scale.
Keep the screen unlocked during the test. Wgpu captures can time out behind
another window on macOS. This is not a measurement of the installed application.

## Dependency security

`.github/workflows/security.yaml` runs on master pushes, pull requests, Monday
07:17 UTC, and manual dispatch. It fails the job on known lockfile
vulnerabilities (`cargo audit`, `cargo deny`, OSV-Scanner, GitHub dependency
review). Snyk, Socket, and OWASP Dependency-Check skip unless their secrets
are set. OWASP 13 needs `NVD_API_KEY`; an empty key aborts the NVD update.

Local equivalents:

```sh
cargo audit
cargo deny check --all-features
osv-scanner --config osv-scanner.toml -r .
```

Keep `.cargo/audit.toml`, `deny.toml`, and `osv-scanner.toml` advisory ignores
in sync.

Dependabot owns Cargo version PRs (`.github/dependabot.yml`). Renovate owns
GitHub Actions PRs (`renovate.json`); install the
[Mend Renovate GitHub App](https://github.com/apps/renovate). Enable Dependabot
alerts and security updates in the repository Code security settings.

Optional secrets: `SNYK_TOKEN`, `SOCKET_SECURITY_API_KEY`, `NVD_API_KEY`.
OpenSSF Scorecard is `.github/workflows/scorecard.yaml` (master + weekly).


Project-owned Python automation has moved to `crates/xtask`. Run commands from
`terminator/`; no Python interpreter is used by these tasks.

| Former script | Rust command |
| --- | --- |
| `package.py` | `cargo xtask package [--debug] [--timings]` |
| Styled macOS disk image | `cargo xtask dmg --app PATH/Terminator.app --output PATH/Terminator.dmg` |
| `integration.py` | `cargo xtask integration` |
| `gui_smoke.py` | `cargo xtask gui smoke --sessions 50 --seconds 5` |
| `workspace_tabs_smoke.py` | `cargo xtask gui workspace-tabs` |
| `editor_lifecycle_smoke.py` | `cargo xtask gui editor-lifecycle` |
| `file_close_smoke.py` | `cargo xtask gui file-close` |
| `inline_rename_smoke.py` | `cargo xtask gui inline-rename` |
| `focus_editor_close_smoke.py` | `cargo xtask gui focus-editor-close` |
| `ui_cleanup_smoke.py` | `cargo xtask gui ui-cleanup` |
| `external_editor_smoke.py` | `cargo xtask gui external-editor` |
| `review_smoke.py --gui` | `cargo xtask gui reviews` |
| `legacy_diff_smoke.py` | `cargo xtask gui legacy-diff` |
| `ui_flat_smoke.py`, `ui_plan3_smoke.py` | `cargo xtask gui terminal-actions` and `cargo xtask gui ui-cleanup` |
| `integration.py --load-seconds N` | `cargo xtask load --seconds N --output report.json` |
| `compare_load.py --conditional` | `cargo xtask load --conditional --seconds 30 --output report.json` |
| before/after load reports | `cargo xtask compare-load before.json after.json` |
| `command_counts.py` | `cargo xtask command-counts --seconds 6 --output counts.json` |
| `muse_echo.py --muse PATH` | `cargo xtask muse-echo --muse PATH` |

New cases: `cargo xtask gui images`, `cargo xtask gui browser`, `cargo xtask gui control`,
`cargo xtask gui window-controls`, `cargo xtask gui split-file-opening`,
`cargo xtask gui markdown`, `cargo xtask gui markdown-busy`, and `cargo xtask browser-check`.
`cargo xtask gui project-sidebar` covers project sort, removal and restoration.
`cargo xtask gui agent-sidebar` covers the bell, waiting/unread counts, left Agents navigation, restart persistence, and live status updates.
`cargo xtask gui all` runs the ordinary native fixture suite. All GUI cases accept
`--scale 1|2`, `--narrow`, and `--output PATH`. Captures default to the Cargo target
validation directory; earlier committed screenshots are not overwritten.

`cargo xtask gui hover-menu` checks that adjacent terminal file paths cannot
replace an open menu. It checks Escape, X, outside-click dismissal and opening
the original file while preserving the shell PID. Fixture actions accept
`hover_offset: [x, y]` for pointer coordinates relative to the target rectangle.

`split-file-opening` reproduces unavailable working directories after a project
move, checks the error and unchanged session inventory, then restores a path
alias. Real native menu clicks verify all four split directions, editor tab
placement, double-click deduplication, and normal/split file-menu actions while
preserving the original shell PID. It runs as part of `gui all`.

`markdown` opens a real Neovim file and exercises Edit/Preview/Split, unsaved
typing, preview focus, per-file buffer identity, GUI restart, and dirty-file
cancel/save-close behavior. It checks editor/shell PIDs and disk bytes, and runs
as part of `gui all`. Use `--scale 2 --narrow` for the compact Retina layout.
`markdown-busy` opens Neovim at a real pager prompt, verifies that Preview still
renders the saved file, then checks that live rendering resumes on the same PID.
The regular Markdown fixture also tests unsaved text through a prompt and Refresh,
Preview on the initial file click, and persistence of an explicit Edit choice.
It checks that filename, view tabs and refresh icon share one row without overlap.
`project-sidebar` sorts visible projects by name and latest activity (clicking a
visible project does not reorder it), then
removes/restores active projects and tests an empty sidebar
across GUI restarts while retaining the same shell/editor PIDs, unsaved buffers,
file bytes and project layouts. Both cases run as part of `gui all`.

Build first with Rust 1.97.1+ and a C compiler. Editor/review fixtures require
Neovim 0.10+ on PATH, Git, and a native desktop. Run GUI cases serially:

`cargo xtask integration` also covers worktree use across projects and descendant
processes, large notification snapshots and restart recovery, stalled attachment
cleanup with original-PID reconnect, and terminal-editor argument fixtures.


```sh
cargo build --workspace --bins --examples --features terminator/test-support --locked
cargo xtask gui all
cargo xtask gui ui-cleanup --scale 2
```

The native window/picker test checks macOS event-posting permission before doing
anything, or requires `TERMINATOR_X11_TEST=1` in an isolated Xvfb/Openbox display.
It targets the fixture GUI; macOS native mouse gestures are bounded to its verified
foreground window and use a non-executing, non-echoing PTY. In ordinary
macOS fixtures, the GUI starts inactive and desktop input is filtered out.

`cargo xtask linux-check --browser --wayland` is restricted to a disposable Linux
container. It installs test dependencies, including an external test browser,
and uses Xvfb/Openbox and headless Weston. The application package does not bundle
that browser. The container source can be mounted read-only; use `CARGO_TARGET_DIR`
and `CARGO_HOME` for writable caches, and Docker `--init` for correct child signals.

`TERMINATOR_TEST_BIN_DIR` optionally chooses a separate binary directory for
before/after measurements. Provider echo testing is opt-in and requires a supplied
Muse executable. Historical validation reports retain their original command names.

`cargo xtask gui updates` checks GUI executable replacement with multiple projects,
splits, hidden sessions, previews, an unsaved editor, continuous output/input, hooks
and unchanged daemon/session PIDs. On macOS it also exercises native Quit. Set
`TERMINATOR_TEST_SPARKLE_FRAMEWORK` to the pinned framework to verify native updater
loading in an isolated fixture bundle. See `docs/UPDATES.md` for signed rollout
checks; this local fixture is not a signed Sparkle installation.

`cargo xtask gui installation` checks the native installation recovery screen,
disabled repair with live sessions, successful repair after the last session
finishes, cancel of Restart session service, and confirm that stops an unsaved
editor and relaunches this installation. It uses only disposable data and executables. `cargo xtask integration`
also verifies that removing the original installation leaves the daemon's private
helper executable, original session identities and new terminal creation intact.

### Local launch preparation

- `cargo xtask local-dmg [--release] [--styled] [--output DIR] [--timings]`
  builds the current Mac architecture and creates a unique directory beneath the
  output root (default `target/local-dmg`). Debug is the default. Plain compressed
  DMGs use `hdiutil`; styled DMGs require `create-dmg` and a desktop. Both include
  the existing ad-hoc-signed app and Applications shortcut. Builds reuse Cargo's
  target directory and never install or restart the live daemon.
- `cargo xtask idle-close` exercises process-group idle close against isolated real shells.
- `cargo xtask gui scrolling` exercises unfocused hover scrolling with sample history.
  Input fixtures support `wheel_unit` (`point`, `line`, `page`), `wheel_phase`
  (`start`, `move`, `end`, `cancel`), `scroll`, `scroll_x`, `shift`, and hover-only actions.
- `cargo xtask gui agent-wheel` checks wheel input reaches a full-screen SGR-mouse
  agent as mouse reports (covers the reattach snapshot path); `agent-wheel-legacy`
  covers legacy non-SGR reports and `agent-wheel-live` covers an agent that starts
  while the GUI is attached.
- After a test-support build, `cargo xtask gui launch` captures public sample scenes;
  `cargo xtask launch-assets` composes `launch/product-hunt/` images and checks copy
  lengths and dimensions. No provider is launched and nothing is posted.

`cargo xtask gui codex-live` is an explicit, account-backed live test: it starts new
Codex conversations containing public numbered lines. It is excluded from `gui all`
and must only be run with user authorization. `TERMINATOR_TEST_CODEX_FOCUS_ONLY=1`
limits it to focused/hovered navigation. Both normal and `--no-alt-screen` launches
are checked. The fixture does not resume stored account conversations or edit saved
Codex configuration. JSON evidence contains line numbers and terminal metadata.

`cargo xtask gui generations` validates initial idle migration and the native
multi-generation status, unsaved Markdown preview, GUI relaunch, all-owner restart
warning, cancellation and confirmed cleanup. The real PTY upgrade fixture is
`cargo test -p terminator --all-features three_generations_preserve --locked -- --ignored --nocapture`
after building the workspace binaries; it requires Neovim.

## Responsive services validation

```sh
cargo xtask async-boundary
cargo test -p terminator-core --features async-client --locked async_ -- --test-threads=1
cargo test -p terminator --all-features --locked stalled_radio_git_and_neovim -- --test-threads=1
cargo xtask gui responsiveness --seconds 15
```

The native responsiveness fixture uses isolated daemons/state, loopback radio and
Neovim servers that withhold responses, and a stalled mock Git child. It checks
100 rapid switches, unrelated focus acknowledgments, operation/pipeline limits,
Stop cleanup, and a 100 ms acknowledgment p95 / 250 ms result-processing maximum.
It records CPU, RSS, threads, descriptors and queue/worker counts in
`responsiveness.json`; CPU percentages are observations, not portable assertions.
`--seconds 1800` continues switching during a 30-minute resource soak.

For release acceptance, build all sibling binaries and run the matching driver:

```sh
CARGO_HUSKY_DONT_INSTALL_HOOKS=1 cargo build --release --workspace --bins --examples --features terminator/test-support --locked
TERMINATOR_TEST_BIN_DIR="$PWD/target/release" target/release/xtask gui responsiveness --seconds 1800 --output target/validation/async-services-release
```

The separately ignored live check opens the real default audio device muted,
requires PCM from 103FM within 20 seconds, then checks Stop cleanup:

```sh
cargo test -p terminator --all-features --locked live_103fm -- --ignored --nocapture --test-threads=1
```

All harness launches strip inherited `TERMINATOR_*` variables before applying
fixture-owned data/runtime/config and test settings. This includes catalog and
generation routing; setting only a temporary data directory is insufficient.
See `docs/VALIDATION.md` for measured results and remaining validation boundaries.
