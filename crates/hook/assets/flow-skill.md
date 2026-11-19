---
name: terminator-flow
description: Work as one step of a Terminator flow. Use when this terminal is a step of a flow (`terminator-hook flow context` answers), to read the context of the step, report the result, or ask the user a question.
---

# Terminator flow step

A Terminator flow runs steps in an order. You are one step, in your own terminal. Other steps wait for your result.

## Commands

Run these in your terminal. The same commands are MCP tools of `terminator-hook mcp`: `flow_context`, `flow_report`, `flow_ask`, `flow_status`.

| Command | Use |
|---|---|
| `terminator-hook flow context` | Your run, your step, and the steps you need (with their session IDs and result notes). |
| `terminator-hook ctl read SESSION` | The terminal text of a needed step. |
| `terminator-hook flow report done --note "result"` | End your step as done. The steps after you start, and they get the note. |
| `terminator-hook flow report failed --note "why"` | End your step as failed. The steps after you do not start. |
| `terminator-hook flow ask "question"` | Ask the user. The command waits and prints the answer. |
| `terminator-hook flow status RUN` | The state of all steps of the run. |

## Procedure

1. Run `flow context` first. If it gives an error, this terminal is not a flow step: stop using this skill.
2. Read the output of the steps you need, if your task depends on them.
3. Do your task.
4. If a decision is the user's, use `flow ask`. Do not guess.
5. Report the result one time, with `flow report done --note "..."` or `flow report failed --note "..."`. In the note of a done step, write what the next steps must know (for example the file that has the plan). Keep it short and on one line.

## Notes for the person who writes the flow file

- Claude Code reports its start, so a step can have a separate `prompt`.
- Codex reports only after its first prompt. Put the prompt in the command: `command = "codex 'Write the plan.'"`.
- A step ends as done when the agent ends its turn (hook), or when the step runs `flow report done`.
- Run inputs: declare `[inputs.NAME]` with an optional `description` and `default` (no default: required). Use `{{inputs.NAME}}` in a command, prompt, gate or `rules`. Run with `terminator-hook flow run PROJECT FLOW --set NAME=VALUE`; Terminator asks for the values in its Run form. In a command, Terminator quotes each value as one shell word, so do not put quotes around the placeholder for safety (quotes are allowed). A placeholder in `$'…'`, `${…}` or backticks is an error. `flow context` shows the run's values.
- After a step fails, the runner waits. `terminator-hook flow retry RUN STEP` (or Retry in Terminator) starts it again as the next attempt in a new terminal; `flow cancel` ends the run. `flow run --end-on-failure` ends the run at the first failure.

## Rules

- Report `done` only when the task is complete and checked.
- Do not start, approve, reject or cancel a flow. Those commands are for the user.
- Do not send text to the terminals of other steps.
