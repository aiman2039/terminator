# Product Hunt launch kit

Status: prepared locally for review. Copy, links, dimensions, and sample-data privacy were checked on 2026-09-15; see `validation.json`. Product runtime acceptance and the posting checklist remain launch gates. Nothing has been posted.

Use `copy.json` for the product name, tagline, description, suggested topics and links. The app is MIT licensed (`LICENSE`); it does not include paid agent services. Recheck current downloadable release availability before posting.

## Maker comment

I built Terminator to keep project terminals, coding agents, and file editing in one native workspace.

Each project has its own tabs and splits. Neovim handles editing, native previews make files easier to inspect, and sessions keep running when the GUI closes so you can reconnect later. The Agents view brings supported hook notifications together when a tool needs attention.

Terminator is free and open source, built in Rust for macOS and Linux. Bring your own tools and agent accounts; supported hooks are required for agent alerts, and agent subscriptions are separate.

I'd like to hear which part of your multi-project terminal workflow is hardest to keep organized. Source and downloads are linked above.

## FAQ

- **Is it free?** Terminator is free and MIT licensed. External tools and agent services have their own terms and pricing.
- **Does it run agents automatically?** No. Launch agents yourself in a terminal. Install supported hooks explicitly in Settings to receive their lifecycle alerts.
- **What survives closing the window?** Daemon-owned sessions remain running. Reconnecting attaches to those sessions; rebooted or ended processes are not automatically restarted.
- **Which editor does it use?** Neovim by default, using your configuration for ordinary editing. Install Neovim separately. Git review uses bundled CodeDiff with read-only snapshots.
- **Which platforms?** Native macOS and Linux. The local macOS DMG is ad-hoc signed; do not describe a local development artifact as a notarized public release.
- **Is it in the Mac App Store?** No Store submission is part of this launch kit.

## Asset specification

Use the existing Terminator mark for `thumbnail.png` (240×240). Final gallery images are 1270×760, in this order: `01-projects-splits.png`, `02-agent-attention.png`, `03-editing-previews.png`, `04-reconnection.png`. Capture isolated sample projects only, after affected UI fixtures pass. Do not fabricate live-provider activity or use private terminal content.

Dimensions and the 260-character description limit were checked against [Product Hunt's posting guidance](https://help.producthunt.com/en/articles/479557-how-to-post-a-product) on 2026-09-14. The requested tagline ceiling is 60 characters.

## Posting checklist

- [ ] Complete scrolling, access-recovery and idle-close validation gates in `docs/VALIDATION.md`.
- [x] Capture and visually inspect all four native sample-project gallery images.
- [x] Verify exact dimensions, readable text, no private paths/messages, and current branding.
- [ ] Open source and download links; confirm the public release's version, architecture and signing status.
- [x] Verify tagline ≤60 (50) and description ≤260 (227) characters from `copy.json`.
- [ ] Verify the current Product Hunt topic choices and pricing/status selections in the posting form.
- [ ] Review maker identity and copy; post only through an explicitly authorized action.
- [ ] Test installation and reconnect on the actual downloadable build before announcing availability.

The public source and download links returned HTTP 200; the download link resolved to v0.20.0. Release signing and installation must still be checked on the downloadable release before posting. The gallery uses the `atlas-web` and `atlas-api` sample projects; the attention scene is explicitly a synthetic hook event, and no provider was launched. Regenerate with `cargo xtask gui launch` after a test-support build, then `cargo xtask launch-assets`.
