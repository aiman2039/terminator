# Upstream

Source: https://github.com/lampsitter/egui_commonmark

Version: crates.io `egui_commonmark` 0.25.0. Retains the MIT and Apache-2.0
license files. The backend is crates.io `egui_commonmark_backend` 0.26.0
with egui 0.37. The parser calls `viewer_cache` and the 0.26 `end` methods.
Search-scroll arguments stay off. Split points use `SplitPoint`.

Local change: table header and body cells use `horizontal_wrapped` instead
of `horizontal`, so explicit text newlines advance to the next line.
Each cell can use the viewer width so the grid's initial narrow cell width
does not split words or formatting spans.
The app converts HTML `br` tags to newline entities before rendering;
these keep the Markdown table row intact during parsing.

Coverage: `markdown::markdown_tests` checks rendered line positions and
preserves table cell counts and code literals.

List item content sets its first row to the body text height. The default
interaction height in `horizontal_wrapped` otherwise moves the text baseline
below the number. Number painting and the existing gutter remain unchanged.

Table state uses `Ui::scope_id`. The scrollable entry takes `AsIdSalt` and `Id::unique`.
The internal scrollable renderer uses the same `pulldown_cmark` feature gate
as its public entry point. Scroll-only types use qualified `egui::Id` and
`egui::Pos2` paths so builds without that feature have no unused imports or
dead method warning.

Both render paths retain extra source blank lines between parsed blocks as
additional hard-break events. The Markdown block tree and source offsets
remain unchanged, including code blocks and list nesting.

Headings use the `Heading` text style family at every level, with sizes
stepped between `Body` and `Heading` (the backend only colors levels 2-6
strong, at sizes close to H1). H1 and H2 paint a GitHub-style divider in
the gap below the heading text; the layout does not change.
