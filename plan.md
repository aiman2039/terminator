# Terminator usability and launch preparation

## Summary

Implement the three application improvements from `plan.md`, then prepare a Mac App Store feasibility report and a local Product Hunt launch kit.

Decisions confirmed: idle shells end without confirmation; first launch offers folder setup; local DMGs default to fast debug builds; Product Hunt presents Terminator as a free, open-source workspace for AI-agent developers.

## Implementation

### 1. One-command local DMG builds

- Add `cargo xtask local-dmg [--release] [--styled] [--output DIR] [--timings]`, reusing the existing package builder.
- Default to incremental debug builds for the current Mac architecture. Package all three executables, resources, and licenses with existing ad-hoc signing.
- Create a plain DMG using `hdiutil`, containing `Terminator.app` and an Applications shortcut. `--styled` uses the existing branded `create-dmg` workflow.
- Write each build to a unique directory under `target/local-dmg`; print artifact paths and build/package timings. Preserve previous successful artifacts when a build fails.
- Document installation and isolated testing. Building must not install the app or restart a running daemon.

### 2. Close idle terminals without confirmation

- Apply one close policy to pane buttons, top-level tabs, keyboard shortcuts, and sidebar actions. Idle shells end directly; running commands, background jobs, active agents, and uncertain state retain **Keep running / Terminate / Cancel**.
- Add a capability-gated daemon operation, `close-idle-sessions-v1`, accepting the daemon generation and target session IDs and returning typed close outcomes.
- Determine idleness from the actual PTY foreground process group, verified shell identity, descendant processes, and existing agent lifecycle records. Perform bounded inspection outside GUI rendering; never infer activity from terminal text.
- Preflight every session in a terminal-only tab before stopping any. Recheck immediately before signalling; preserve the view on failure or uncertainty. Deduplicate pending closes and retain the originating project/tab through asynchronous responses.
- Preserve existing editor save/discard handling and mixed editor/terminal confirmations. Closing the application continues to preserve daemon sessions. Older daemons retain the current confirmation behavior.
- Update repository guidance to document the idle-shell exception. This follows [Orca’s running-process confirmation model](https://github.com/stablyai/orca/blob/c6a72169843ececf3a21da370ac50c5c5a4e6462/src/renderer/src/components/terminal/running-terminal-close-guard.ts).

### 3. First-launch folder access and recovery

- Show a compact macOS setup panel for installations without projects, offering **Choose project folder** and **Skip**. Persist dismissal in the existing UI preferences; provide access guidance again in Settings.
- Reuse the native folder picker and add appropriate protected-folder usage descriptions to packaged apps. macOS controls the resulting permission prompts. [Apple documentation](https://developer.apple.com/documentation/bundleresources/information-property-list/nsdocumentsfolderusagedescription)
- Replace swallowed directory-read failures with structured errors. Preserve cached entries with a visible error and provide **Retry**, **Choose folder again**, and privacy-settings guidance.
- Include optional Full Disk Access instructions without making it mandatory or claiming that ordinary access errors prove it is missing.
- Verify access separately through the GUI and daemon-owned shells/editors. Permission recovery must not terminate live sessions.

### 4. Mac App Store feasibility report

- Produce a cited report mapping current features to sandboxing, helper-process access, persistent-background-process consent, hook installation, external tool dependencies, signing/submission, and update requirements.
- Explicitly assess Sparkle replacement, project access persistence, the private attachment helper, and user-installed Neovim/agent tools.
- Classify findings as confirmed requirements, implementation gaps, or unresolved review questions. Provide a sequenced roadmap and a recommendation on whether a separate Store edition merits prototyping.
- Deliver documentation only. Apple requires sandboxing and Store-distributed updates, making this more than repackaging the existing DMG. [App Review Guidelines §2.4.5](https://developer.apple.com/app-store/review/guidelines/#hardware-compatibility)

### 5. Product Hunt launch kit

- Prepare listing copy, a maker comment, three suggested launch tags, FAQs, download/source links, and a posting checklist.
- Lead with persistent terminals, parallel agent work, and attention alerts. State that alerts require supported hooks and that agent subscriptions are separate.
- Supply a tagline within 60 characters, a conservative description within 260 characters, a 240×240 thumbnail, and four 1270×760 gallery images showing projects/splits, agent attention, editing/previews, and session reconnection. These fit Product Hunt’s published guidance. [Launch preparation](https://www.producthunt.com/launch/preparing-for-launch), [asset specifications](https://help.producthunt.com/en/articles/479557-how-to-post-a-product)
- Use native screenshots from isolated sample projects and the current branding assets. Verify every feature claim and download link before marking the kit ready. Deliver locally without scheduling or publishing.

## Validation

- **Terminal lifecycle:** test idle shells, foreground commands, background jobs, agents, unknown inspection results, duplicate close actions, multiple splits, mixed editors, daemon-generation changes, and older-daemon compatibility.
- **File access:** test first-run dismissal, picker cancellation, denied/revoked access, cached-list errors, and successful retry. Validate real macOS permissions using an isolated account or VM.
- **Packaging:** build twice to check incremental reuse; verify DMG contents, executable permissions, signatures, paths containing spaces, and failure cleanup. Test styled mode separately.
- Run formatting, Clippy, workspace tests/builds, real-PTY integration checks, and affected native GUI fixtures. Record evidence and platform limitations in `docs/VALIDATION.md`.

## Delivery order and defaults

Build local-DMG tooling first, then terminal closing and folder access; capture launch assets after those changes pass validation. Prepare the Store report independently.

Preserve concurrent branding edits and unrelated work. Native Rust remains the application platform; publishing, Store submission, and production installation are outside this implementation.
