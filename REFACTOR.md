# Refactor Plan — Split Oversized Files

Goal: break god-files into topic directories. No behavior change per step.
Follow `rules-commands-etc/rust.md`: one directory per topic + `mod.rs`
re-exports, large `impl` split across files, input structs, flat
orchestrators, `TopicHelpers` for stateless utils.

Gates per step (all must pass before marking done):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --locked
cargo build --workspace --locked
```

Global rules for every split:

- [ ] New directory keeps old public paths via `pub use` re-exports.
- [ ] Default to `pub` fields/methods; narrow to `pub(crate)`/`pub(super)` only if compiler allows.
- [ ] Keep input structs (`FooInput`), fully destructure them.
- [ ] Free stateless fns become `TopicHelpers::fn()` static methods.
- [ ] Orchestrator methods stay flat; move branching into small private steps.
- [ ] No dead code left behind; delete old items, don't `#[allow(dead_code)]`.
- [ ] One PR/step per file below; keep diff reviewable (<~800 lines moved per commit if possible).

## Phase 0 — Baseline

- [ ] Add `[workspace.lints.clippy] pedantic` table per `rust.md` to root `Cargo.toml`.
- [ ] Record baseline sizes: `find crates -name '*.rs' | xargs wc -l | sort -rn | head -n 20`.
- [ ] Decide max target: no file >1000 lines, ideally <800.

## Phase 1 — Critical god-files

### 1. `crates/app/src/lib.rs` (12,287 lines)

Problems: `struct App` (:509), `impl App` (:823, ~4k lines), `eframe::App` (:4968), plus `Tab/Job/Update/TerminalFind` types.

- [ ] Create `crates/app/src/app/` with `mod.rs` re-exporting old paths.
- [ ] Extract in order: `tab.rs` (`Tab`, `PaneDropZone`, `RenameSurface`), `jobs.rs` (`Job`, `Update`, `After`), `terminal_find.rs`, `project_tabs.rs` (`select_project`, `insert`, `create_workspace_tab`, close/prune fns), `rename.rs`, `browser_tabs.rs` (`open_browser_url`, `sync_browsers`, `place_gui_tab`), `shortcuts_dispatch.rs`, `state_sync.rs` (`process_updates`, `apply_state`, `migrate_attention`), `app.rs` (struct + ctor) + `update.rs` (`eframe::App` impl).
- [ ] `lib.rs` becomes declarations + `pub use app::*` only.

### 2. `crates/app/src/workspace_ui.rs` (5,416 lines)

Problems: header/tab bar (`impl App` :404) + diff painter (:2372-3018) + `TabViewer` (:3030) in one file.

- [ ] Create `crates/app/src/workspace_view/`: `mod.rs`, `header.rs` (tab bar, `HeaderAction*`, hover popup), `terminal_tabs.rs` (`impl App` workspace part), `diff_view.rs` (`DiffColors/Metrics/Paint*`, all `paint_diff_*`), `viewer.rs` (`Viewer`, `TabViewer` impls).
- [ ] Move `DiffHelpers` stateless fns into struct statics.
- [ ] Keep existing tests (`tab_tooltip_tests`, `tab_width_tests`) with their module.

### 3. `crates/app/src/sidebar_ui.rs` (4,224 lines)

Problems: `impl App` explorer+git+attention (:85-2164) + git panel types + tree + attention cards.

- [ ] Create `crates/app/src/sidebar/`: `mod.rs`, `explorer.rs` (`explorer_*`, `CardLayout`), `git_panel.rs` (`GitPanelInput/Outcome`, `git_*`, `ChangeTree`, `PreparedGit`), `attention.rs` (`attention_*` cards/badges).
- [ ] Share `GitHelpers` / `ExplorerHelpers` for pure fns (`git_color`, `build_change_tree`, `notice_preview`).

## Phase 2 — Core + daemon

### 4. `crates/core/src/lib.rs` (1,927 lines)

- [ ] Move RPC/transport (`connect`, `rpc`, `fanout_owners`, `read_frame/write_frame`, `conditional_snapshot`, `sanitize_layout`) to `ipc.rs`; process helpers to `process.rs`; keep `lib.rs` as `mod` declarations + small shared types (`now`, `id`, `atomic_write`) only.

