//! Shared agent catalog and live-presence observation types.
//!
//! Detection coverage (Codex, Claude Code, Pi, `OpenCode`, Grok, Muse, Cursor) is
//! separate from the six hook installers in `terminator-integrations`: Pi has
//! identity detection but no installer, and lifecycle reporting still requires
//! hooks for every agent. Process observations never create lifecycle events,
//! notifications, or resume commands.
use serde::{Deserialize, Serialize};

/// Daemon capability advertising live agent-presence observations.
pub const CAPABILITY: &str = "agent-presence-v1";
/// Background inspection cadence in seconds (daemon-side, never during GUI render).
pub const INSPECTION_INTERVAL_SECS: u64 = 2;
/// Observations older than this cannot claim verified presence.
pub const STALE_AFTER_SECS: u64 = 5;
/// Shell PID/start-time tolerance, matching idle-close identity checks.
pub const IDENTITY_TOLERANCE_SECS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentInfo {
    /// Hook `agent_kind` value, e.g. `codex`.
    pub kind: &'static str,
    /// Canonical display name, e.g. `Codex`.
    pub name: &'static str,
    /// GUI icon key, e.g. `AgentCodex`.
    pub icon: &'static str,
    /// Exact executable file names (no argument sniffing).
    pub exes: &'static [&'static str],
    /// Exact Node/Bun entrypoint basenames for `node`/`bun` launches.
    pub entrypoints: &'static [&'static str],
}

/// Muse Code conversation ids are UUID version 7 (`01…-…-7…`).
/// Tool and subagent hooks report a different id on the same terminal.
fn muse_conversation_id(id: &str) -> bool {
    let mut parts = id.split('-');
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(second) = parts.next() else {
        return false;
    };
    let Some(third) = parts.next() else {
        return false;
    };
    let Some(fourth) = parts.next() else {
        return false;
    };
    let Some(fifth) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && first.len() == 8
        && first.starts_with("01")
        && second.len() == 4
        && third.len() == 4
        && third.starts_with('7')
        && fourth.len() == 4
        && fifth.len() == 12
        && id
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
}

/// Agent record that should drive a terminal's lifecycle.
///
/// Muse tool and subagent events are separate records. Once a conversation id
/// exists, those records do not outrank it, so a later tool event cannot keep
/// the row on Working after Stop.
pub fn select_session_agent<'a>(
    agents: &'a [crate::Agent],
    session_id: &str,
) -> Option<&'a crate::Agent> {
    select_matched_agent(agents.iter().filter(|agent| agent.session_id == session_id))
}

/// Select the lifecycle record from agents that already belong to one session.
///
/// `matched` must be in the original slice order. Equal `updated` values keep
/// the later record. Muse tool and subagent ids never outrank a conversation id.
pub fn select_matched_agent<'a>(
    matched: impl IntoIterator<Item = &'a crate::Agent>,
) -> Option<&'a crate::Agent> {
    let matched: Vec<_> = matched.into_iter().collect();
    let has_conversation = matched.iter().any(|agent| {
        agent.kind == "muse"
            && agent
                .provider_session_id
                .as_deref()
                .is_some_and(muse_conversation_id)
    });
    matched
        .into_iter()
        .filter(|agent| {
            !has_conversation
                || agent.kind != "muse"
                || agent
                    .provider_session_id
                    .as_deref()
                    .is_some_and(muse_conversation_id)
        })
        .max_by_key(|agent| agent.updated)
}

