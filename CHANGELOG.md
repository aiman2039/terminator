# Changelog

## Unreleased

- Settings and Player open as the main pane between the sidebars, not as
  floating popups. Each is a singleton: hide (workspace tab, new terminal,
  open file/session) keeps the instance; reopen shows the same view.
  Settings Cancel discards. Hiding Player does not stop audio.
- Use less GUI RAM: snapshot only the visible terminal grid (keep 10k
  scrollback in the live emulator), paint style runs instead of per-glyph
  shapes, cap the font atlas at 4096, and downscale image previews to 4K.

- Top-level tab right-click: close all tabs to the left/right, add a tab to
  the left/right. Close left/right reuses the existing per-tab keep-running,
  idle, and unsaved-editor prompts; Cancel stops the rest.

- Terminal left-drag always selects host text, including in nvim and
  full-screen agents that enable mouse reporting. Wheel still goes to the
  app. Empty Cmd/Ctrl+C no longer clears the clipboard. Dragging is cheaper
  (no full-grid clone until output changes).

- OpenCode Agent Hooks forward permission and question events (v1 and v2),
  matching Claude/Codex/Grok notification coverage. Repair an existing
  OpenCode install to pick this up.

- Compact Agents inbox cards to one row: a status icon, the session name,
  and go / snooze / dismiss. Hover the icon for the status meaning. Actions
  stay aligned across cards and wrap as a group when the sidebar is narrow.

- Keep creating tabs and splits if the serving generation is wrongly marked
  Retired. Catalog-active owners stay live for Create; the daemon restores
  Active and skips archive prune while sessions are still running.

- Play a system sound with desktop banners. Settings → Notifications has
  **Play sound with desktop notifications** (default on). The in-app Agents
  inbox stays silent. Older running daemons hide the control until updated.

- Open mp3/flac/ogg/wav/m4a/opus/aac in the chrome player next to the
  project-sidebar Agents bell. Playing shows title plus previous / play-pause /
  next / playlist. The icon opens a single global Player view in the main pane
  (like Settings, not a tab and not a popup). Opening it again shows the same
  instance; leftover Player tabs close. The full view is a Winamp-inspired
  stacked deck: LCD + transport, visual EQ, playlist with ADD/REM/SEL/MISC/LIST.
  Shuffle and repeat persist. Icecast/Shoutcast HTTP(S) radio shares the same
  engine (ADD url / Stations). Named playlists are global. Opening a file
  appends to the selected playlist. Audio stops when the GUI exits, not when
  the Player view hides. The player is native egui.

- Explorer always uses default viewers (html → browser, images, markdown
  preview, audio). Git sidebar click on a changed file opens the diff.

- Open HTML and http(s) in a GUI-only Browser tab (layout version 6, OS webview).
  v4 Html tabs migrate. Isolated profile. Covered panes hide the native view.

- Always show **Check for Updates…** in the application menu, matching AppDock.
  Sparkle still loads only from an installed release; other launches explain why
  updates are unavailable instead of hiding the item.
- Probe for Sparkle updates as soon as the updater can check, then every minute.
  Automatic checks default on. Do not consume the minute or get stuck while
  Sparkle is not ready.

- Forward unbound Command/Ctrl chords in a focused terminal as kitty CSI u
  (Command is Super, so Cmd+E reaches Neovim as `<D-e>`). Settings shortcuts,
  Cmd/Ctrl+C/V copy and paste, and Ctrl+A–Z control characters are unchanged.

- Ship Apple Silicon-only macOS releases. Drop the Intel slice, universal lipo,
  and `macos-15-intel` runner. The hosted DMG is `terminator-vVERSION-macos.dmg`.
  Linux x86-64 is unchanged. Local Intel source builds still compile.

- Sort the PROJECTS sidebar by name (A → Z / Z → A) or latest activity.
  The choice persists in UI preferences. Latest activity uses session, agent,
  and notice timestamps, plus restoring, adding, or creating a project.
  Selecting a visible project does not move it.

- Restart a mismatched session service from the status bar or Settings →
  Installation: confirm, then a detached helper stops live sessions and reopens
  this version. Sparkle updates still leave running terminals attached.

- Keep attention compact on the left Agents row until that inbox is opened.
  Git, Explorer, and History no longer list pending agent events. Waiting
  input and permission notices leave the inbox when the agent continues or a
  newer request replaces them.

- Default Git reviews to the native similar+syntect viewer (word-level hunks,
  syntax highlighting, Unified/Split). Settings → Diff viewer can restore Neovim
  CodeDiff. Double-click a git-dirty file to open the matching side.

- Keep release package output outside Cargo caches, reuse the native assembly
  tooling, and build only shipped binaries. Run macOS assembly independently of
  Linux builds and upload Cargo timing reports for each target.

- Keep a private helper for each daemon so app replacement or removal cannot break
  its terminal creation, shell hooks or GUI attachments. Add installation recovery
  under Settings → Updates, with live-session navigation and safe idle repair.

- Refuse worktree removal when another project's live session, editor file or
  child process uses the checkout, including symlink paths and missing cwd hooks.
- Negotiate chunked snapshots when state exceeds the 8 MiB frame limit, retaining
  all pending notifications and their details. Legacy oversized requests receive
  an explicit error; ordinary snapshots retain their existing format.
- Close both attachment directions on output failure while preserving the PTY
  for reattachment.
- Use editor-specific launch arguments in terminal-editor mode; custom programs
  receive the absolute file operand without Neovim startup commands.

- Extract settings, sidebar, dialog and workspace rendering into sibling app
  modules while retaining `App` state and daemon PTY ownership.
- Reject saved layouts with invalid pane focus or a missing main surface, report
  the error and preserve the original layout. Record retained runtime assertions
  and their invariants in the [panic audit](docs/PANIC_AUDIT.md).
- Test that clean locked worktrees refuse removal through the Git helper and
  daemon/CLI, retaining files, locks, registration and branch references. Verify
  successful removal after unlocking without deleting the branch.
- Refresh development and CodeDiff validation instructions to use `cargo xtask`.

Compatibility: additive, opt-in snapshot response framing; no dependency,
persistence-format or protocol-version bump.
Hosted macOS releases are Apple Silicon only; Intel Macs can still be built from source.
Running daemons keep their binary version until they exit; building or reopening
only the GUI does not replace them. Neovim reviews require the advertised
`nvim-review-v1` capability and Neovim 0.10+; older daemons use the built-in diff.
Image-bearing layouts use version 3; ordinary version-2 layouts remain readable,
and unknown versions are preserved without writes.
