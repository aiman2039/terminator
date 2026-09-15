# Pending work: scrolling, usability, and launch preparation

## Summary

This is the active backlog, updated 2026-09-15. The implementation status table distinguishes delivered work from remaining runtime acceptance gates. Fix hovered-pane scrolling and inaccessible Codex history first, then implement local DMG convenience tooling, safe idle-shell closing, and folder setup/access recovery. Prepare the Mac App Store feasibility report and local Product Hunt launch kit as separate deliverables.

Completed task files for the in-file unsaved close bar and Agents inbox have been removed. Their implementation and validation history remain in `docs/VALIDATION.md`; preserve those behaviors while implementing this plan.

Decisions confirmed: idle shells end without confirmation; first launch offers folder setup; local DMGs default to fast debug builds; Product Hunt presents Terminator as a free, open-source workspace for AI-agent developers.

## Implementation status — 2026-09-15

The six workstreams now have local implementations/deliverables. The live Codex gate was subsequently verified using user-approved fresh public conversations. Remaining hardware/platform gates are listed below.

| Workstream | Delivered and verified | Remaining acceptance |
| --- | --- | --- |
| Scrolling | Hover routing, event consumption, fractions/phases, Shift-local bypass; native scales 1/2 and live Codex 0.154.0 in normal/`--no-alt-screen` modes, focused/hovered, streaming and reconnect | Physical trackpad/overlay matrix and Linux runtime |
| Local DMG | Reused package builder; repeated plain builds and styled build; mounted contents/signatures; failure cleanup and Cargo timings | No installation or publication authorized |
| Folder recovery/setup | Structured errors, retained cache, forced retry, reselection, persisted setup, Settings guidance and package descriptions; unit/native Unix-denial recovery | Real macOS TCC revocation, native first-run picker decisions and separate shell/editor permission behavior |
| Idle closure | Capability-gated batch API, authenticated prompt generations, in-flight input protection and GUI coordinator; real zsh/bash safety cases and native pane close | Fish runtime (not installed here), Linux process checks, broader custom shell profiles |
| Store report | `docs/MAC_APP_STORE_FEASIBILITY.md` | Prototype and submission are separate work |
| Product Hunt | `launch/product-hunt/`: copy, five images, provenance/checklist; lengths, dimensions, links and privacy checked | Runtime launch gates and review of the downloadable release; no posting authorized |

Fresh evidence and artifact paths are recorded in `docs/VALIDATION.md`. Existing installed applications and live daemons have not been replaced.

## Implementation

### 0. Scroll the hovered content and restore access to Codex history

**Reported problem:** Scrolling in the current window does not reliably reveal earlier Codex messages. Wheel and trackpad input must act on the content under the pointer without requiring a click or moving keyboard focus.

**Original inspected evidence (before implementation):** `vendor/egui_term/src/view.rs::process_input` returned early when the terminal lacks keyboard focus, before handling wheel events. This prevented scrolling an unfocused hovered terminal. The widget already sends wheel reports to applications that enable terminal mouse reporting; the 2026-09-14 validation entry explicitly did not exercise live agent/native GUI scrolling. The original focused-pane cause was not independently established; the follow-up live tests now verify focused and hovered navigation, streaming and reconnect as recorded above.

- [ ] Reproduce the history failure and record which GUI build is running, whether the pane is focused, terminal mouse/alternate-screen modes, and whether the problem affects the wheel, trackpad, or both. Use isolated sessions for instrumentation; do not log private conversation contents or restart the current session.
- [ ] Route wheel events by pointer location and visible interactive region, independently of keyboard focus. Only the hovered pane or scrollable control receives the event; scrolling must not change the typing destination. Respect menus, dialogs, overlays, and pane boundaries.
- [ ] Cover terminal splits, editor panes, Markdown preview/split views, Explorer, Agents, History, Settings, and overflowing tab strips. Preserve each view's intended scroll direction and existing image-preview wheel behavior. Prevent duplicate handling by nested/overlapping controls.
- [ ] Diagnose Codex history separately from hover routing: verify mouse-report delivery, pointer coordinates, alternate-screen behavior, retained terminal scrollback, and whether new output resets the viewport. Preserve application-owned history navigation where supported; do not substitute arrow-key input indiscriminately or promise recovery of history no longer retained.
- [ ] Add focused regressions for unfocused hovered panes, keyboard-focus preservation, two-pane isolation, occluding controls, trackpad accumulation, wheel direction, terminal mouse reporting, ordinary scrollback, and alternate-screen behavior. Document vendored changes in `vendor/egui_term/UPSTREAM.md`.
- [ ] Validate native split-pane scrolling and a real Codex history interaction. Confirm earlier messages remain readable while output arrives, scrolling down returns to recent output, and the current conversation/session survives. Record build identity and any unverified live-provider behavior in `docs/VALIDATION.md`.