/// First-release detection coverage. Custom hook agents receive a generic icon.
pub const CATALOG: &[AgentInfo] = &[
    AgentInfo {
        kind: "codex",
        name: "Codex",
        icon: "AgentCodex",
        exes: &["codex", "codex-bin"],
        entrypoints: &[
            "codex.js",
            "codex.mjs",
            "codex.cjs",
            "codex.ts",
            "codex.mts",
            "codex.cts",
        ],
    },
    AgentInfo {
        kind: "claude",
        name: "Claude Code",
        icon: "AgentClaude",
        exes: &["claude", "claude-bin"],
        entrypoints: &[
            "claude.js",
            "claude.mjs",
            "claude.cjs",
            "claude.ts",
            "claude.mts",
            "claude.cts",
        ],
    },
    AgentInfo {
        kind: "pi",
        name: "Pi",
        icon: "AgentPi",
        exes: &["pi", "pi-bin"],
        entrypoints: &[
            "pi.js",
            "pi.mjs",
            "pi.cjs",
            "pi.ts",
            "pi.mts",
            "pi.cts",
            "pi-agent.js",
            "pi-agent.ts",
        ],
    },
    AgentInfo {
        kind: "opencode",
        name: "OpenCode",
        icon: "AgentOpencode",
        exes: &["opencode", "opencode-bin"],
        entrypoints: &[
            "opencode.js",
            "opencode.mjs",
            "opencode.cjs",
            "opencode.ts",
            "opencode.mts",
            "opencode.cts",
        ],
    },
    AgentInfo {
        kind: "grok",
        name: "Grok",
        icon: "AgentGrok",
        exes: &["grok", "grok-bin"],
        entrypoints: &[
            "grok.js", "grok.mjs", "grok.cjs", "grok.ts", "grok.mts", "grok.cts",
        ],
    },
    AgentInfo {
        kind: "muse",
        name: "Muse Code",
        icon: "AgentMuse",
        exes: &["muse", "muse-bin"],
        entrypoints: &[
            "muse.js", "muse.mjs", "muse.cjs", "muse.ts", "muse.mts", "muse.cts",
        ],
    },
    AgentInfo {
        kind: "cursor",
        name: "Cursor",
        icon: "AgentCursor",
        // Bare `agent` is also Grok's shim. The Node launcher is matched by
        // its namespaced installation path below.
        exes: &["cursor-agent"],
        entrypoints: &[],
    },
];

/// Generic fallback for custom hook agents and multi-agent terminals.
pub const GENERIC_ICON: &str = "AgentGeneric";
pub const MULTIPLE_ICON: &str = "AgentMultiple";

/// JavaScript runtimes whose first script argument may be a known entrypoint.
const SCRIPT_RUNTIMES: &[&str] = &["node", "bun"];
/// Shells whose first script argument may be a catalog executable. Muse's
/// installer is `#!/usr/bin/env bash` and stays `bash` until it execs
/// `muse-bin-<version>`. The script basename is the agent; command strings
/// (`sh -c "…"`) are not.
const SHELL_LAUNCHERS: &[&str] = &["sh", "bash", "zsh", "dash", "fish"];

#[must_use]
pub fn catalog(kind: &str) -> Option<&'static AgentInfo> {
    CATALOG.iter().find(|info| info.kind == kind)
}

/// Shared CLI lookup for the Settings status and hook installation gate.
#[must_use]
pub fn find_installed(kind: &str) -> Option<std::path::PathBuf> {
    let info = catalog(kind)?;
    info.exes
        .iter()
        .find_map(|name| crate::find_executable(name))
        .or_else(|| {
            (kind == "cursor")
                .then(|| crate::find_executable("agent"))
                .flatten()
                .filter(|path| {
                    path.canonicalize().is_ok_and(|resolved| {
                        match_agent(&resolved.to_string_lossy(), &[]) == Some("cursor")
                    })
                })
        })
}

#[must_use]
pub fn display_name(kind: &str) -> &str {
    catalog(kind).map_or(kind, |info| info.name)
}

#[must_use]
pub fn icon_key(kind: &str) -> &'static str {
    catalog(kind).map_or(GENERIC_ICON, |info| info.icon)
}

/// Exact exe name or a `-<digit>...` version suffix after a known exe.
fn match_exe(basename: &str) -> Option<&'static str> {
    CATALOG
        .iter()
        .find(|info| {
            info.exes.contains(&basename)
                || info.exes.iter().any(|known| {
                    known
                        .len()
                        .checked_add(1)
                        .is_some_and(|end| basename.len() > end)
                        && basename.starts_with(*known)
                        && basename.as_bytes().get(known.len()) == Some(&b'-')
                        && basename
                            .as_bytes()
                            .get(known.len().saturating_add(1))
                            .is_some_and(u8::is_ascii_digit)
                })
        })
        .map(|info| info.kind)
}

