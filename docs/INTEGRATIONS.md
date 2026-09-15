# Agent integration contract

## Built-in targets

| Agent | Installer destination | Observed/mapped events | Validation boundary |
| --- | --- | --- | --- |
| Claude Code | `~/.claude/settings.json` | Start, prompt/tool progress, input/permission notification, permission request, completion, failure, end | Installed CLI 2.1.260 identified; official hook schema and isolated installer/normalizer tests. No paid model run was made. |
| Codex | `~/.codex/config.toml` under `hooks` | Start, prompt/tool progress, permission, input tool request, completion, interrupt/end | CLI 0.153.4 identified; its `hooks` feature is enabled. Configuration matches the upstream schema. Failure/input coverage depends on the events exposed by that CLI. |
| OpenCode | `~/.config/opencode/plugins/terminator.js` | Session status/idle/error/end, permission and question events | CLI 1.18.29 identified; plugin follows the published event API. Generated sequence and invocation IDs avoid conflating repeated deliveries. Live provider combinations were not exercised. |
| Muse Code | `~/.muse/hooks.json` | Start, prompt/tool progress, permission, completion/end | CLI 1.0.3-R2198.1 tested end to end with the offline echo provider and isolated project hooks. Its cleared hook environment is handled by ancestor correlation. |
| Grok Build | `~/.grok/hooks/terminator.json` | Start, prompt/tool progress, input/permission notification, completion/failure/end | CLI 1.0.13 identified; official Grok hook format. Imported Claude hook commands are ignored when the actual agent is Grok, avoiding duplicate agent attribution. |

These are capability targets, not an assertion that all providers report identical lifecycle events. A missing callback cannot reliably be replaced with “the terminal was silent.” In particular, failures and clarification prompts are only reported when the integration receives an appropriate event. CLI upgrades may change their configuration or payloads.

Installers add/remove only managed command entries (or the app's own OpenCode plugin), retain unrelated configuration, and create a backup before mutation. No credentials or account setup are modified. Installation does not launch a provider.

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
- [OpenCode plugins](https://opencode.ai/docs/plugins/)
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
