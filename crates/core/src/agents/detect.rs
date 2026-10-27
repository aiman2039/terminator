use super::*;

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
    verified_detection(agents)
}

fn verified_detection(mut agents: Vec<DetectedAgent>) -> ShellDetection {
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
                parent: process.parent().map(sysinfo::Pid::as_u32),
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
