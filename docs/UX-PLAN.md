# UX, settings, and worktree plan

User-authorized follow-up to the Orca comparison. Steal information architecture
and control types, not Electron chrome. Keep daemon-owned PTYs, native egui, and
user-driven agent launches.

Sources: installed Orca 1.4.201, `stablyai/orca` renderer/settings/style guide,
local `orca-ui` (Tauri CLI chrome only), and this repo’s settings/worktree code.

## Goal

1. Make the GUI recede: quieter chrome, status as color/dots, actions on hover.
2. Stop asking people to type paths, key chords, and hex as the primary control.
3. Productize Git worktrees in the GUI so parallel agent work is a first-class
   loop, not a `terminator-hook ctl worktree` side path.

## Constraints

These are not optional. They come from `AGENTS.md`, `docs/ARCHITECTURE.md`, and
`docs/INTEGRATIONS.md`.

- Native `eframe`/`egui` only. No Electron, Chromium, CEF, OS webview, or Servo.
- The daemon owns PTYs, shells, and editors. Closing the GUI must leave sessions
  running. Do not restart live sessions to ship UI.
- Gate new daemon requests on advertised capabilities (`worktrees-v1`, and any
  new ones). A newly built GUI must keep working against an older running daemon.
- Git is the authority for worktrees. Refuse removal of dirty, locked, or
  live-session checkouts. Do not delete branches as a side effect of “close”.
- Do not auto-launch agents or infer lifecycle from terminal text. Hooks stay
  explicit Settings actions. The worktree wizard may create a checkout and a
  terminal; the user starts `claude` / `codex` / … in that terminal.
- Preserve unrelated TOML keys, hook config, and unknown layout versions.
- Tests use isolated `TERMINATOR_DATA_DIR` / config dirs, not the user’s machine.
- Appearance lives in config TOML; functional settings stay in the daemon
  snapshot. Do not mix them.

## Out of scope

Orca features we are **not** taking in this plan:

- Design Mode / per-worktree embedded browser
- Native chat UI, agent hibernation, cloud VMs, mobile companion
- Orchestration DAGs, computer-use, public artifacts
- Geist/glass visual identity, i18n packs, plugin marketplaces

Those need a browser or a different product. Revisit only after the native loop
below exists.

## Current state

| Area | What exists | Gap |
| --- | --- | --- |
| Appearance | Live hex preview, Orca-copied menu tokens in `appearance.rs` | No presets, density, or font family. First pane is 17 hex fields. |
| Settings chrome | 7-section sidebar, Apply/Cancel, `settings-section:*` test targets | No search, no row descriptions, Debug enum labels (`Embedded`). |
| Shell / editor | Empty `TextEdit` for shell; `nvim` as text; external presets + test file picker | Paths are typed. No detected-binary combo or Browse except “test draft”. |
| Shortcuts | `Settings.keybindings` map with `command+T` strings | **Stored but not consumed.** GUI only handles Escape/Enter for rename. Native `Cmd+O` in window tests is OS-level, not this map. |
| Agent hooks | Install / Repair / Remove grid | No PATH detection badges, no “open config” / resolved helper path. |
| macOS access | Prose in Updates | No “Choose folder” that grants the same access Explorer uses. |
| Worktrees | Daemon `WorktreeAdd/List/Remove`, CLI, sidebar tooltip “Managed Git worktree” | No create wizard, no cards, no jump palette. Add already creates a new `Project`. |
| Palette | None | No fuzzy jump across projects, sessions, files, settings. |

Relevant files:

- `crates/app/src/settings_ui.rs` — settings window
- `crates/app/src/appearance.rs` — theme apply, menu recipe, rows
- `crates/app/src/external_editor.rs` — presets + `rfd` test launch
- `crates/app/src/dialogs_ui.rs` — native folder/file pickers
- `crates/core/src/lib.rs` — `Settings`, `Request::Worktree*`
- `crates/core/src/worktrees.rs` — Git porcelain helpers
- `crates/daemon/src/main.rs` — worktree add creates a project + registry row
- `crates/hook/src/control.rs` — `ctl worktree`
- `crates/xtask/src/native.rs` — `editor-settings` fixture (clicks
  `settings-section:Terminal & Editor` and `external-program`)

---

## Shared primitives (do first)

Add a small settings-control kit in the GUI crate so later panes do not invent
new layouts. Keep it egui, not a widget library.

**New module:** `crates/app/src/settings_controls.rs`

