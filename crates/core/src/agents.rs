//! Shared agent catalog and live-presence observation types.
//!
//! Detection coverage (Codex, Claude Code, Pi, OpenCode, Grok, Muse) is
//! separate from the five hook installers in `terminator-integrations`: Pi has
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
];

/// Generic fallback for custom hook agents and multi-agent terminals.
pub const GENERIC_ICON: &str = "AgentGeneric";
pub const MULTIPLE_ICON: &str = "AgentMultiple";

/// JavaScript runtimes whose first script argument may be a known entrypoint.
const SCRIPT_RUNTIMES: &[&str] = &["node", "bun"];

#[must_use]
pub fn catalog(kind: &str) -> Option<&'static AgentInfo> {
    CATALOG.iter().find(|info| info.kind == kind)
}

#[must_use]
pub fn display_name(kind: &str) -> &str {
    catalog(kind).map_or(kind, |info| info.name)
}

#[must_use]
pub fn icon_key(kind: &str) -> &'static str {
    catalog(kind).map_or(GENERIC_ICON, |info| info.icon)
}

/// Match one process by exact executable file name, or by known Node/Bun
/// entrypoint. `argv` is the full command line; only the first non-flag
/// argument after a `node`/`bun` executable is considered, so arbitrary
/// arguments, titles, output, and working directories never match.
#[must_use]
pub fn match_agent(exe_file_name: &str, argv: &[&str]) -> Option<&'static str> {
    let exe = exe_file_name.rsplit('/').next().unwrap_or(exe_file_name);
    if let Some(info) = CATALOG.iter().find(|info| info.exes.contains(&exe)) {
        return Some(info.kind);
    }
    if !SCRIPT_RUNTIMES.contains(&exe) {
        return None;
    }
    let entry = argv
        .iter()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .map(|arg| arg.rsplit('/').next().unwrap_or(arg))?;
    CATALOG
        .iter()
        .find(|info| info.entrypoints.contains(&entry))
        .map(|info| info.kind)
}

/// PID/start-time identity. A reused PID has a different start time and never
/// matches an earlier observation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PresenceOutcome {
    #[default]
    Unavailable,
    Verified,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedAgent {
    pub kind: String,
    pub process: ProcessIdentity,
    #[serde(default)]
    pub foreground: bool,
}

/// Additive per-terminal presence observation. Never persisted and never a
/// lifecycle event source.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalPresence {
    pub session_id: String,
    pub generation: String,
    #[serde(default)]
    pub agents: Vec<DetectedAgent>,
    #[serde(default)]
    pub outcome: PresenceOutcome,
    pub observed_at: u64,
}

#[must_use]
pub fn observation_fresh(observed_at: u64, now: u64) -> bool {
    now.saturating_sub(observed_at) <= STALE_AFTER_SECS
}

#[must_use]
pub fn presence_verified(presence: Option<&TerminalPresence>, now: u64) -> bool {
    presence.is_some_and(|p| {
        p.outcome == PresenceOutcome::Verified && observation_fresh(p.observed_at, now)
    })
}

/// Renderer-independent process view used by [`detect_agents`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcView {
    pub pid: u32,
    pub parent: Option<u32>,
    pub start_time: u64,
    pub exe_name: String,
    pub argv: Vec<String>,
    /// False for terminated-but-unreaped processes, which have exited and
    /// never claim live presence.
    pub live: bool,
}

/// Outcome of one batched inspection pass for a single owned shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellDetection {
    pub outcome: PresenceOutcome,
    pub agents: Vec<DetectedAgent>,
}

