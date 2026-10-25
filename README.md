<p align="center">
  <img src="crates/app/assets/branding/terminator-512.png" alt="Terminator app icon" width="128" height="128">
</p>

<h1 align="center">Terminator</h1>

<p align="center">Your terminals, AI agents, and project files in one place.<br>For macOS and Linux.</p>

<p align="center">
  <a href="https://github.com/aiman2039/terminator/actions/workflows/ci.yaml"><img src="https://github.com/aiman2039/terminator/actions/workflows/ci.yaml/badge.svg" alt="CI"></a>
  <a href="https://github.com/aiman2039/terminator/actions/workflows/security.yaml"><img src="https://github.com/aiman2039/terminator/actions/workflows/security.yaml/badge.svg" alt="Security"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://github.com/aiman2039/terminator/network/updates"><img src="https://img.shields.io/badge/dependabot-enabled-brightgreen?logo=dependabot" alt="Dependabot"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/aiman2039/terminator"><img src="https://api.scorecard.dev/projects/github.com/aiman2039/terminator/badge" alt="OpenSSF Scorecard"></a>
</p>

![Split terminals with project navigation and a file sidebar](docs/screenshots/split-file-opening.png)

**Keep your work together.** Organize projects, work side by side, and return to running terminals after closing the app.

## Features

- **Projects, tabs, and splits** — give each project its own workspace and arrange terminals side by side.
- **Keep work running** — close the app and reconnect to your terminals when you reopen it.
- **AI agent alerts** — see when an agent needs attention or finishes work, with supported agent hooks enabled.
- **Files and editing** — browse project files and open them in Neovim or your preferred editor.
- **Git changes** — see changed files and compare staged or working changes side by side.
- **Markdown previews** — read formatted documents or edit with a live preview beside your text.
- **Image previews** — open images and SVGs, zoom in, or fit them to the window.
- **Terminal search and history** — find text in terminal output and revisit past sessions.
- **Make it yours** — customize colors, fonts, shortcuts, and your shell.

## Get started

1. Follow the [installation and build guide](docs/REFERENCE.md#build-and-run).
2. Add a project folder using **+** beside **Projects**.
3. Open a terminal tab. Right-click a terminal to split your workspace.
4. Click a file to open it, or launch your usual tools in the terminal.

For AI agent alerts, open **Settings → Agent hooks**, enable your agent, then start a fresh agent session inside Terminator. Integrations include Claude Code, Codex, OpenCode, Muse, Grok, and Cursor; available alerts vary by agent.

Install Neovim for editing inside the app; Git comparisons need Neovim 0.10 or newer. You can also choose an external editor in Settings.

## A closer look

**Read and edit Markdown.** Switch between Edit, Preview, and Split views.

![Markdown preview with headings, a checklist, a table, code, and an image](docs/screenshots/markdown-header.png)

**Preview images.** Open an image from your project without leaving the workspace.

![Image preview with fit, actual-size, reload, and open-externally controls](docs/screenshots/reuse/image-preview-macos.png)

*Screenshots from native app tests using sample projects.*

[Setup and reference](docs/REFERENCE.md) · [Agent setup](docs/INTEGRATIONS.md) · [What's new](CHANGELOG.md) · [Development and testing](scripts/README.md)

### Perforce

The **Git / Perforce** sidebar detects Perforce folders when Git is absent.
Install the `p4` CLI on `PATH`, configure your workspace with `P4CONFIG` or the
usual `P4PORT`, `P4USER`, and `P4CLIENT` settings, and run `p4 login` in a terminal.
The app uses that configuration and ticket. It does not store credentials.

Open a folder inside the workspace. The sidebar shows opened files by changelist,
local changes from a read-only reconciliation preview, pending changelists, and
recent submissions for the current client. Click a changed file to compare the
synced **have revision** with the workspace file in the native diff viewer.
Perforce diffs are read-only; Git hunk actions are disabled.

Use file context menus to **Open for edit** or **Reconcile file**. The Explorer
also has these actions for unchanged files. The sidebar offers **Sync** for the
current folder. Select or enter a numbered changelist to **Shelve**, **Unshelve**
(into the default changelist), or **Submit**. Submit requires confirmation and
submits the complete numbered changelist, including files outside the open folder.
Create numbered changelists and edit their descriptions with `p4 change` in a
terminal. Revert, conflict resolution, stream switching, and shelf deletion also
use the terminal. Refresh never opens, reverts, syncs, or submits files.

The CLI and server must support tagged JSON output (`p4 -ztag -Mj`). Non-UTF-8
file names, symlink diffs, binary diffs, and diffs larger than 1 MiB are unsupported.