| Control | Use for |
| --- | --- |
| `settings_row(ui, label, description, \|ui\| control)` | Every settings field |
| `segmented<T>(ui, value, options)` | Editor mode, diff viewer, dismissal, theme, density |
| `path_field(ui, value, kind)` | Shell, editor, custom external binary; Browse uses existing `rfd` + `picker_active` |
| `detected_combo(ui, value, candidates, allow_custom)` | Detected shells/editors + Custom |
| `shortcut_row(ui, action, binding)` | Capture on click, display `⌘T` / `Ctrl+Shift+D` |
| `status_badge(ui, text, tone)` | Detected / Not on PATH / Configured |
| `settings_search` | Filter sidebar + fields by label, description, keywords |

Rules:

- Description is one muted line. Empty description is allowed for obvious toggles.
- Invalid paths show the existing failed-status color, not a toast-only error.
- Browse never blocks the GUI thread (same `thread` + `Update::Picked*` pattern as
  `dialogs_ui.rs`).
- Apply/Cancel stays global for daemon `Settings` and appearance TOML. Path
  pickers write the **draft**, not disk, until Apply. Appearance already
  live-previews the draft; keep that.

Unit-test the kit with headless egui: row hit targets, combo selection, invalid
hex/path labels, shortcut parse/display round-trip.

---

## Phase 1 — Settings information architecture

Make Settings look like a product, not a debug grid.

1. Named sections instead of magic indexes `0..=6`. Deep links from Installation
   (`settings_section = 6`) and Agent Hooks (`= 5`) become enums.
2. Search field at the top of the window. Query matches section titles, field
   labels, descriptions, and keywords (`nvim`, `zsh`, `cmd`, `hooks`). Matching
   fields stay visible; non-matching sections hide. Empty query is today’s list.
3. Replace `format!("{:?}", editor_mode)` with segmented labels:
   Embedded Neovim / Terminal editor / External editor.
4. Same for diff viewer and notification dismissal.
5. Each field gets a description. Example: shell “Used for new terminals. Leave
   Automatic unless you need a specific binary.”
6. Keep test ids: `settings-section:Terminal & Editor`, `settings-section:Updates`,
   `external-program`, `settings-cancel`. Add ids for new controls rather than
   renaming old ones until fixtures are updated in the same change.

**Validation:** existing `cargo xtask gui` editor-settings and updates-settings
cases still click the old targets. New headless tests for search filtering.

**PR shape:** GUI-only. No daemon/schema change.

---

## Phase 2 — Path and binary wizards

Stop free-text as the primary way to choose executables.

### Shell override

- Candidates: Automatic (empty string, current `default_shell()` order
  zsh → bash → sh), then every `find_executable` hit among `zsh`, `bash`,
  `fish`, `sh`, plus `$SHELL` if it exists.
- Combo of display name + resolved path. Custom → Browse file picker.
- Show resolved path under the control. Missing binary uses failed color and
  blocks Apply via `Settings::validate`.

### Editor executable

- Candidates: `nvim` on PATH (label the resolved path), then Browse.
- Terminal-editor mode is the only time this field is enabled.
- Embedded mode copy: “Uses Neovim with your config. Install `nvim` on PATH.”
  If missing, badge + link-style “Open install notes” is enough; do not invent
  a download wizard.

### External editor

- Keep presets (System default, VS Code, Cursor, RustRover, Zed, Custom).
- Custom executable uses Browse, not a blank text field as the first action.
- Arguments stay a list (one row each) because they are tokens, not a path.
  Add a read-only chip `{file}` so people do not type the path themselves.
- Keep “Choose file and test…”.

### macOS folder access

- Move the Files and Folders prose out of Updates into Terminal & Editor (or a
  short “Privacy” subsection).
- Primary control: **Choose folder…** — the same `rfd` folder picker Explorer
  uses to grant access. After pick, add/select that project if the user confirms.
- Keep the System Settings sentence as help, not as the action.

Daemon: extend `Settings::validate` so non-empty `shell` / `editor_program` /
`external_editor` must exist and be executable (`executable_available`). Empty
shell remains valid (Automatic). Older daemons already accept these strings;
validation is additive and local.

**Validation:** unit tests for candidate lists (PATH fixtures). Native fixture:
open Settings → Terminal, pick Custom, Browse is the same picker path as
`editor-settings` (update the fixture to click Browse if `external-program`
becomes a path field). Do not fail Apply on a missing optional Custom when the
preset is not Custom.

---

## Phase 3 — Appearance: presets first, hex last

Today the first Settings pane is a color spreadsheet. Invert it.

1. **Theme preset** segmented: Current saved, Terminator dark (today’s default),
   High contrast. Selecting a preset fills `theme_draft`.
2. **Accent** color picker (drives `accent` + `selection` derived from it).
3. **Density** segmented: Comfortable / Compact (maps to `pane_divider_width`,
   row heights, `item_spacing` — keep numeric ranges already validated).
