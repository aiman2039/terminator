# Plan: Agents inbox (Orca-style Needs You)

**Summary:** Replace Agents graveyard with Needs You cards (Go / Snooze / Dismiss). Hooks only.

**SPEC Alignment:** no SPEC.md. Aligned with AGENTS.md / REFERENCE.md (hooks only; dismiss ≠ lifecycle).

```
Hook -> Agent.state + maybe Notification
Agents pane -> pending notices as cards
  Go -> session
  Snooze -> hide 10m
  Dismiss -> hide; agent unchanged
Stopped -> History, not Agents
```

- [x] Task 1: `agents_view` = pending notifications, not `state.agents`
- [x] Task 2: shared attention card (inline Go/Snooze/Dismiss); chips don’t double-modal
- [x] Task 3: unit tests (native `agent-sidebar` click target now `agent-go`)

**Unresolved (decided for impl):** card-body click = Go; no Working-without-notice rows; keep Attention chips as compact bar.
