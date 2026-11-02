# Syntax-aware diff evaluation (#77) — 2026-11-02

**Decision: no-go for now.** The native diff viewer keeps the `similar` line
diff (`crates/app/src/diff/build.rs`, `build_with_options`). Re-evaluate when
one of the conditions in "Revisit when" is true.

## Why

1. **No compatible library engine.**
   - `difftastic` (0.72.0) is published as the `difft` binary. Its parser and
     syntax modules are `pub(crate)`, so other crates cannot call them.
   - `syndiff` (0.2.0, January 2026, about 1,900 downloads, no repository
     link) has a library API, but it requires `tree-sitter ^0.26`. Terminator
     uses `tree-sitter 0.27` in `crates/native_edit`. The `tree-sitter` crate
     declares `links = "tree-sitter"`, so Cargo cannot build both versions in
     one binary.
   - `difftastic-lib` (0.0.2) is a third-party fork on `tree-sitter 0.20`.
   - JetBrains HiMark vendors difftastic's core (`vendor/difftastic-core`).
     Doing the same here adds a large vendored engine that we must keep in
     step with upstream. That is more cost than the expected benefit below.
2. **Little grammar coverage.** Terminator has tree-sitter grammars only for
   Rust, Python and JavaScript (`crates/native_edit/src/treesitter/mod.rs`).
   Markdown and all other languages would always fall back to text.
3. **The common cases already have a text answer.**
   - Re-indentation and whitespace-only formatting: the Ignore whitespace
     option (`crates/app/src/diff/options.rs`).
   - Moved code: moved-block detection (`crates/app/src/diff/moved.rs`).
   - Wrapped or reworded lines: word and character inline diff.
   A syntax diff would mainly improve reflowed expressions and changed
   delimiters inside one statement.

## Constraints a future producer must keep

- Produce the same downstream model: `DiffDocument` lines and exact,
  non-overlapping `review::Hunk` line ranges.
- Hunk stage, unstage and discard use only hunk line ranges and the raw
  texts (`crates/app/src/diff/actions/mod.rs`). Keep the `similar` hunks for
  patch building; use the syntax result only for display alignment.
- Run on the CPU pool with a cancellation token (`service.cpu().run`), with
  an input size cap and a work limit, like the existing `DIFF_TIMEOUT` and
  `MAX_HIGHLIGHT_BYTES`.
- Fall back silently to the `similar` result when there is no grammar, the
  parse has errors, a limit is exceeded, or the result fails an exactness
  check. HiMark uses the same rule: the structural path is an upgrade, never
  a requirement.
- Neovim reviews stay read-only and do not change.

## Revisit when

- `syndiff` or another maintained crate supports the workspace tree-sitter
  version and has a public repository, or
- difftastic publishes a library API, or
- the app adds tree-sitter grammars for more of the languages users review.

Then measure latency, peak memory, alignment quality and fallback frequency
on a fixed corpus (Rust, Python, JavaScript and Markdown pairs with
reformatting, rewrapping, re-indentation and moved code) before an opt-in
setting is added. It must not become the default without that data.

## Sources

- HiMark structural diff design:
  <https://github.com/JetBrains/himark/blob/main/docs/editor/structural-diff.md>
- crates.io metadata for `syndiff`, `difftastic` and `difftastic-lib`,
  read 2026-11-02.
