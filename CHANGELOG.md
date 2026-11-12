# Changelog

## Unreleased

- The start panel lists a missing Neovim (or editor program) and Git with their install command (PATH is checked off the GUI thread). Find Action lists single Settings rows (for example Settings › Terminal › Font size) and opens Settings at that row. A dialog opened by a key opens centered on the focused pane; a dialog opened by a click keeps the pointer position.
- A Welcome page on the first run joins the setup steps: add a project, hook Install for each agent found on PATH, the keymap, SSH host import and a short Agents inbox tour. After an update, What's New shows this changelog's section for the new version (bundled, no network). Find Action reopens both.
- Settings → Appearance can import a Ghostty or kitty theme file or an iTerm2 `.itermcolors` file into the colors (Apply saves it), and Follow OS appearance picks one preset while the OS is light and one while it is dark (`follow_os_theme`, `light_theme`, `dark_theme` in `config.toml`).
- Tasks: Run task… (Find Action, project menu) lists named commands from `.terminator/tasks.toml` in the project and from `tasks.toml` next to `config.toml`, and Rerun last task runs the last one again. A Run opens a new terminal in the project and runs the command; a command that names an agent, or a task with `enter = false`, is only typed. Project task files wait for the project trust answer.
- Native editor language servers for Go (gopls), C/C++ (clangd), Java (jdtls), Lua (lua-language-server), Bash (bash-language-server), Ruby (ruby-lsp) and PHP (intelephense), and user-defined servers in Settings → Languages (file types, command, arguments). Nothing is bundled or downloaded; a missing server shows its install command, and every server still waits for the project trust answer.
- Mermaid edge labels fill with the pane color (no gray box; dark panes use the dark theme), Mermaid files show no word counts, numeric CSV/TSV columns are right-aligned, the status bar says "1 session running", and the Resume chip shows "Resume ?" (no Enter) when presence is unverified.
- Sessions, tabs and projects: multi-select session rows (tile, move, close),
  Open in split and drag a session onto a pane, Next/Previous tab keys,
  middle-click and Close editor tabs, pinned projects, Manual project order,
  folder drops on the project list, clickable status-bar agent counts, Mark
  all read, and the macOS Dock badge.
