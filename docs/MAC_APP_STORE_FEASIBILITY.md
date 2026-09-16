# Mac App Store feasibility

Assessment: 2026-09-14, Terminator 0.20.0 source checkout. This is a feasibility report, not a sandbox prototype, approval prediction, or submission.

**Recommendation:** keep direct distribution as the primary edition. Time-box a separate Store prototype before committing to feature parity. Persistent shells, arbitrary developer tools, and user-owned configuration are central to Terminator; restricted access would change those workflows substantially.

## Confirmed review requirements

Apple §2.4.5 requires appropriate sandboxing, a self-contained app packaged with Xcode technologies, consent for processes that survive quitting, and Store-delivered updates. It prohibits root escalation and installing code/resources in shared locations. A Store edition must remove Sparkle's update mechanism. These are distribution requirements, not evidence that this application will pass review. [App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/#hardware-compatibility)

## Feature and implementation matrix

The gaps below are findings or proposed engineering work from this repository, not additional Apple rulings.

| Feature | Relevant requirement / design concern | Current implementation and gap | Prototype acceptance |
| --- | --- | --- | --- |
| Sandboxing | Appropriate sandboxing | Native executables launch arbitrary shells; packaging has no Store entitlement configuration. | Signed sandbox build runs an isolated shell with a minimal, audited entitlement set. |
| Folder access | User-directed access outside the container | Project records persist filesystem paths. Picker recovery does not persist security-scoped bookmarks. | Choose, relaunch, revoke, reselect, and move a folder; resolve stale bookmarks without losing project identity. |
| Persistent daemon | Explicit background consent | GUI exit deliberately leaves daemon-owned PTYs running. No separate Store consent lifecycle exists. | Explain persistence before enabling it; consent denial and revocation have defined, non-destructive behavior. |
| Private helper | Self-contained distribution; sandbox/process boundaries | Daemon copies its helper to a private data directory so updates cannot break live sessions. | Validate signed helper execution, update survival, and access propagation under the chosen sandbox/service design. |
| Hook installation | Configuration owned by other tools | Explicit installers edit external configuration with backups. | User-selected access and supported configuration APIs work for every offered integration; unsupported integration remains disabled. |
| Neovim and agent tools | External executable and plugin access | Ordinary Neovim loads user configuration; bundled CodeDiff uses isolated snapshots; agents are user-launched. | Test executable access, dynamic libraries, plugins, tool credentials, subprocesses, and network access in a clean account. |
| Signing and packaging | Store submission pipeline | Local apps are ad-hoc signed; direct releases have their own signing and Sparkle assembly. | Separate Store bundle identity, entitlements, provisioning, archive, validation, and receipt handling if needed. |
| Updates | Store-only application updates | `package::assemble` embeds Sparkle; `updater` presents its controls. | Exclude Sparkle framework, feed keys, updater UI, and native callbacks from Store builds; retain daemon compatibility across Store replacements. |

Source evidence: `docs/ARCHITECTURE.md`, `docs/INTEGRATIONS.md`, `crates/daemon/src/shell.rs`, `crates/daemon/src/helper.rs`, `crates/xtask/src/package.rs`, and `crates/app/src/updater.rs`. Local package access descriptions do not confer sandbox entitlements or prove shell/editor access.

## Unresolved review and platform questions

- Can the proposed terminal/tool execution model operate within acceptable entitlements while retaining useful project access? Do not assume an unrestricted external terminal is an available sandbox escape.
- Which consented service design best preserves sessions after the GUI exits? Assess ServiceManagement against the supported macOS versions before choosing it.
- Can user-installed Neovim and agent binaries, their configuration and subprocesses work reliably without broad exceptions?
- Is the copied private helper acceptable for the final signed architecture, or must it remain bundled and communicate through a different service lifetime?
- Which hook configuration operations remain supportable under the selected access model?

Apple's [App Sandbox documentation](https://developer.apple.com/documentation/security/app-sandbox), [security-scoped URL API](https://developer.apple.com/documentation/foundation/url/startaccessingsecurityscopedresource()), and [SMAppService reference](https://developer.apple.com/documentation/servicemanagement/smappservice) are implementation starting points. Their full JavaScript documentation was not retrievable through the research tool in this run; exact API deployment requirements still need verification during the prototype.

## Sequenced roadmap

1. Create an isolated Store build configuration and bundle identity; exclude Sparkle and produce a signed sandbox archive. Make no changes to direct-distribution updates.
2. Implement bookmark-backed project access. Test GUI access separately from the daemon and its children across relaunch and revocation.
3. Prototype the consented daemon/helper lifetime with two live PTYs, GUI exit, app replacement, and reconnect. Record actual entitlement failures.
4. Test stock shells, ordinary Neovim configuration, bundled review snapshots, and each supported agent integration in a clean macOS account.
5. Document feature differences and ask Apple focused review questions using the working prototype. Decide whether the demonstrated workflow warrants a separate edition.

Stop the prototype if core tool access requires unsupported exceptions or if persistence cannot preserve user work. Store implementation, developer-account actions, and submission require a separate task.
