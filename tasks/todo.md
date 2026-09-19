# Plan: Settings + Player as center-pane singletons

SPEC.md absent — this file is spec until a human adds one.

Settings and Player are floating `egui::Window` popups (`popups.window`). Open them instead as the **center pane** (between left/right sidebars). Not workspace tabs. Not popups. One instance each; hide/reopen keeps that instance.

## Summary

- Gear / `open_settings` / palette / Fix installation → Settings fills `CentralPanel`.
- Player icon / playlist / palette / audio open → Player fills `CentralPanel`.
- Sidebars, header workspace tabs, status bar stay.
- Dock/terminals are not painted while either view is showing (native webviews hide).
- Opening one hides the other. Audio keeps playing.
- Workspace tab click, `+`, `go_session`, file/diff/browser open → hide overlay, show dock. Instances stay.
- Reopen shows the same Settings draft/section/search and the same Player UI (eq/playlist/radio). Cancel on Settings discards and ends the session.
- Leftover `Tab::Player` still stripped. Playback still stops only on GUI exit.

## Flow

```
CenterView: Workspace | Settings | Player
                    (mutually exclusive; App owns both UIs)

open_settings  → CenterView::Settings
                 first show / after Cancel: clone drafts, clear search
                 reopen while session live: no reinit
open_player    → CenterView::Player  (seed samples once; strip Tab::Player)
hide           → CenterView::Workspace  (keep instances)
Settings Cancel → discard drafts, end settings session, Workspace
Player hide/X  → Workspace; audio continues
click workspace tab / + / go_session / open file → hide

Settings ──open──► Player  (settings session kept)
Player   ──open──► Settings
```

```
T1 ─ T2 ─ T3 ─ T4
```

- [x] T1 CenterView + hide-on-workspace
- [x] T2 Settings fills center (singleton)
- [x] T3 Player fills center (singleton)
- [x] T4 Docs (CHANGELOG, REFERENCE, ARCHITECTURE). AGENTS.md: ask first

## T1 CenterView + hide-on-workspace

`enum CenterView { Workspace, Settings, Player }` (or keep `settings_open`/`player_open` exclusive: opening one clears the other).

`fn hide_center_overlay(&mut self)` → Workspace without tearing down drafts/player.

Call hide from: workspace tab click (even already-active), workspace `+`, `go_session`, file/diff/image/browser open, New terminal.

`CentralPanel`: if Settings/Player, draw that UI and **skip** `DockArea`. Else current dock.

`browser_covered` includes Player. Terminal input stays blocked.

Keep `settings-section:*`, `settings-cancel`, `player-window` test ids.

## T2 Settings center singleton

Stop `popups.window(ctx, "Settings")`. Draw existing settings chrome in the center (nav + section + Apply/Cancel). Fill available size (drop 760×580 popup clamp).

`open_settings`: if session not live, current init (drafts, search, hook status). If live, only `CenterView::Settings`.

Cancel: revert, `settings_open = false`, session dead. Overlay X / workspace hide: keep draft.

Palette Settings / `open_installation_settings` / Set up hooks: show Settings; init only if session dead. `Show session` uses hide.

Live theme preview only while Settings is the center view.

## T3 Player center singleton

Stop `popups.window(ctx, "Player")`. `draw()` in center; playlist/radio take leftover height.

Hide/X does not stop audio. Reopen same `self.player` + prefs. Opening Player hides Settings (session kept).

Keep `dismiss_player_tabs` / `strip_player`. Chrome mini-controls unchanged.

## T4 Docs

CHANGELOG unreleased. REFERENCE + ARCHITECTURE: floating window → center pane singleton; hide ≠ stop audio; not a workspace tab.

AGENTS.md (permission): Player/Settings paragraph — popup/window → main pane between sidebars; reopen same instance.

## Out

- Extra header tabs for Settings/Player (workspace strip stays project tabs)
- OS child viewports
- Stopping audio on hide
- Restoring Player as `Tab::Player`
- Palette / first-project / keep-running stay popups

## Accept

- Open Settings: center fills; sidebars stay; no floating window; dock not painted
- Open Player while Settings showing: Player center; Settings draft intact; reopen Settings restores it
- Workspace tab click: dock back; reopen Player/Settings = same instance
- Cancel Settings discards; hide via tab does not
- Hide Player: audio continues
- No `Tab::Player`; leftover tabs still stripped
- Native fixtures still hit `settings-section:*` / `player-window`
- `cargo fmt`, `cargo test -p terminator --locked --bin terminator`, clippy app crate

## Unresolved

- Same-icon while already showing: stay (assumed) vs toggle hide
- Overlay X on Settings: hide keep draft (assumed) vs Cancel
