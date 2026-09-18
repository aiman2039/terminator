# Rust development tasks

## Quality checks

Run `sh scripts/check.sh` for formatting, compiler checks (`lint`), build, and
Clippy with warnings denied across all workspace targets and features. Run
`sh scripts/check.sh test` for workspace tests. Individual checks accept `fmt`,
`lint`, `build`, or `clippy`. Checks use the lockfile and do not rewrite files.

[cargo-husky](https://github.com/rhysd/cargo-husky) installs the tracked
`.cargo-husky/hooks/pre-commit` when its development dependency is first built
(for example, `cargo test -p terminator --no-run --locked`). The hook runs
`sh scripts/check.sh` against the working tree; stage any fixes before committing.
Existing non-cargo-husky hooks are preserved. CI skips hook installation and runs
all checks plus tests on native macOS and Linux for pushes and pull requests to
`master`, or via manual dispatch. Local checks, CI, and releases use Rust 1.97.1, pinned locally by
`rust-toolchain.toml` with rustfmt and Clippy.

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