**Acceptance:** Hover over a visible scrollable pane and scroll without clicking: that content moves and keyboard focus stays put. In Codex, earlier retained messages can be reached and read, then the latest messages can be reached again. A source fix or synthetic test alone does not close the reported live issue.

### 1. One-command local DMG builds

**Existing foundations:** `cargo xtask package --debug --timings --output DIR` already builds/packages all three binaries with resources, licenses, staging, and ad-hoc signing. `cargo xtask dmg --app APP --output DMG` already creates the branded DMG. Reuse these implementations.

- Add `cargo xtask local-dmg [--release] [--styled] [--output DIR] [--timings]`, reusing the existing package builder.
- Default to incremental debug builds for the current Mac architecture. Package all three executables, resources, and licenses with existing ad-hoc signing.
- Create a plain DMG using `hdiutil`, containing `Terminator.app` and an Applications shortcut. `--styled` uses the existing branded `create-dmg` workflow.
- Write each build to a unique directory under `target/local-dmg`; print artifact paths and build/package timings. Preserve previous successful artifacts when a build fails.
- Document installation and isolated testing. Building must not install the app or restart a running daemon.

**Acceptance:** One command produces an installable local DMG; a second unchanged build reuses compilation. Both plain and styled modes pass content/signature checks, and a failed attempt preserves previous successful artifacts.

### 2. Close idle terminals without confirmation

