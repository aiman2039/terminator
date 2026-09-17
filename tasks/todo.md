# Plan: Workspace tab context actions

SPEC.md absent — this file is spec until a human adds one.

Top-level project tabs (header strip, not inner pane dock tabs) right-click currently: Rename, Close tab…. Add left/right close and insert.

## Summary

Workspace tab context menu gains:

1. Close all tabs to the left
2. Close all tabs to the right
3. Add tab to the left
4. Add tab to the right

Reuse existing close confirmation (idle / unsaved editor / keep-running). New tabs are the same as `+` (top-level terminal), inserted at the chosen index.

## Flow

```
Right-click workspace tab i
  ├ Add left  → Create → Workspace::add_at(i)
  ├ Add right → Create → Workspace::add_at(i+1)
  ├ Close left  → queue tabs[0..i]
  └ Close right → queue tabs[i+1..]
        → existing close_workspace per tab
        → Cancel aborts remaining queue
```

```
T1 ─ T2 ─ T3
```

- [x] T1 Insert: `add_at` + create left/right
- [x] T2 Close queue: left/right through existing close path
- [x] T3 Docs (CHANGELOG, REFERENCE). AGENTS.md: none

## T1 Insert

`Workspace::add_at(index, id, pane)`; `add` becomes append. Clamp index.

Thread insert index through `After::Workspace` / `Update::WorkspaceCreated` (append if `None`).

Menu: **Add tab to the left** / **Add tab to the right**. Same create as `+`. Disable nothing (always valid).

Extract tab context menu from `workspace_bar` so complexity stays flat.

## T2 Close queue

Menu: **Close all tabs to the left…** / **Close all tabs to the right…**. Disabled when that side is empty.

Queue remaining tab IDs. Current tab still uses `close_workspace`. On success, pop next. Cancel / dialog dismiss clears the queue. Empty/dead tabs drain without a prompt (existing path).

Do not batch into one dialog — editor/idle/shell rules differ per tab.

## T3 Docs

CHANGELOG unreleased. REFERENCE: top-tab right-click lists the four actions. VALIDATION: unit tests only; no native fixture unless asked.

## Out

- Inner pane (egui_dock) tab menus
- Close others
- Reorder-by-drag
- AGENTS.md (no rule change)

## Accept

- Right-click tab 2 of 3: add left → new tab at index 1; add right → index 3
- Close left/right uses existing keep-running / editor / idle prompts; Cancel stops the rest
- Empty side items disabled
- `cargo fmt`, `cargo test -p terminator --locked`, clippy app crate

## Unresolved

- Confirm target is the header workspace strip (assumed), not inner pane tabs
