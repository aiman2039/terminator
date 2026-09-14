# Changelog

## Unreleased

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
Running daemons keep their binary version until they exit; building or reopening
only the GUI does not replace them. Neovim reviews require the advertised
`nvim-review-v1` capability and Neovim 0.10+; older daemons use the built-in diff.
Image-bearing layouts use version 3; ordinary version-2 layouts remain readable,
and unknown versions are preserved without writes.