4. **Interface / Terminal / Git / Agent** collapsing groups remain, but default
   **closed**, labeled Advanced. Hex + swatch stay there for power users.
5. Live preview unchanged (`preview_appearance`).
6. Reset appearance still restores `AppearanceConfig::default()`.

Optional follow-up, same phase if small: **UI font / terminal font family**
combobox of families already loaded (`Inter`, `JetBrains Mono`) plus system
families we can enumerate without extra crates if egui/winit expose them. If
system enumeration is messy on Linux, ship bundled families + “Default” first
and defer OS fonts.

Do not add a 16-color terminal palette editor in v1. We already set
background/foreground; full ANSI maps live in `egui_term` and can stay default.

**Validation:** `AppearanceConfig::validate` unchanged. Headless test: choosing
High contrast changes draft tokens; Cancel restores committed. Native screenshot
of Appearance after presets (replace `docs/screenshots/*/settings.png` in a
dedicated capture, not in the logic PR).

**PR shape:** GUI + `appearance` TOML only. No daemon field.

---

## Phase 4 — Shortcuts that actually work

Two bugs, one phase: the UI is free text, and the map is not applied.

1. Parse `command+shift+D` (and `cmd`, `ctrl`, `alt`, `super`) into
   `egui::Modifiers` + `egui::Key`. Reject unknown tokens in `Settings::validate`
   **or** in the GUI draft if we want older daemons to keep accepting junk
   strings. Prefer GUI-side validation plus a tolerant parser so an older
   daemon snapshot cannot take the GUI down.
2. Capture: click a row, next key event with modifiers becomes the binding.
   Display as `⌘⇧D` on macOS, `Ctrl+Shift+D` on Linux. Esc cancels capture.
   Conflict: two actions with the same chord show failed color and block Apply.
3. Wire consumption in one place (`crates/app/src/shortcuts.rs`) from
   `App::ui` / `logic`, **before** terminal input, and only when a terminal is
   not in a raw key-capture situation (rename, settings capture, modal).
4. Default actions stay: `new_terminal`, `open_file`, `split_right`,
   `split_down`, `next_pane`. Add `open_settings`, `open_palette` when Phase 6
   lands. Reset-to-defaults button.
5. Menu items that have bindings show the pretty shortcut on the right (the
   `menu_item` trailing column already exists and is unused for these).

Hardcoded OS tests (`open_file_shortcut` sending `Cmd+O`) should start working
for Open File once wired. If they already open a native dialog through some
other path, document that in the PR; do not double-open.

**Validation:** parser unit tests; conflict tests; headless consume_key for
`command+T` creating a tab intent without sending `T` to the PTY. Native:
Shortcuts pane capture is optional; at least keep Apply of a valid map.

---

## Phase 5 — Agent catalog

Keep explicit Install / Repair / Remove. Add detection so the grid is a wizard,
not a guess.

For each `terminator_integrations::AGENTS` row:

| Column | Content |
| --- | --- |
| Name | claude / codex / … |
| CLI | badge Detected (`find_executable`) or Not on PATH |
| Hooks | Configured / Not configured (existing `hook_status`) |
| Actions | Install or Repair, Remove |

Help text stays: “Agents are launched manually.” Show the managed config path
on hover. Do not add “Start Claude” buttons.

`Job::HookStatus` can grow a parallel `Job::AgentBinaries` on the existing
worker, or HookStatus can return both. Avoid a new daemon capability.

**Validation:** installer tests already isolated; add unit tests for badge
matrix. Native screenshot optional.

---

## Phase 6 — Look and feel chrome + command palette

Do this after Settings feels adult, so the palette can jump into Settings rows.

### Chrome (GUI-only)

- Session/project rows: status **dot** using `status_running` / `waiting` /
  `failed` instead of long badge strings when space is tight. Keep the full
  text on hover.
- Hover-reveal for `+`, split, close on tabs/rows (`can-hover` equivalent:
  show on hover or when the row is selected; always show on coarse pointer if
  we can detect it, otherwise selected-or-hover is enough).
- Compact density from Phase 3 actually changes sidebar row height (28 → 24)
  and tab padding.
- Destructive menu items already tint red; keep that.

Do not restyle the whole app in one PR. Ship dots + hover-reveal + density as
one change with before/after screenshots of the project sidebar and a split
workspace.

### Command palette

New modal, same `popups.window` placement rules.

- Shortcut: `command+P` / `command+K` (pick one default, put the other in
  Settings). Wired through Phase 4.
- Sources, in one fuzzy list: visible projects, live sessions, recent files
  from Explorer/Git, Settings sections/fields, “Add project”, “New terminal”,
  “New task worktree” (Phase 7).
- Enter runs the action; Esc closes; typing filters.
- Does not send keys to the focused PTY while open (same as Settings).

