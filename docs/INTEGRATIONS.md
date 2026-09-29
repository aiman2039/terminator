# Agent integration contract

## Built-in targets

| Agent | Installer destination | Observed/mapped events | Validation boundary |
| --- | --- | --- | --- |
| Claude Code | `~/.claude/settings.json` | Start, prompt/tool progress, input/permission notification, permission request, completion, failure, end | Installed CLI 2.1.260 identified; official hook schema and isolated installer/normalizer tests. No paid model run was made. |
| Codex | `~/.codex/config.toml` under `hooks` | Start, prompt/tool progress, permission, input tool request, completion, interrupt/end | CLI 0.153.4 identified; its `hooks` feature is enabled. Configuration matches the upstream schema. Failure/input coverage depends on the events exposed by that CLI. |
| OpenCode | `~/.config/opencode/plugins/terminator.js` | Session status/idle/error/end; permission and question (v1 and v2), matching Claude/Codex/Grok notification coverage | CLI 1.18.31 identified; dual-entry plugin (`server()` for 1.18.29+, `setup()` subscribe for V2). Generated sequence and invocation IDs avoid conflating repeated deliveries. Live provider combinations were not exercised. Existing installs need Settings → Agent hooks → Repair. |
| Muse Code | `~/.config/muse/settings.json` (user hooks). Install also writes `~/.muse/hooks.json` for older CLIs and trusted project hooks. | Start, prompt/tool progress, permission, subagent start/stop, completion/end | CLI 1.4 reads user hooks from `settings.json`. Tool and subagent events use a different session id than Stop; the sidebar follows the conversation id so a later tool event does not stay Working. CLI 1.0.3-R2198.1 was tested with the offline echo provider. Its cleared hook environment is handled by ancestor correlation. |
| Grok Build | `~/.grok/hooks/terminator.json` | Start, prompt/tool progress, input/permission notification, completion/failure/end | CLI 1.0.13 identified; official Grok hook format. Imported Claude hook commands are ignored when the actual agent is Grok, avoiding duplicate agent attribution. |

These are capability targets, not an assertion that all providers report identical lifecycle events. A missing callback cannot reliably be replaced with “the terminal was silent.” In particular, failures and clarification prompts are only reported when the integration receives an appropriate event. CLI upgrades may change their configuration or payloads.

Installers add/remove only managed command entries (or the app's own OpenCode plugin), retain unrelated configuration, and create a backup before mutation. No credentials or account setup are modified. Installation does not launch a provider.

Detection coverage is separate from installers: Pi has process-identity
detection but no hook installer, and lifecycle reporting still requires
hooks for every agent, including Pi. The GUI shows detected processes
without hook state as Status unavailable, never Working or Idle.

## Manual integration

Within an app-owned terminal, send a JSON event to `terminator-hook emit` on stdin. The session ID is taken from the terminal environment rather than trusted from the payload. Required example:

```json
{
  "protocol_version": 1,
  "event_id": "unique-delivery-id",
  "terminal_session_id": "overridden-by-helper",
  "agent_invocation_id": "unique-agent-run-id",
  "agent_kind": "my-agent",
  "provider_session_id": "provider-conversation-id",
  "state": "waiting_input",
  "request_id": "unique-question-id",
  "sequence": 12,
  "summary": "Choose the migration strategy",
  "details": "A short explanation intended for the local user",
  "resume": null
}
```

States: `unknown`, `running`, `waiting_input`, `waiting_permission`, `completed`, `failed`, `stopped`. Use a new invocation ID for a new process/run; provider conversation IDs may be reused when resuming. Use request IDs to distinguish separate questions and stable event IDs for retries. Include a monotonic sequence if your source guarantees it.

The helper exits successfully without emitting a permission decision even if delivery fails. Input is size/time bounded. Events are accepted only for live app sessions, with authenticated local IPC. Hooks outside an app-owned ancestry are inert. If an agent strips environment variables, pass `--data-dir PATH --runtime-dir PATH` after `emit`/`event`, as the built-in installers do. These are paths, not authentication tokens. The helper still verifies live process ancestry before correlating the event.

If an agent executes hooks in a shared process that has neither the terminal environment nor a matching ancestor shell, use that agent's supported mechanism to propagate the session capability explicitly. The application must not guess the association.

The app displays provider resume commands for manual copying; it never executes them. Built-in templates are `claude --resume ID`, `codex resume ID`, `opencode --session ID`, `muse resume ID`, and `grok --resume ID`. A custom event can omit resume information. Do not put credentials in summaries, details, or resume arguments.

## References

- [Claude Code hooks](https://code.claude.com/docs/en/hooks)
- [Codex hook engine](https://github.com/openai/codex/tree/main/codex-rs/hooks) and [config schema](https://github.com/openai/codex/blob/main/codex-rs/core/config.schema.json)
- [OpenCode plugins](https://opencode.ai/docs/plugins/) and [V2 plugin migration](https://opencode.ai/v2/docs/build/plugins/migrate-v1)
- [Muse Code SDK](https://github.com/meta-models/muse-code-sdk); installed CLI echo-provider testing verifies the project hook path and payloads.
- [Grok hook guide](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/10-hooks.md)

## Idle-shell close (2026-09-15)

Closing a shell tab asks only when something besides the launched shell is
running: a child process, a non-shell foreground process group, or a live agent.
An idle shell, a waiting builtin without children, and half-typed input close
without confirmation. Generated startup files still install cwd hooks; they do
not gate close on prompt acknowledgments. Older shells may still emit `prompt`
helper calls; the daemon ignores them. The GUI never infers readiness from
terminal text.

## ntfy phone notifications

Settings → Notifications includes an opt-in ntfy toggle, channel (topic on
`ntfy.sh`), and optional machine name. Subscribe to that channel in ntfy. The
machine prefixes titles, for example `[laptop] codex: Needs permission`.
Only agent name and status are sent; hook summaries, details, paths and prompts
are excluded. Delivery uses ntfy's [JSON publishing API](https://docs.ntfy.sh/publish/#publish-as-json).

The daemon queues pings after accepting and deduplicating a hook notification.
The selected **In app** events control which states send; desktop focus and OS
notification selections do not suppress ntfy. It continues with the GUI closed.
No hook reinstall is required. `ntfy-v1` gates the settings UI; sessions owned by
older daemons do not gain this delivery until they use a supporting daemon.
Settings are stored with functional settings in SQLite.

Delivery uses `curl` (required on PATH), one worker and a 64-item bounded queue.
Requests time out after eight seconds and are not retried; queue overflow drops
pings rather than blocking hooks. Failures log a generic daemon message and do
not affect lifecycle tracking. `TERMINATOR_NO_NOTIFICATIONS` suppresses delivery
for fixtures. This option currently supports public ntfy.sh topics without auth.

## Notification self-tests

Settings → Notifications → Send test posts directly to ntfy.sh with the draft
channel; it needs no Apply and no live terminal. If the test arrives but real
alerts do not, check the toggle, Apply, and the selected In app events. Copy
test command gives the equivalent `curl` for any terminal.

Settings → Agent hooks → Send hello from all agents emits one `waiting_input`
hello per built-in agent to the active (or first live) terminal through the
normal hook path, so it exercises the Agents inbox, desktop, and ntfy together.
It uses saved settings, needs Waiting input enabled, and works even for agents
whose hooks are not installed. Copy hello command gives a
`terminator-hook emit` loop to paste inside a Terminator terminal instead.
