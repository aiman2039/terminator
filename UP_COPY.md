# Plan: copy the focused agent's last response

Research conclusion: the shortcut is reliable for Claude, Codex, and Grok when it copies the provider's own last-answer field from the Stop hook. It is not reliable if it copies the terminal grid or scrollback.

The focused pane is a terminal session (`active_session`). Hooks bind an agent to that session. There is no separate focused-agent object.

## Current behavior

`Agent` (`crates/core/src/lib.rs`) stores the invocation, the terminal session, the kind, the provider session id, the state, the sequence, and a resume command. It does not store the reply.

`normalize` (`crates/integrations/src/lib.rs`) already reads `message`, `last_assistant_message`, `last-assistant-message`, and `lastAssistantMessage`, then keeps 1000 characters as the notification `summary`. `details` is only `Agent`, `Event`, and `Session`. Transcripts are omitted on purpose.

`State::apply_hook` drops a second completion while the agent is already `Completed` and the event has no `request_id` (`repeated_state`). The inbox text is therefore not the last reply.

`terminator-hook` reads at most 128 KiB of stdin and allows 500 ms (`crates/hook/src/main.rs`). A larger Stop payload is rejected, so the completion never lands. `apply_hook` rejects a `summary` over 4096 bytes or `details` over 65536 bytes. The reply needs its own field so a long answer does not fail the lifecycle event.

Clipboard writes already exist (`ctx.copy_text`, used by Copy working directory). An empty copy must not clear the clipboard.

## Provider text

| Agent | Source | Rule |
| --- | --- | --- |
| Claude | `Stop.last_assistant_message` (CLI 2.1.47+) | Final text of that turn. Prefer this field over `transcript_path`; the transcript can lag when Stop fires. |
| Codex | `Stop.last_assistant_message` | Store it when the field is a non-empty string. A null field leaves the previous reply in place. |
| Grok | `Stop.lastAssistantMessage` | Store it only when `reason == "end_turn"`. Grok clips the field at 32,768 characters and appends `… [+N chars]`. Session-end Stop (`channel_closed`, `shutdown`) and `StopFailure` are not the answer. `StopFailure.lastAssistantMessage` is the error text. |
| OpenCode | none on `session.idle` | The payload is `{ sessionID }`. Message text is on `message.updated` and the session message API. The plugin does not read either. Out of scope for the first shortcut. |
| Muse | Claude-shaped Stop hook | No confirmed last-answer field. Out of scope until a payload shows one. |

`SubagentStop` is not subscribed. The stored reply is the parent turn's final text.

A turn that is still streaming has no Stop payload yet. The shortcut copies the last finished answer, including while the agent waits on the next prompt.

## Behavior

1. On a real completion, store the provider string on that agent.
2. The shortcut copies the stored reply for `active_session`.
3. Several agents on one session: use the latest `updated` agent that has a stored reply.
4. No stored reply, focus on an editor, browser, image, diff, or center-pane Settings/Player: leave the clipboard unchanged and set a status line.
5. Do not scrape the terminal. These agents are full-screen TUIs. The grid is a viewport. The scrollback is not the conversation. Do not infer lifecycle from terminal text.

Suggested chord: `command+alt+Y`, labeled "Copy last agent response". On Linux, stored `command` already means Ctrl+Shift, so the chord must not be `command+shift+…` or it collides with an unshifted `command` chord. `command+alt+C` is already Copy working directory. Copy and Paste stay terminal-native.

`fill_defaults` inserts the chord for existing installs. Menu item next to Copy working directory, shown when the focused session has a stored reply.

## Storage

Add `last_reply: String` on `Agent`, `#[serde(default)]`, so an older snapshot and an older GUI keep working. No new daemon request and no new capability: the field rides the snapshot the GUI already polls.

Update `last_reply` inside `apply_hook` before the `repeated_state` return. A repeated `Completed` still replaces the reply when the new text is non-empty. An empty field does not wipe a stored reply.

Cap the stored string at 32 KiB (Grok's own clip). Truncation appends the same `… [+N chars]` marker only when Terminator clips below what the provider sent. The notification `summary` stays a short label. Do not put the reply in `summary` or `details`.

SQLite keeps this in the generation state document, next to the agent record. One reply per agent. Hooks stay an explicit Settings install. The shortcut does not launch an agent and does not run the resume command.

## Code

- `crates/integrations/src/lib.rs` — extract the reply in `normalize`. Grok: require `reason == "end_turn"`. Other Stop events: take the first non-empty key listed above. OpenCode idle and Muse stay empty.
- `crates/core/src/lib.rs` — `HookEvent` and `Agent` gain the field. `apply_hook` writes it even when no notification is created. Keep the 4096 `summary` check.
- `crates/app/src/shortcuts.rs`, `main.rs` — action `copy_last_agent_reply`. Resolve `active_session`, pick the agent, `ctx.copy_text` only when the string is non-empty.
- `crates/hook/src/main.rs` — leave the 128 KiB / 500 ms stdin bound. Document it as a hard miss: a Stop body over the cap drops the event and the reply together.

`AGENTS.md` needs one sentence under Architecture if this ships: the shortcut copies the stored Stop reply for the focused terminal and does not read the screen. Show that sentence and get permission before editing `AGENTS.md`.

## Tests

Name them after the behavior.

- Claude Stop with `last_assistant_message` stores the full text, beyond 1000 characters, and does not replace `details`.
- A second Stop while the agent is already `Completed` replaces `last_reply`.
- Grok `reason: "end_turn"` stores `lastAssistantMessage`. `channel_closed`, `shutdown`, and `StopFailure` leave the previous reply unchanged.
- Codex `last_assistant_message: null` leaves the previous reply unchanged.
- OpenCode `session.idle` stores no reply.
- The shortcut copies the latest reply on the focused session and does not call `copy_text` when the reply is missing or focus is an editor.
- A reply over 4096 bytes still applies the hook.

## Out of scope

- OpenCode plugin reading `client.session.messages()` on idle and sending the last assistant text parts.
- Muse, until a Stop payload shows a last-answer field.
- `SubagentStop`, in-progress streaming text, and reading `transcript_path` or provider session files.
- Raising the hook stdin cap.