No daemon change. File list uses the in-memory Explorer/Git model already on
the GUI worker; do not walk the disk on the UI thread.

**Validation:** headless palette filter + action dispatch. Native `cargo xtask
gui` case if we add a `palette` target; otherwise unit + one screenshot.

---

## Phase 7 — Worktree-native task loop

This is the product feature. Shape it to Terminator, not Orca.

Orca: one prompt fans out into N agent processes in N worktrees automatically.

Terminator: wizard creates isolated checkouts and terminals; **user** starts
agents; existing hooks show waiting/done; Git review stays the merge path.

### 7a. New task worktree wizard (GUI + existing `worktrees-v1`)

Dialog fields (all pickers, not a raw git command line):

1. **Source project** — current project, must be a git checkout.
2. **Start from** — combo of `HEAD`, current branch, `main`/`master` if they
   exist, plus a short ref text for tags/SHAs (this one text field is justified).
   Resolve with existing `worktrees::resolve_start`.
3. **Branch name** — default `terminator/<project>-<short>` or the leaf folder
   name; validate with `git check-ref-format --branch` (already in `add`).
4. **Destination folder** — Browse (`rfd` pick_folder parent) + generated leaf
   name. Must be absolute, UTF-8, must not exist (`worktrees::add` already
   enforces this).
5. **Open** — checkbox “Create a terminal in the new project” (default on).

On confirm: `Request::WorktreeAdd` (already creates a `Project` + registry
row). Select that project. If Open is checked, `Request::Create` in it.

If the running daemon lacks `worktrees-v1`, hide the action and keep CLI-only
behavior. Do not send the request.

Remove stays the existing CLI contract in the GUI: only for **managed**
worktrees (`state.worktrees` with `removed == false`), after `ensure_unused`.
Confirmation names the path and live-session rule. Dirty/locked Git errors
surface as the existing error banner.

### 7b. Sidebar grouping

Under a source repo, show managed worktrees as child cards: name, branch,
status dot from any live agent in that project, terminal count.

Primary checkout remains a normal project. Do not invent a new identity; the
daemon already gives worktree projects their own `project_id`.

Context menu: Open, New terminal, Remove worktree (gated), Reveal in Explorer.

### 7c. Parallel, still user-driven

Palette / wizard: “New task worktree” can be run N times. Optional later:
“Duplicate this task into N worktrees” that only clones the checkout + opens
N terminals, still without starting agents.

Compare: existing Git panel + native diff on each project. No auto-merge.

### 7d. Docs and CLI

- `docs/REFERENCE.md`: GUI wizard steps.
- `docs/ARCHITECTURE.md`: one paragraph that the GUI is now a client of
  `worktrees-v1`, Git still owns the checkout.
- CLI `ctl worktree` remains the scriptable path; do not break flags.

**Validation:**

- Existing worktree unit tests in `core` stay the source of truth for Git
  refusal cases.
- Integration: isolated repo, GUI (or ctl) add, project appears, terminal
  create, remove refused while live, remove succeeds after stop.
- Native screenshot: wizard + two worktree cards.
- Record limits in `docs/VALIDATION.md` (no claim that we race five providers).

**PR split:** 7a wizard first (smallest user-visible win), 7b sidebar, 7c
palette entry (depends on Phase 6).

---

## Suggested order

Ship in this order so each PR is usable alone:

1. Shared settings controls + Phase 1 (search, segmented, descriptions)
2. Phase 2 path wizards (shell/editor/folder)
3. Phase 3 appearance presets
4. Phase 4 shortcuts parse + capture + wire
5. Phase 5 agent catalog
6. Phase 6 chrome + palette
7. Phase 7 worktrees (wizard, then sidebar, then palette action)

Do not start Phase 7 until Phase 2 Browse exists; the wizard is mostly pickers.

## Testing matrix (every phase)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
```

Plus the native fixture that the phase touches. Settings work must keep:

- `crates/xtask/src/native.rs` `editor-settings` (draft does not save until Apply)
- `crates/xtask/src/native/updates.rs` Updates section target
- installation settings visibility

After GUI-visible changes, recapture `docs/screenshots/*/settings.png` in one
dedicated capture pass, not mixed into logic commits.

## Explicit non-goals for v1 of each phase

- Auto-apply daemon settings on every keystroke (keeps the editor-settings
  fixture’s “draft does not save” guarantee).
- Downloading Neovim, shells, or agent CLIs.
- Changing hook installers to run on GUI launch.
- Worktree branch deletion, force-remove, or dirty discard from the GUI.
- Sending a starter prompt into the new terminal (that is launching an agent
  by another name). Revisit only as an explicit “paste my prompt” checkbox
  that still does not exec the CLI.