- Terminal OSC 8 hyperlinks open their URI (file:// URIs open the local file),
  and link menus add Open in browser tab and Copy link. Browser tabs search
  non-URL address text, zoom (⌘=/⌘-/⌘0), find in page (⌘F), reopen closed tabs
  (⌥⌘T), open status-bar ports in a tab, and have optional developer tools.
- Terminal tabs show the program's window title as a subtitle and tooltip (`terminal-title-v1`; a named tab keeps its name), a dot for unread completed agents, and a Resume chip in the pane caption when a stopped agent can resume in that shell.
- CSV and TSV files open in a read-only table tab with sortable columns and a
  row filter, and `.mmd`/`.mermaid` files preview as one diagram; Mermaid
  diagrams pan and zoom with Fit and 100%.
- Windows and Linux show an "Update vX" status-bar chip with release notes and Skip this version; Installation details adds Open logs folder, Copy diagnostics and Report a problem…; Linux gets a tray icon with the agent menu (GNOME needs the AppIndicator extension).
- Agent presets and launchers: Settings → Agents keeps named agent commands,
  and the **+** menu, Search Everywhere and New agent session… (⌥⌘N) open
  "New terminal with <agent>" for each preset and installed agent. The
  command is typed without Enter. The New task form can pick one too.
- Settings → Appearance has a Terminal font choice (installed monospace
  families, for example a Nerd Font for prompt glyphs) and a CJK fallback font.
  A missing saved family shows a warning and keeps the bundled JetBrains Mono.
- JetBrains keys work in every pane, terminals and Settings included: double
  Shift (Search Everywhere), ⇧⌘A (Find Action) and ⇧⌘O (Go to File). ⌘E is
  now Recent Files everywhere, and Toggle IDE mode moved to ⌥⌘E. Saved
  bindings that are still at the old defaults move once; changed bindings
  stay. On Linux a focused terminal keeps Ctrl+E and Ctrl+Shift+A.
- Tool window keys: ⌘1 Explorer, ⌘6 Problems, ⌘7 Structure, ⌘9 Git (Alt+digit
  on Linux), ⌥F12 terminal and ⌥F1 Select In. A key shows and focuses a hidden
  tool window, and hides it when it has focus. Esc goes back and never reaches
  a terminal.
- Find Action lists the native editor commands with their keys (for example
  Duplicate Line or Selection ⌘D) and the Git commands. The Git commands have
  no default keys.
- Find in Files (⇧⌘F) and Replace in Files (⇧⌘R) open from every pane, Search
  Everywhere, the editor menu and Explorer folders. They have keyboard control,
  a preview, results that stream in, file masks and Exclude globs. Replace in
  Files shows each change before it writes, with a checkbox per file and per
  line, and `$1` groups in regex mode. Rename terminal moved to ⌥⌘R, so ⇧⌘R
  is only Replace in Files; a saved binding still at ⇧⌘R moves once. On Linux
  a focused terminal keeps Ctrl+Shift+F for terminal find.
- Explorer Contents search hits open at their line. Go to File accepts
  `file:line` and `file:line:column`, and lists dotfiles that Git does not
  ignore. Find in Files and Go to File skip version control folders such as
  `.git` and `.svn`.
- Native editor: a save sends `didSave` to the language servers, so
  rust-analyzer checks again and Problems, underlines and counts update.
- Native editor completion shows a docs panel, the parameters and type on each
  row, and auto-imports. Snippets keep their placeholders: Tab and Shift+Tab
  move between them.
- Native editor Quick Documentation: rest the pointer on a symbol, or press F1,
  to see its docs and the problems at that spot in one popup. Settings →
  Editor → Documentation turns the mouse hover off.
- Native editor: Undo and Redo move the caret to the change. ⇧↩ starts a new
  line below the caret line, and ⌥⌘↩ starts one above.
- Native editor indentation follows `.editorconfig` and the indentation the
  file already uses. A save applies `insert_final_newline` and
  `trim_trailing_whitespace`, but never while Redo is possible.
- Native editor: a file opens again at its last caret, scroll position and
  folds, also after a restart and from Recent files.
- Git: Commit and Push run in the background with a spinner and Cancel. The
  20-second limit is gone; a failure shows the end of the hook output. Cancel
  sends Ctrl-C first, so Git and hooks can clean up, and stops the run after
  5 seconds. Hook output above 8 MiB no longer fails the commit.
- Git: while a commit runs, the actions that can change it in that repository
  wait. Staging and unstaging (Git panel, Stage All, diff tabs, editor gutter,
  Mark resolved), Discard, Shelve, Unshelve and Drop wait until the push
  starts; the branch switch waits until the push ends. Their controls show
  "Wait for the commit to finish". A stage sent just before Commit lands
  before Git commits. Reads, Explorer file operations and other repositories
  are not blocked.
- Git Log draws a commit graph with branch, remote and tag labels.
- Git Log filters: message or hash, branch, user, date and path. Typed text
  loads after 0.4 seconds, and the old list stays until the new one is ready.
- Compare with… a branch, remote branch, tag, typed revision, or a commit from
  the file history. It opens the read-only native diff. A renamed file is
  compared with the name it had in that revision.
- Toolbars use one icon size per row. The native editor header row is gone:
  the status bar shows the file path and the symbol at the caret, and a
  Markdown file has a floating Edit / Preview / Split toolbar. Tabs show the
  file-type icon, a dot for unsaved changes, and a folder hint when two tabs
  have the same name. The Explorer file menu no longer shows wrong keys.
- Text sizes and corner radii come from one shared scale (13 pt base, like
  JetBrains). Count and kind badge letters are 11 pt. The Find in Files and
  Find Usages previews show code at 12 pt, the same size as their line
  numbers.
- Explorer: a moved or deleted folder (for example a removed Git worktree)
  shows "Folder not found" with Locate Folder…, Remove Project and Retry, not
  the Privacy hint. If the focused terminal's folder is deleted, the Explorer
  shows the project folder with a note. A folder pick always shows a result.
  The sidebar marks a project or worktree whose folder is missing.
- IDE mode defaults to full-height sidebars, so the terminal strip stays in
  the center column. The status-bar columns button (left of the strip
  hide control) and the command palette entry "Toggle IDE sidebar height"
  switch the strip back to full width under both sidebars. The choice is
  saved in `ui-preferences.json` and in named layout presets.




- Removing ntfy settings no longer traps Apply: clearing the channel
  switches the toggle off, and a disabled toggle saves regardless of
  leftover field contents. Re-enabling still requires a valid channel.
- Settings survive damage: an unreadable store is quarantined
  (`state.corrupt-*`) and the daemon starts with defaults instead of dying;
  stored settings that fail validation reset to defaults with a degraded
  notice. A panicking request kills at most its own connection, and shared
  locks recover from poisoning instead of cascading the failure.
- ntfy phone pushes are time-gated: repeats from one agent run and state
  are held back for 60s, and any push holds back every other push for 15s,
  so bursts buzz once. The Agents inbox still records every notification.
- Agents inbox rows fold all pending notices from one agent run into a
  single card ("Terminal 3 · 3 events", hover lists each). Read, snooze,
  and dismiss apply to the whole row; Go navigates once.

- macOS menu-bar item opens a pending-notifications menu: each entry jumps
  straight to its waiting agent's terminal (same as the inbox Go button),
  with Show Terminator below and a "No pending notifications" placeholder
  when empty. The left agent bell and the IDE status-bar bell now share one
  waiting/unread count helper; both sidebars already render the same Agents
  inbox state.

- Header is a single 40px drag band again: the workspace tab strip moved
  into its own center-only row beside the sidebars, so both sidebars run
  full height. Tabs stay below the native drag band, so tab-reorder drags
  still never race window moves.

- IDE mode toggle (header switch, `command+E`, command palette): pins both
  sidebars on, adds a bottom terminal strip bound to the active shell
  session, and mirrors the player transport plus the agent notification
  bell in the status bar. Pure GUI view state in `ui-preferences.json`;
  sessions, saved layouts, and older daemons are untouched. IDE mode is
  session-only and always opens off; the bottom strip is a full terminal
  dock (tabs, splits, drag-reorder) sharing the main dock's viewer, with
  its own per-project layout persisted GUI-locally. Splits follow the
  focused dock; editors and new workspace tabs stay in the main dock.
  Strip sessions never appear as main-dock tabs, so each terminal renders
  exactly once (previously both views drove the same PTY and doubled
  input). Navigation (sidebar, palette, agents) reveals strip sessions in
  the strip.

- Main-thread panics write a crash log under the data directory
  (`crashes/`) and the GUI shows a dialog with that path before quitting.
  Terminal sessions keep running in the daemon.

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
