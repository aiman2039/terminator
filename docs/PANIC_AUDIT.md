# Runtime panic audit — 2026-09-10

Scope: app and daemon Rust runtime code, including indexing into the docking
library from app code. Searched `unwrap`, `expect`, `panic!`, `unreachable!`,
assertions, indexing/slicing, mutex acquisition and thread creation; traced
callers and guards. Test modules, test-support diagnostics, build scripts and
xtask assertions are separate from production findings. This is a bounded source
review and regression exercise, not exhaustive fuzzing of dependencies or every
possible corrupted docking-tree representation.

Two input-triggered GUI panics were reproduced and fixed. No daemon runtime panic
was reproduced. Assertions with the invariants below remain; no poisoned-state
recovery was added. Fatal panics on the installing (main) thread now write a
crash log and, in the GUI, show a native dialog before exit; the process still
dies. See [Fatal panics](ARCHITECTURE.md#fatal-panics).

| Candidate / trigger | Invariant or handling | Thread and session impact |
| --- | --- | --- |
| `workspace::active_pane`: persisted `focused_node = 999` in a one-node tree | **Reproduced** out-of-bounds panic. `Workspace::load` now rejects focus outside a leaf, for legacy and version-2/3 layouts. | GUI state loading/rendering; previously GUI loss, daemon-owned sessions remain. Existing `apply_state` status error and read-only-layout marker preserve saved JSON. |
| `Workspace` main-surface access with persisted `surfaces = []` | **Reproduced** docking-index panic. Loading now requires surface zero to be `Main`. | GUI; same existing error/preservation path, no session restart or termination. |
| `Workspace` tab indexing in dereference/normalize | Constructors create a tab; loading checks nonempty tabs and active identity; add pushes after retain, close/remove normalize before returning. Normalization bounds its index. | GUI; an internal invariant violation would lose the GUI, not daemon PTYs. |
| `App::apply_state` old-group unwrap | The `survives` predicate requires `Some(id)` still present in the owned workspace. No intervening mutation of the option. | GUI; retained. |
| `workspace_ui::inline_rename` option unwrap | `renaming` returns false unless the session and rendering surface match; early return precedes the unwrap. | GUI; retained. |
| `sidebar_ui::sidebar` Git root unwraps | Return when the cloned context has no root; render closures use that same immutable clone. | GUI; retained. |
| `settings_ui::settings` first-character unwrap and preset indexing | Appearance names come from a fixed nonempty token list. Preset state is initialized/loaded with `selected`, or selected from `PRESETS`; `preset` is called only below `CUSTOM`. | GUI; retained. |
| `workspace_ui::Viewer` unreachable diff arm and backend unwraps | Outer match already establishes Diff. Missing backend is inserted successfully or rendering returns on attach error; no backend removal between lookups. | GUI; attach failures already display errors and leave daemon ownership intact. |
| Diff/history `lines[row]`, pane cycling, split result and image-size indexing | `show_rows` uses the same line count; pane cycling checks nonempty nodes and uses modulo; split results and image sizes are fixed-size arrays. | GUI; retained. |
| `services::resolve_path` option unwrap | Parsing `rest.unwrap_or("")` as a number must succeed first; absent rest cannot pass. | File/target worker; retained. |
| `external_editor::launch` piped stderr expect | The only command builder explicitly pipes stderr before successful spawn. Missing executable/spawn/exit errors use `Result` and existing status delivery. | GUI worker; external editor lifetime remains independent. |
| External-editor diagnostic mutexes | Only bounded byte append and diagnostic conversion occur under the lock. Poisoning requires a preceding unwinding panic; no ordinary-input trigger demonstrated. | Pipe reader/waiter; a panic could lose diagnostics, not daemon sessions. No unproven poison recovery. |
| `storage::History` writer/segment unwraps and map indices | Writers and matching segments are inserted together by the sole history worker. Removal is guarded by presence; delete/prune/clear traverse owned keys. I/O failures return errors before violating those map relationships. Read slices use the bounded write count. | History worker; a panic would stop history service while PTYs remain owned elsewhere. I/O errors already report degraded storage. |
| `review::runtime` / `prepare` parent unwraps | Paths append pinned relative asset names under the runtime directory, or a checked filename under HEAD/INDEX/WORKTREE. They have parents; input/path/I/O failures return errors. | Daemon request thread; retained. |
| Daemon state/store/session/focus/worktree mutex unwraps | Poisoning requires an earlier panic while holding that particular guard. Ordinary Git, storage, request-validation and PTY errors use `Result`; no first poisoning trigger demonstrated. | Request, PTY reader/waiter/input, history and notification workers. State poison could cascade across sessions; runtime/writer poison affects its PTY. Retained rather than assuming poisoned state is consistent. |
| PTY byte slices and terminal OSC parameter indices | Read count is bounded by the buffer; OSC handlers check parameter length before indexing or slicing. Chunk/reply/notice queues are bounded. | PTY reader; retained. |
| `thread::spawn`, allocation and dependency-internal assertions | Thread/resource exhaustion and dependency faults can still panic or abort; no reproducible ordinary-input failure established here. | Depends on caller; not claimed panic-free. Blanket catching would not establish shared-state consistency. |

Regression evidence: before handling, `invalid_saved_focus_reports_error_and_preserves_layout`
panicked at `workspace.rs` with index 999/length 1;
`missing_main_surface_is_rejected_before_pane_access` panicked in the docking
library with index 0/length 0. Both now pass. The app regression verifies status
messaging and that the saved layout stays protected from writes. Supported
versioned and legacy layouts share validation without changing their format.
See [validation evidence](VALIDATION.md) for the completed workspace/native checks.
