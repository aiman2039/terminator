# Native diff chrome

SPEC Alignment: aligned — `docs/REFERENCE.md` native Unified/Split + syntax + word hunks. Paint-only.

Stay in current workspace (existing `workspace_ui.rs` WIP).

```
diff_view → measure font → zero spacing → show_rows(row_h)
  unified: old | new | sign | code
  split:   (old|sign|code) | (new|sign|code)
paint_row: full-width bg → gutter bg → numbers at fixed x → syntax galley
```

- [x] 1. Row geometry: item_spacing=0, row_h=font, left-align, contiguous change bands
- [x] 2. Pixel gutter: glyph-width columns; blank missing side; split one number/side; hunk bar
- [x] 3. Tests: same gutter x short vs long; sign clear of numbers; row pitch = row_h not row_h+8; split sides

Unresolved: none