/// Match one process by executable file name, known Node/Bun entrypoint, or
/// shell-script basename. `argv` is the full command line; only `argv[0]`
/// (the invoked program), the first non-flag argument after a `node`/`bun`
/// executable, and the first script path after a shell are considered, so
/// arbitrary arguments, titles, output, and working directories never match.
///
/// Launcher-managed binaries carry a version suffix (e.g.
/// `muse-bin-1.4.1-R4503.1`, `grok-1.0.44-macos-aarch64`). A trailing
/// `-<digit>...` after a known exe also matches, so `codex-helper` and
/// similar names still do not match. Symlink-resolved executables whose file
/// name is a bare version (e.g. Claude's `versions/2.1.283`) match through
/// the `argv[0]` invocation instead.
#[must_use]
pub fn match_agent(exe_file_name: &str, argv: &[&str]) -> Option<&'static str> {
    let exe = exe_file_name.rsplit('/').next().unwrap_or(exe_file_name);
    if let Some(kind) = match_exe(exe) {
        return Some(kind);
    }
    // Version-resolved `exe` (e.g. `2.1.283`) with a recognizable invocation.
    if let Some(invoked) = argv.first() {
        let invoked = invoked.rsplit('/').next().unwrap_or(invoked);
        if invoked != exe
            && let Some(kind) = match_exe(invoked)
        {
            return Some(kind);
        }
    }
    if SHELL_LAUNCHERS.contains(&exe) {
        return match_shell_script(argv);
    }
    if !SCRIPT_RUNTIMES.contains(&exe) {
        return None;
    }
    let script = argv.iter().skip(1).find(|arg| !arg.starts_with('-'))?;
    if cursor_entrypoint(script) {
        return Some("cursor");
    }
    let entry = script.rsplit('/').next().unwrap_or(script);
    CATALOG
        .iter()
        .find(|info| info.entrypoints.contains(&entry))
        .map(|info| info.kind)
}

/// Cursor's `agent` launcher execs Node with a generic `index.js`. Only the
/// versioned Cursor installation layout identifies that entrypoint.
fn cursor_entrypoint(script: &str) -> bool {
    let path = std::path::Path::new(script);
    let Some(version) = path.parent() else {
        return false;
    };
    path.is_absolute()
        && path.file_name().is_some_and(|name| name == "index.js")
        && version.file_name().is_some_and(|name| {
            name.to_str()
                .and_then(|name| name.as_bytes().first())
                .is_some_and(u8::is_ascii_digit)
        })
        && version.parent().is_some_and(|versions| {
            versions.file_name().is_some_and(|name| name == "versions")
                && versions
                    .parent()
                    .and_then(std::path::Path::file_name)
                    .is_some_and(|name| name == "cursor-agent")
        })
}

/// First script path after a shell launcher. Flags and nested interpreters
/// are skipped. A whitespace command string is a `-c` body, not an executable.
fn match_shell_script(argv: &[&str]) -> Option<&'static str> {
    for arg in argv.iter().skip(1).filter(|arg| !arg.starts_with('-')) {
        if arg.contains(char::is_whitespace) {
            return None;
        }
        let base = arg.rsplit('/').next().unwrap_or(arg);
        if SHELL_LAUNCHERS.contains(&base) || SCRIPT_RUNTIMES.contains(&base) {
            continue;
        }
        return match_exe(base);
    }
    None
}

/// PID/start-time identity. A reused PID has a different start time and never
/// matches an earlier observation.
mod detect;
pub use detect::{
    DetectAgentsInput, DetectedAgent, PresenceOutcome, ProcView, ProcessIdentity, ShellDetection,
    ShellTarget, TerminalPresence, detect_agents, inspect_shells, observation_fresh,
    preferred_agent, presence_verified, snapshot_processes,
};

#[cfg(test)]
mod tests;