### 5. `crates/core/src/generations.rs` (1,718 lines)

- [ ] Create `crates/core/src/generations/`: `mod.rs`, `catalog.rs` (`Catalog`, `coordinate`, `workspace_paths`), `migration.rs` (`migrate*`), `ownership.rs` (`owner_for`, `owner_is_serving`, `claim_dead_owner`), `recovery.rs` (`recover_exited`, `prune_retired`, `snapshot`, `historical`).

### 6. `crates/daemon/src/main.rs` (1,708 lines)

- [ ] Create `crates/daemon/src/server/`: `mod.rs`, `shared.rs` (`Shared`, `Disconnect`, `relock`), `dispatch.rs` (`serve` match arms), `handlers/` (one file per request group), `main.rs` (thin `fn main` + listener setup).

## Phase 3 — App leaves (do first if new to pattern)

These are smaller and good warm-ups; any order.

### 7. `crates/app/src/appearance.rs` (2,210 lines)

- [ ] Split to `appearance/`: `theme.rs` (`install`, `apply`, `color`), `controls.rs` (buttons, menus, `sidebar_action`, `selectable_icon`), `rows.rs` (`file_row`, `project_row`, `session_row_spec`, `row`), `bars.rs` (`terminal_bar`, `unsaved_close_bar`, `focus_stroke`).

### 8. `crates/app/src/settings_ui.rs` (1,441 lines)

- [ ] Split giant `impl App` (:87+): `settings/nav.rs`, `settings/body.rs`, `settings/footer.rs`, `settings/state.rs` (`open_settings`, `apply_settings`, `revert_settings_draft`, pending/dirty logic).

### 9. `crates/app/src/preferences.rs` (1,318 lines)

- [ ] Split to `preferences/`: `sorting.rs` (`sort_visible_projects`, `sort_history`), `persistence.rs` (`UiPreferences` impl), `strip.rs` (`strip_*_equal`), keep tests beside each.

### 10. `crates/app/src/workspace.rs` (1,258 lines)

- [ ] Split `impl Workspace` + `Deref/DerefMut` + `contains_browser`: `workspace/model.rs`, `workspace/layout.rs`, `workspace/query.rs`.

### 11. `crates/app/src/gui_services.rs` (1,234 lines)

- [ ] Split to `gui_services/`: `submit.rs` (`Submit`, `Services`), `jobs.rs` (`ImageJobs`, `editor_ids`, `rejection`, `context`).

### 12. `crates/app/src/player/` (`mod.rs` 1,321 + `engine.rs` 1,344)

- [ ] `mod.rs`: extract `Controller` playback logic to `controller/playback.rs` + `controller/selection.rs`; keep `impl App` (`poll_player` etc.) in `app_hooks.rs`.
- [ ] `engine.rs`: split `run/playback/pipeline` + `decode_audio/output_stream` into `engine/tasks.rs`, `engine/audio_io.rs`; keep `Handle/Analysis` in `mod.rs`.

### 13. `crates/native_edit/src/vim.rs` (1,688 lines)

- [ ] Split to `vim/`: `options.rs` (`parse_vimrc`), `motions.rs` (`word_*`, `line_len`), `engine.rs` (`VimEngine` core), `modal.rs` (`ModalEngine` impl), `text.rs` (`doc_is_empty`, `range_text`, `flat_text`).

### 14. `crates/xtask/src/native.rs` (1,446 lines)

- [ ] Already has submods; extract each case fn (`workspace_tabs`, `split_file_opening`, `editor_lifecycle`, `agent_wheel*`, etc.) into `native/cases/*.rs`; keep `run()` dispatcher thin.

## Phase 4 — Verify + close

- [ ] Re-run size check; confirm no non-vendor file >1000 lines.
- [ ] Remove `too_many_lines = allow` if now unused.
- [ ] Update `docs/ARCHITECTURE.md` if module paths changed.
- [ ] Delete this file or move remaining items to issues when all boxes ticked.
