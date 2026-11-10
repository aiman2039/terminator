# Upstream

Source: https://github.com/lampsitter/egui_commonmark

Version: crates.io `egui_commonmark` 0.25.0. Retains the MIT and Apache-2.0
license files. The backend is crates.io `egui_commonmark_backend` 0.26.0
with egui 0.37. The parser calls `viewer_cache` and the 0.26 `end` methods.
Search-scroll arguments stay off. Split points use `SplitPoint`.

Local change: table header and body cells use a wrapping row layout instead
of `horizontal`, so explicit text newlines advance to the next line.
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

Local change (2026-11-06): a heading-scroll target stays set until its
heading paints; a later `show` call no longer clears it, so a host can show
one document as several calls. `heading_scroll_pending` is public for
Terminator's virtualized preview.

Local changes (2026-11-10), to match GitHub-style previews:

- Tables no longer use `egui::Grid`. A Grid places cells by the previous
  frame's row heights, so a wrapped cell overlapped the next row. Each row is
  now laid out directly (`table_cell`): cells have a fixed column width, wrap
  inside it, and a row is as tall as its tallest cell. Odd rows keep the
  stripe. `table_column_widths` is close to the CSS automatic table layout:
  columns that fit stay unwrapped; otherwise each column gets its longest
  word and the rest goes to the columns in proportion to their unwrapped
  width. A table that still does not fit scrolls in a horizontal
  `ScrollArea`; one that fits gets none (the area keeps room for its bar).
  Columns have an explicit gap (the document sets no item spacing), so
  `End(TableCell)` paints no gap label (the backend's `parse_row` puts it at
  the start of the next cell, where it pushed the first word to a second
  line). Rows skip the empty row `parse_table` returns last. Only a table
  that overflows measures each word.
- Header cells and `**strong**` text use the `Heading` text style family (a
  bold face when the app sets one); the backend only gives strong text the
  strong color.
- Item content after a nested list (for example a second paragraph in the
  item) starts a new row at the item's text indent and wraps there
  (`List::continue_item`); it used to continue on the nested list's last row.
- A heading that is the first event of a `show` call no longer adds an empty
  row above itself.
