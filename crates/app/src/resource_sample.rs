//! One background sample of host, app, and session resource use.
//! The GUI thread only sends which session to follow; GUI/daemon/hook totals
//! ride along every sample so the status strip can show them without an extra
//! sampler.

use crate::Update;
use std::{
    collections::{HashMap, HashSet},
    panic::AssertUnwindSafe,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// While Info is open. The full process list is the costly read, so stay under 1 Hz.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub pid: Option<u32>,
    pub started: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionStats {
    pub cpu: f32,
    pub memory: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemStats {
    pub cpu: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub pressure: Option<terminator_sys::MemoryPressure>,
    pub load_one: f64,
    pub load_five: f64,
    pub load_fifteen: f64,
    pub cpus: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub pid: Option<u32>,
    pub started: u64,
    pub session: Option<SessionStats>,
    pub system: SystemStats,
    pub app: AppStats,
}

/// CPU and memory of one Terminator component across all its processes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ComponentStats {
    pub cpu: f32,
    pub memory: u64,
    pub processes: usize,
}

/// CPU and memory of the app's own processes: the GUI, the session-service
/// daemon, and hook helpers. Matched by executable name (see `classify`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AppStats {
    pub gui: ComponentStats,
    pub daemon: ComponentStats,
    pub hooks: ComponentStats,
}