/// Detect agent roots beneath one owned shell.
///
/// Only descendants of the verified shell (PID/start-time identity) are
/// considered. An agent keeps its identity while it runs child tools: nested
/// agent processes beneath an already recognized root are ignored, while
/// independently launched sibling agents each produce an entry.
pub fn detect_agents(
    procs: &[ProcView],
    shell_pid: u32,
    shell_start_time: u64,
    foreground_pgid: Option<u32>,
) -> ShellDetection {
    let shell = procs.iter().find(|p| p.pid == shell_pid);
    let Some(shell) = shell else {
        return ShellDetection {
            outcome: PresenceOutcome::Unavailable,
            agents: Vec::new(),
        };
    };
    if !shell.live {
        return ShellDetection {
            outcome: PresenceOutcome::Unavailable,
            agents: Vec::new(),
        };
    }
    if shell.start_time.abs_diff(shell_start_time) > IDENTITY_TOLERANCE_SECS {
        return ShellDetection {
            outcome: PresenceOutcome::Unavailable,
            agents: Vec::new(),
        };
    }
    let by_pid: std::collections::HashMap<u32, &ProcView> =
        procs.iter().map(|p| (p.pid, p)).collect();
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for proc in procs {
        if let Some(parent) = proc.parent {
            children.entry(parent).or_default().push(proc.pid);
        }
    }
    // Depth-first from the shell; the first recognized ancestor claims the
    // subtree so nested agents beneath a root are ignored for this release.
    let mut agents = Vec::new();
    let mut stack = vec![shell_pid];
    let mut claimed_roots: std::collections::HashSet<u32> = std::collections::HashSet::new();
    while let Some(pid) = stack.pop() {
        let Some(members) = children.get(&pid) else {
            continue;
        };
        let mut members = members.clone();
        members.sort_unstable();
        for member in members {
            let Some(proc) = by_pid.get(&member) else {
                continue;
            };
            if !proc.live {
                continue;
            }
            if claimed_roots.contains(&pid) {
                // Beneath an agent root: retain the root, ignore the subtree.
                continue;
            }
            let argv: Vec<&str> = proc.argv.iter().map(String::as_str).collect();
            if let Some(kind) = match_agent(&proc.exe_name, &argv) {
                // Foreground programs lead their own process group, so the
                // PTY foreground pgid equals the agent root PID. sysinfo
                // exposes no job-control pgid; PID equality is the check.
                let foreground = foreground_pgid.is_some_and(|fg| proc.pid == fg);
                agents.push(DetectedAgent {
                    kind: kind.to_string(),
                    process: ProcessIdentity {
                        pid: proc.pid,
                        start_time: proc.start_time,
                    },
                    foreground,
                });
                claimed_roots.insert(proc.pid);
            } else {
                stack.push(proc.pid);
            }
        }
    }
    agents.sort_by(|a, b| {
        b.foreground
            .cmp(&a.foreground)
            .then_with(|| a.process.pid.cmp(&b.process.pid))
    });
    ShellDetection {
        outcome: PresenceOutcome::Verified,
        agents,
    }
}

/// Preferred agent for a terminal icon: the foreground-associated agent, or
/// the single remaining agent. Multiple agents without a foreground process
/// use the generic multiple-agent indicator instead.
#[must_use]
pub fn preferred_agent(agents: &[DetectedAgent]) -> Option<&DetectedAgent> {
    agents
        .iter()
        .find(|a| a.foreground)
        .or_else(|| agents.first().filter(|_| agents.len() == 1))
}

/// One owned live shell to inspect in a batched pass.
#[derive(Clone, Debug)]
pub struct ShellTarget {
    pub session_id: String,
    pub generation: String,
    pub pid: u32,
    /// Recorded `Session.created` for PID/start-time identity.
    pub start_time: u64,
    /// PTY foreground process group, when the terminal reports one.
    pub foreground_pgid: Option<u32>,
}

/// One batched process inventory for [`inspect_shells`]. The caller refreshes
/// a single `sysinfo::System` per pass. Only executable names and the Node/Bun
/// entrypoint position are retained; later command arguments are dropped here
/// so they can never match or leak into diagnostics.
#[must_use]
pub fn snapshot_processes(system: &sysinfo::System) -> Vec<ProcView> {
    system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let exe_name = process
                .exe()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| process.name().to_string_lossy().into_owned());
            let mut argv = Vec::new();
            for (index, arg) in process.cmd().iter().enumerate() {
                let arg = arg.to_string_lossy().into_owned();
                let entrypoint = index > 0 && !arg.starts_with('-');
                argv.push(arg);
                if entrypoint {
                    break;
                }
            }
            ProcView {
                pid: pid.as_u32(),
                parent: process.parent().map(|pid| pid.as_u32()),
                start_time: process.start_time(),
                exe_name,
                argv,
                live: process.status() != sysinfo::ProcessStatus::Zombie,
            }
        })
        .collect()
}

