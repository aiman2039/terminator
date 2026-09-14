# Validation evidence — 2026-09-08

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
  title/body notifications. The GUI still uses its Alacritty-backed widget.
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