- Apply one close policy to pane buttons, top-level tabs, keyboard shortcuts, and sidebar actions. Idle shells end directly; running commands, background jobs, active agents, and uncertain state retain **Keep running / Terminate / Cancel**.
- Add a capability-gated daemon operation, `close-idle-sessions-v1`, accepting the daemon generation and target session IDs and returning typed close outcomes.
- Determine idleness from the actual PTY foreground process group, verified shell identity, descendant processes, and existing agent lifecycle records. Perform bounded inspection outside GUI rendering; never infer activity from terminal text.
- Preflight every session in a terminal-only tab before stopping any. Recheck immediately before signalling; preserve the view on failure or uncertainty. Deduplicate pending closes and retain the originating project/tab through asynchronous responses.
- Preserve existing editor save/discard handling and mixed editor/terminal confirmations. Closing the application continues to preserve daemon sessions. Older daemons retain the current confirmation behavior.
- Update repository guidance to document the idle-shell exception. This follows [Orca’s running-process confirmation model](https://github.com/stablyai/orca/blob/c6a72169843ececf3a21da370ac50c5c5a4e6462/src/renderer/src/components/terminal/running-terminal-close-guard.ts).

**Acceptance:** Verified idle shells close directly from every close entry point. Busy or uncertain sessions require a choice; failed or stale requests retain their views. Existing idle-daemon retirement is separate and must not be treated as shell-idleness detection.

### 3. First-launch folder access and recovery

- Show a compact macOS setup panel for installations without projects, offering **Choose project folder** and **Skip**. Persist dismissal in the existing UI preferences; provide access guidance again in Settings.
- Reuse the native folder picker and add appropriate protected-folder usage descriptions to packaged apps. macOS controls the resulting permission prompts. [Apple documentation](https://developer.apple.com/documentation/bundleresources/information-property-list/nsdocumentsfolderusagedescription)
- Replace swallowed directory-read failures with structured errors. Preserve cached entries with a visible error and provide **Retry**, **Choose folder again**, and privacy-settings guidance.
- Include optional Full Disk Access instructions without making it mandatory or claiming that ordinary access errors prove it is missing.
- Verify access separately through the GUI and daemon-owned shells/editors. Permission recovery must not terminate live sessions.

**Implementation order:** First replace `refresh.rs`'s directory-listing `unwrap_or_default()` with an explicit success/error result and retain the last successful listing. Handle individual directory-entry errors deliberately rather than silently filtering them out. Then add retry/reselection UI, first-run dismissal state, Settings guidance, and package metadata.

**Acceptance:** Denied or revoked access displays an actionable error instead of an apparently empty folder. Retry recovers without losing cached context or live sessions. Choosing a folder, cancelling the picker, skipping setup, and relaunching preserve the expected onboarding state.

### 4. Mac App Store feasibility report

- Produce a cited report mapping current features to sandboxing, helper-process access, persistent-background-process consent, hook installation, external tool dependencies, signing/submission, and update requirements.
- Explicitly assess Sparkle replacement, project access persistence, the private attachment helper, and user-installed Neovim/agent tools.
- Classify findings as confirmed requirements, implementation gaps, or unresolved review questions. Provide a sequenced roadmap and a recommendation on whether a separate Store edition merits prototyping.
- Deliver documentation only. Apple requires sandboxing and Store-distributed updates, making this more than repackaging the existing DMG. [App Review Guidelines §2.4.5](https://developer.apple.com/app-store/review/guidelines/#hardware-compatibility)

**Deliverable:** `docs/MAC_APP_STORE_FEASIBILITY.md`, with dated official sources, a feature/requirement/gap matrix, unresolved review questions, a sequenced prototype roadmap, and a recommendation. Recheck current Apple requirements during research; this plan is not the feasibility assessment. Store prototyping and submission are separate decisions.

### 5. Product Hunt launch kit

- Prepare listing copy, a maker comment, three suggested launch tags, FAQs, download/source links, and a posting checklist.
- Lead with persistent terminals, parallel agent work, and attention alerts. State that alerts require supported hooks and that agent subscriptions are separate.
- Supply a tagline within 60 characters, a conservative description within 260 characters, a 240×240 thumbnail, and four 1270×760 gallery images showing projects/splits, agent attention, editing/previews, and session reconnection. These fit Product Hunt’s published guidance. [Launch preparation](https://www.producthunt.com/launch/preparing-for-launch), [asset specifications](https://help.producthunt.com/en/articles/479557-how-to-post-a-product)
- Use native screenshots from isolated sample projects and the current branding assets. Verify every feature claim and download link before marking the kit ready. Deliver locally without scheduling or publishing.

**Deliverable:** A local `launch/product-hunt/` directory containing listing copy, maker comment, FAQ, tags, checklist, thumbnail, and four gallery images. Recheck current Product Hunt field/asset limits before export. Mark the kit ready only after links, claims, dimensions, and sample-data privacy have been checked.

## Validation

- **Scrolling:** unit/input regressions, isolated native hover/focus/overlay fixtures, and explicit Codex-history verification. Include mouse and trackpad input, multiple panes, new output while scrolled up, and supported platform coverage; record platform gaps separately.
- **Terminal lifecycle:** test idle shells, foreground commands, background jobs, agents, unknown inspection results, duplicate close actions, multiple splits, mixed editors, daemon-generation changes, and older-daemon compatibility.
- **File access:** test first-run dismissal, picker cancellation, denied/revoked access, cached-list errors, and successful retry. Validate real macOS permissions using an isolated account or VM.
- **Packaging:** build twice to check incremental reuse; verify DMG contents, executable permissions, signatures, paths containing spaces, and failure cleanup. Test styled mode separately.
- Run formatting, Clippy, workspace tests/builds, real-PTY integration checks, and affected native GUI fixtures. Record evidence and platform limitations in `docs/VALIDATION.md`.

## Delivery order and defaults

1. Diagnose and fix hovered-pane scrolling and Codex history access; validate the reported behavior before closing the issue.
2. Add the small local-DMG wrapper around existing packaging to simplify isolated testing.
3. Implement directory-read error recovery, then folder onboarding and permission guidance.
4. Implement safe idle-terminal closing with daemon capability gating and lifecycle regressions.
5. Prepare the Store feasibility report independently of the engineering changes.
6. Complete Product Hunt copy and capture final assets after the UI changes pass validation.

Track completion here with concrete evidence and remaining validation limits. Completed implementation is not proof that a currently running GUI or daemon has been updated.

Preserve concurrent branding edits and unrelated work. Native Rust remains the application platform; publishing, Store submission, and production installation are outside this implementation.