/// Inspect every owned shell against one shared inventory. Exactly one pass
/// per interval; the daemon runs this on a background thread, never during
/// GUI rendering.
#[must_use]
pub fn inspect_shells(
    procs: &[ProcView],
    targets: &[ShellTarget],
    observed_at: u64,
) -> Vec<TerminalPresence> {
    targets
        .iter()
        .map(|target| {
            let found = detect_agents(procs, target.pid, target.start_time, target.foreground_pgid);
            TerminalPresence {
                session_id: target.session_id.clone(),
                generation: target.generation.clone(),
                agents: found.agents,
                outcome: found.outcome,
                observed_at,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, parent: Option<u32>, exe: &str) -> ProcView {
        ProcView {
            pid,
            parent,
            start_time: 1000 + u64::from(pid),
            exe_name: exe.into(),
            argv: vec![format!("/usr/local/bin/{exe}")],
            live: true,
        }
    }

    #[test]
    fn six_catalog_matchers_cover_exact_executables() {
        for (kind, exe) in [
            ("codex", "codex"),
            ("claude", "claude"),
            ("pi", "pi"),
            ("opencode", "opencode"),
            ("grok", "grok"),
            ("muse", "muse"),
        ] {
            assert_eq!(match_agent(exe, &[exe]), Some(kind), "{exe}");
            assert_eq!(
                match_agent(&format!("/usr/local/bin/{exe}"), &[exe]),
                Some(kind),
                "path {exe}"
            );
        }
        assert_eq!(CATALOG.len(), 6);
    }

    #[test]
    fn wrappers_match_exact_names_and_known_entrypoints() {
        assert_eq!(match_agent("codex-bin", &["codex-bin"]), Some("codex"));
        assert_eq!(
            match_agent("node", &["node", "/opt/claude.js"]),
            Some("claude")
        );
        assert_eq!(
            match_agent("bun", &["bun", "--smol", "dist/pi-agent.js"]),
            Some("pi")
        );
        assert_eq!(
            match_agent("node", &["node", "/opt/opencode.mjs"]),
            Some("opencode")
        );
    }

    #[test]
    fn arbitrary_arguments_titles_and_directories_never_match() {
        // Agent names in arguments, flags, or script bodies are not identity.
        assert_eq!(match_agent("sh", &["sh", "-c", "codex resume abc"]), None);
        assert_eq!(match_agent("node", &["node", "server.js", "codex"]), None);
        assert_eq!(
            match_agent("node", &["node", "/home/user/my-codex-server.js"]),
            None
        );
        assert_eq!(match_agent("python3", &["python3", "claude.py"]), None);
        assert_eq!(match_agent("codex-helper", &["codex-helper"]), None);
        assert_eq!(match_agent("mycodex", &["mycodex"]), None);
        // Only the first non-flag argument is examined: a known entrypoint
        // later on the command line never matches.
        assert_eq!(
            match_agent("node", &["node", "server.js", "claude.js"]),
            None
        );
    }

    #[test]
    fn child_tools_keep_the_root_identity_and_nested_agents_are_ignored() {
        let procs = vec![
            proc(100, None, "zsh"),
            proc(200, Some(100), "codex"),
            // Child tools of the agent, including a nested agent binary.
            proc(201, Some(200), "rg"),
            proc(202, Some(200), "claude"),
            proc(203, Some(202), "node"),
        ];
        let found = detect_agents(&procs, 100, 1100, None);
        assert_eq!(found.outcome, PresenceOutcome::Verified);
        assert_eq!(found.agents.len(), 1);
        assert_eq!(found.agents[0].kind, "codex");
        assert_eq!(found.agents[0].process.pid, 200);
    }

    #[test]
    fn independently_launched_sibling_agents_each_produce_an_entry() {
        let procs = vec![
            proc(100, None, "zsh"),
            proc(200, Some(100), "codex"),
            proc(300, Some(100), "claude"),
        ];
        let found = detect_agents(&procs, 100, 1100, None);
        assert_eq!(found.outcome, PresenceOutcome::Verified);
        assert_eq!(found.agents.len(), 2);
        // No foreground process: no single preferred agent.
        assert!(preferred_agent(&found.agents).is_none());
    }

    #[test]
    fn foreground_association_prefers_one_agent_for_the_icon() {
        let procs = vec![
            proc(100, None, "zsh"),
            proc(200, Some(100), "codex"),
            proc(300, Some(100), "claude"),
        ];
        let found = detect_agents(&procs, 100, 1100, Some(300));
        assert_eq!(preferred_agent(&found.agents).unwrap().kind, "claude");
        let single = detect_agents(&procs[..2], 100, 1100, None);
        assert_eq!(preferred_agent(&single.agents).unwrap().kind, "codex");
    }

    #[test]
    fn pid_reuse_and_missing_shells_cannot_claim_verified_presence() {
        let procs = vec![proc(100, None, "zsh"), proc(200, Some(100), "codex")];
        // Same PID, recycled start time: shell identity changed.
        let stale = detect_agents(&procs, 100, 9999, None);
        assert_eq!(stale.outcome, PresenceOutcome::Unavailable);
        assert!(stale.agents.is_empty());
        // Shell gone entirely.
        let gone = detect_agents(&procs, 999, 1999, None);
        assert_eq!(gone.outcome, PresenceOutcome::Unavailable);
    }

    #[test]
    fn exit_removes_live_identity_on_the_next_successful_inspection() {
        let before = vec![proc(100, None, "zsh"), proc(200, Some(100), "pi")];
        assert_eq!(detect_agents(&before, 100, 1100, None).agents.len(), 1);
        let after = vec![proc(100, None, "zsh")];
        let found = detect_agents(&after, 100, 1100, None);
        assert_eq!(found.outcome, PresenceOutcome::Verified);
        assert!(found.agents.is_empty());
    }

    #[test]
    fn terminated_but_unreaped_processes_have_already_exited() {
        let mut zombie = proc(200, Some(100), "codex");
        zombie.live = false;
        let procs = vec![proc(100, None, "zsh"), zombie];
        let found = detect_agents(&procs, 100, 1100, None);
        assert_eq!(found.outcome, PresenceOutcome::Verified);
        assert!(found.agents.is_empty());
        let mut dead_shell = proc(100, None, "zsh");
        dead_shell.live = false;
        let gone = detect_agents(&[dead_shell], 100, 1100, None);
        assert_eq!(gone.outcome, PresenceOutcome::Unavailable);
    }

    #[test]
    fn observations_older_than_five_seconds_are_not_verified_presence() {
        let presence = TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: vec![DetectedAgent {
                kind: "codex".into(),
                process: ProcessIdentity {
                    pid: 1,
                    start_time: 1,
                },
                foreground: false,
            }],
            outcome: PresenceOutcome::Verified,
            observed_at: 100,
        };
        assert!(presence_verified(Some(&presence), 105));
        assert!(!presence_verified(Some(&presence), 106));
        let unavailable = TerminalPresence {
            outcome: PresenceOutcome::Unavailable,
            observed_at: 105,
            ..presence.clone()
        };
        assert!(!presence_verified(Some(&unavailable), 105));
        assert!(!presence_verified(None, 105));
    }

    #[test]
    fn custom_kinds_fall_back_to_the_generic_icon() {
        assert_eq!(icon_key("codex"), "AgentCodex");
        assert_eq!(icon_key("my-agent"), GENERIC_ICON);
        assert_eq!(display_name("my-agent"), "my-agent");
        assert_eq!(display_name("muse"), "Muse Code");
    }
}