impl AppStats {
    #[must_use]
    pub fn total(self) -> ComponentStats {
        ComponentStats {
            cpu: finite_cpu(self.gui.cpu + self.daemon.cpu + self.hooks.cpu),
            memory: self
                .gui
                .memory
                .saturating_add(self.daemon.memory)
                .saturating_add(self.hooks.memory),
            processes: self.gui.processes + self.daemon.processes + self.hooks.processes,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Component {
    Gui,
    Daemon,
    Hooks,
}

/// Which app component a process belongs to, by executable name.
/// The daemon prefix match also covers the 15-byte Linux `comm` truncation
/// (`terminator-daem…`); nothing else uses this prefix.
fn classify(name: &str) -> Option<Component> {
    if name == "terminator" {
        Some(Component::Gui)
    } else if name.strip_prefix("terminator-daem").is_some() {
        Some(Component::Daemon)
    } else if name.strip_prefix("terminator-hook").is_some() {
        Some(Component::Hooks)
    } else {
        None
    }
}

fn accumulate(stats: &mut ComponentStats, cpu: f32, memory: u64) {
    stats.cpu = finite_cpu(stats.cpu + finite_cpu(cpu));
    stats.memory = stats.memory.saturating_add(memory);
    stats.processes += 1;
}

#[derive(Clone, Copy, Debug)]
pub struct ProcNode {
    pub pid: u32,
    pub parent: Option<u32>,
    pub cpu: f32,
    pub memory: u64,
    pub started: u64,
}

/// CPU and memory of `pid` plus its descendants.
/// A non-zero `started` must match the root process within two seconds.
#[must_use]
pub fn tree_usage(nodes: &[ProcNode], pid: u32, started: u64) -> Option<SessionStats> {
    let root = nodes.iter().find(|node| node.pid == pid)?;
    if started != 0 && root.started.abs_diff(started) > 2 {
        return None;
    }
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for node in nodes {
        if let Some(parent) = node.parent {
            children.entry(parent).or_default().push(node.pid);
        }
    }
    let mut by_pid: HashMap<u32, &ProcNode> = HashMap::with_capacity(nodes.len());
    for node in nodes {
        by_pid.insert(node.pid, node);
    }
    let mut cpu = 0.0f32;
    let mut memory = 0u64;
    let mut stack = vec![pid];
    let mut seen = HashSet::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current) {
            continue;
        }
        if seen.len() > 4096 {
            break;
        }
        if let Some(node) = by_pid.get(&current) {
            cpu += finite_cpu(node.cpu);
            memory = memory.saturating_add(node.memory);
        }
        if let Some(kids) = children.get(&current) {
            stack.extend(kids.iter().copied());
        }
    }
    Some(SessionStats { cpu, memory })
}

fn finite_cpu(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

fn finite_load(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

pub fn spawn(
    updates: Sender<Update>,
    repaint: impl Fn() + Send + 'static,
) -> Sender<Option<Request>> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("terminator-resources".into())
        .spawn(move || sample_loop(rx, updates, repaint))
        .expect("spawn resource sampler");
    tx
}

fn sample_loop(rx: Receiver<Option<Request>>, updates: Sender<Update>, repaint: impl Fn()) {
    let mut system = System::new();
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| warmup(&mut system)));
    let mut current: Option<Request> = None;
    loop {
        let incoming = if current.is_none() {
            match rx.recv() {
                Ok(value) => value,
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(SAMPLE_INTERVAL) {
                Ok(value) => value,
                Err(mpsc::RecvTimeoutError::Timeout) => current.clone(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        };
        let Some(request) = incoming else {
            current = None;
            continue;
        };
        current = Some(request.clone());
        if let Some(sample) = take_sample(&mut system, &request) {
            if updates.send(Update::Resources(sample)).is_err() {
                return;
            }
            repaint();
        }
    }
}

fn warmup(system: &mut System) {
    system.refresh_cpu_usage();
    system.refresh_memory();
}

fn take_sample(system: &mut System, request: &Request) -> Option<Sample> {
    let request = request.clone();
    let sampled = std::panic::catch_unwind(AssertUnwindSafe(|| sample(system, &request)));
    match sampled {
        Ok(sample) => Some(sample),
        Err(_) => {
            *system = System::new();
            None
        }
    }
}

fn sample(system: &mut System, request: &Request) -> Sample {
    system.refresh_cpu_usage();
    system.refresh_memory();
    // One process-list read serves both the session tree and the app totals.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    let mut app = AppStats::default();
    let mut nodes = Vec::new();
    if request.pid.is_some() {
        nodes.reserve(system.processes().len());
    }
    for process in system.processes().values() {
        let cpu = process.cpu_usage();
        let memory = process.memory();
        match classify(&process.name().to_string_lossy()) {
            Some(Component::Gui) => accumulate(&mut app.gui, cpu, memory),
            Some(Component::Daemon) => accumulate(&mut app.daemon, cpu, memory),
            Some(Component::Hooks) => accumulate(&mut app.hooks, cpu, memory),
            None => {}
        }
        if request.pid.is_some() {
            nodes.push(ProcNode {
                pid: process.pid().as_u32(),
                parent: process.parent().map(Pid::as_u32),
                cpu,
                memory,
                started: process.start_time(),
            });
        }
    }
    let session = request
        .pid
        .and_then(|pid| tree_usage(&nodes, pid, request.started));
    let load = System::load_average();
    let cpu = system.global_cpu_usage();
    let counted = system.cpus().len();
    let cpus = if counted == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        counted
    };
    Sample {
        pid: request.pid,
        started: request.started,
        session,
        app,
        system: SystemStats {
            cpu: if cpu.is_finite() {
                cpu.clamp(0.0, 100.0)
            } else {
                0.0
            },
            memory_used: system.used_memory(),
            memory_total: system.total_memory(),
            pressure: terminator_sys::memory_pressure(),
            load_one: finite_load(load.one),
            load_five: finite_load(load.five),
            load_fifteen: finite_load(load.fifteen),
            cpus,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(pid: u32, parent: Option<u32>, cpu: f32, memory: u64) -> ProcNode {
        ProcNode {
            pid,
            parent,
            cpu,
            memory,
            started: 1_000,
        }
    }

    #[test]
    fn info_session_tree_sums_descendants_and_rejects_pid_reuse() {
        let nodes = [
            node(1, None, 1.0, 10),
            node(10, Some(1), 10.0, 100),
            node(11, Some(10), 5.0, 50),
            node(12, Some(11), 1.0, 10),
            node(20, Some(1), 99.0, 999),
        ];
        let usage = tree_usage(&nodes, 10, 1_000).unwrap();
        assert!((usage.cpu - 16.0).abs() < f32::EPSILON);
        assert_eq!(usage.memory, 160);
        assert!(tree_usage(&nodes, 10, 1_100).is_none());
        assert!(tree_usage(&nodes, 10, 0).is_some());
    }

    #[test]
    fn info_session_tree_stops_on_a_cycle() {
        let nodes = [
            ProcNode {
                pid: 1,
                parent: Some(2),
                cpu: 1.0,
                memory: 1,
                started: 5,
            },
            ProcNode {
                pid: 2,
                parent: Some(1),
                cpu: 1.0,
                memory: 1,
                started: 5,
            },
        ];
        let usage = tree_usage(&nodes, 1, 5).unwrap();
        assert!((usage.cpu - 2.0).abs() < f32::EPSILON);
        assert_eq!(usage.memory, 2);
    }

    #[test]
    fn app_classify_routes_gui_daemon_and_hooks() {
        assert_eq!(classify("terminator"), Some(Component::Gui));
        assert_eq!(classify("terminator-daemon"), Some(Component::Daemon));
        // 15-byte Linux `comm` truncation of the daemon binary.
        assert_eq!(classify("terminator-daem"), Some(Component::Daemon));
        assert_eq!(classify("terminator-hook"), Some(Component::Hooks));
        assert_eq!(classify("nvim"), None);
        assert_eq!(classify("terminator-something-else"), None);
    }

    #[test]
    fn app_total_sums_components() {
        let mut app = AppStats::default();
        accumulate(&mut app.gui, 2.5, 100);
        accumulate(&mut app.daemon, 1.5, 200);
        accumulate(&mut app.hooks, 0.5, 50);
        accumulate(&mut app.hooks, 0.5, 50);
        let total = app.total();
        assert!((total.cpu - 5.0).abs() < f32::EPSILON);
        assert_eq!(total.memory, 400);
        assert_eq!(total.processes, 4);
        assert_eq!(app.hooks.processes, 2);
    }
}
