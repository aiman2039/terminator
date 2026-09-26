# Architecture, flow, and code explorer

Research date: 2026-09-25. Status: proposed approach; no implementation or prototype validation yet.

## Goal

Build a browsable code atlas: **architecture ↔ flows ↔ functions ↔ source**. Use one shared model behind these views so navigation preserves context and supports tracing relationships in both directions.

| Action | Result |
|---|---|
| Open the architecture | See major components and their relationships |
| Dive into a component | Reveal its modules and relevant flows |
| Select a flow | Follow its steps, branches, and boundaries |
| Click a step or arrow | Show the implementing functions and exact call sites |
| Select code → “Show in flow” | Highlight every mapped flow containing that symbol |
| Go back/up | Restore the previous selection, zoom, and breadcrumb |

## Existing tools to reuse

### Archify

The installed Archify skill supports nested diagram links, source references with line ranges, relationship tracing, and shareable deep links. It supplies much of the desired visual experience; semantic code indexing and reverse navigation would be additional work.

Its architecture schema supports component `sources` containing a path and optional line range, plus an `interior` link to a same-directory HTML diagram. These references do not by themselves provide live symbol resolution or editor synchronization.

Local references inspected during research:

- `/Users/ohaddahan/.agents/skills/archify/SKILL.md`
- `/Users/ohaddahan/.agents/skills/archify/references/viewer-runtime.md`
- `/Users/ohaddahan/.agents/skills/archify/schemas/architecture.schema.json`

### LikeC4

A strong alternative for the architecture layer. It supports scoped drill-down views and flow steps that navigate into more detailed flows. These are authored models, not automatically recovered execution paths.

Sources: [scoped views](https://likec4.dev/dsl/views/), [dynamic flows](https://likec4.dev/dsl/views/dynamic/).

### rust-analyzer, SCIP, and LSP

Reuse compiler-aware symbol resolution and code navigation. SCIP supplies a reusable code index; LSP defines navigation and call-hierarchy requests, subject to language-server support. Validate the actual index and server capabilities before depending on particular relationship types.

Sources: [SCIP](https://github.com/scip-code/scip), [LSP specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/), [rust-analyzer SCIP maintenance](https://rust-analyzer.github.io/thisweek/2025/12/22/changelog-307.html).

### React Flow and ELK

An option if a more customizable explorer is needed than Archify provides. Existing packages handle nested nodes and graph layout; they do not supply the semantic code model.

Sources: [nested graphs](https://reactflow.dev/learn/layouting/sub-flows), [ELK integration](https://reactflow.dev/examples/layout/elkjs).

## Shared model and source mapping

Store many-to-many mappings between diagram elements and code:

```text
Component / flow step / relationship
    ↕ many-to-many mapping
Repository + revision + symbol + source range
```

Map arrows as well as nodes. Clicking “GUI requests editor creation” should reveal the sending call site, IPC message, and receiving handler.

Use symbol identities with revision and file hashes; line numbers alone become stale. Maintain a reverse index so a function can reveal all associated architecture views and flows. When code changes, refresh affected mappings or mark them stale rather than silently pointing at unrelated code. Symbol identities also need reconciliation after renames or moves.

Keep view state separate from semantic data: selection, zoom, and breadcrumbs should not redefine the underlying relationships.

## Generation and evidence

Combine three sources:

- **Code analysis:** symbols, references, calls, imports, and implementations.
- **Authored or AI-assisted interpretation:** subsystem boundaries and meaningful workflows, backed by source evidence.
- **Optional runtime traces:** actual observed execution, including async boundaries.

Keep authored relationships, statically resolved calls, and observed execution visibly distinct. A call graph cannot fully recover application behavior across IPC, callbacks, or dynamic dispatch. AI-proposed relationships need evidence or an explicit inferred label.

OpenTelemetry provides spans and causal links for runtime tracing, but requires instrumentation and mapping back to code. An observed trace describes one execution, not every possible path.

Source: [OpenTelemetry trace model](https://opentelemetry.io/docs/concepts/signals/traces/).

## Terminator integration

Existing HTML tabs can host the diagram through the GUI-owned OS webview. Current repository rules prohibit a JS bridge into the app, so bidirectional control of the Neovim editor needs a separately designed integration that respects those rules. Any proposed change to `AGENTS.md` requires specific user permission and the exact proposed edit.

A standalone diagram with an embedded, read-only source pane is a practical first prototype within the current constraint. Source-to-flow navigation can work inside that artifact without controlling the native editor.

Preserve daemon ownership of editors and PTYs. Indexing and source preparation must run outside GUI rendering through the existing bounded service architecture.

Repository reference: [implementation architecture](docs/ARCHITECTURE.md).

## Recommended first prototype

Start with one end-to-end flow: **opening a text file**.

1. Show an architecture overview of the participating components.
2. Drill into the file-opening flow and its boundaries.
3. Select a step or relationship to inspect exact source evidence.
4. Select a mapped symbol in the source pane to return to its flow occurrences.
5. Preserve selection, zoom, and breadcrumbs when navigating back or up.
6. Detect changed source and visibly mark stale mappings.

Use Archify for the initial visual direction and existing code-indexing tools for source identity. Evaluate LikeC4 or React Flow only where the prototype exposes a concrete navigation or rendering limitation.

Validate the round trip from architecture to flow to source and back before expanding to whole-workspace indexing or live editor integration. Runtime traces can follow once static navigation is useful.
