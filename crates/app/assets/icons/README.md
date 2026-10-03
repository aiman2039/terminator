Lucide 0.468.0 SVGs (ISC/Feather MIT), tinted at render time. See LICENSE.
SVG currentColor is replaced with white for egui multiplicative tinting.
The flat UI also bundles Lucide 0.468.0 action/navigation icons for copy, paste,
selection, splits, close, search, external opening, settings and sidebar tools.

Menu action additions use [Lucide 0.577.0](https://github.com/lucide-icons/lucide/tree/0.577.0/icons):
`panel-{right,bottom,left,top}-close`, `panel-left`, `panel-right`, `pencil`,
`text-select`, `clipboard`, `eraser`, and `save`. These use the same ISC/Feather
MIT licensing (see LICENSE) and white-stroke tint adaptation. `panel-left` and
`panel-right` are the header sidebar toggles.
Right/down splits, rename, select-all, left/up extend the same pane icon family.

`refresh-cw.svg` also comes from Lucide 0.577.0, with the same license and
white-stroke adaptation, for the native Markdown refresh control.

`moon.svg` comes from Lucide 0.577.0 with the same license and white-stroke
adaptation, for the notification snooze action.

`eye.svg` and `eye-off.svg` come from Lucide 0.577.0 with the same license and
white-stroke adaptation, for the explorer's show-ignored-files toggle.

Agent inbox status icons also come from Lucide 0.577.0 with the same license
and white-stroke adaptation: `message-circle-question-mark` (needs input),
`shield-question-mark` (needs permission), `circle-check` (done),
`circle-alert` (failed), `loader-circle` (working), `circle-stop` (stopped),
and `circle-question-mark` (unknown).

`audio-lines.svg`, `skip-back.svg`, `play.svg`, `pause.svg`, `skip-forward.svg`,
`list-music.svg`, `square.svg`, `shuffle.svg`, `repeat.svg`, and `radio.svg` come from
Lucide 0.577.0 with the same license and white-stroke adaptation, for the chrome
player.

`menu.svg` comes from Lucide 0.577.0 with the same license and white-stroke
adaptation, for the header overflow menu.

`layout-dashboard.svg` comes from Lucide 0.577.0 with the same license and
white-stroke adaptation, for the IDE mode header action. Its four-pane grid
stays distinct from the single-divider sidebar toggles at 16px.

`columns-3.svg` comes from Lucide 0.577.0 with the same license and
white-stroke adaptation, for the IDE status-bar control that lets the
sidebars run full height beside the terminal strip.

`archive.svg` comes from Lucide 0.577.0 with the same license and white-stroke
adaptation, for the removed-projects control.

Agent identity icons are official vendor marks, used only to identify the
agent running in a terminal (nominative use; all marks are trademarks of
their owners: OpenAI, Anthropic, SST, xAI, Meta, Mario Zechner/pi.dev):

- `agent-codex.svg`: `OpenAI-white-monoblossom.svg` verbatim from the
  official bundle at <https://cdn.openai.com/brand/OpenAI-Logos-2025.zip>
  (brand terms: <https://openai.com/brand/>).
- `agent-claude.svg`: `favicon.svg` verbatim from <https://claude.com/>.
- `agent-pi.svg`: `favicon.svg` from <https://pi.dev/>, theme media query
  replaced with a fixed white fill for the tint pipeline; paths verbatim.
- `agent-opencode.svg`: `opencode-logo-dark.svg` verbatim from the MIT
  repo at <https://github.com/sst/opencode>
  (`packages/console/app/src/asset/brand/`).
- `agent-grok.png`: xAI mark from the official `xai-org` GitHub avatar
  (<https://github.com/xai-org>), black background converted to
  transparency (luminance alpha); mark pixels otherwise unchanged.
- `agent-muse.svg`: Meta loop mark (Muse Code has no separate public
  vector mark) via <https://cdn.simpleicons.org/meta> (CC0 reproduction).
- `agent-generic.svg` (`bot`) and `agent-multiple.svg` (`copy`) come from
  Lucide 0.577.0 with the same license and white-stroke adaptation, for
  custom hook agents and multi-agent terminals.

Tooltips and the Agents sidebar always pair a glyph with the agent name.
