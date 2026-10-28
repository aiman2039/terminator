use super::report::{Run, percentile};
use super::{Case, Label, Options};
use crate::harness::{Harness, output};
use anyhow::Result;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub(super) struct MeasureInput<'a> {
    pub(super) label: Label,
    pub(super) case: Case,
    pub(super) repetition: u32,
    pub(super) binary: &'a Path,
    pub(super) options: &'a Options,
}

/// The notification workload's progress across polling iterations.
struct Workload<'a> {
    h: &'a Harness,
    session: &'a str,
    case: Case,
    burst: usize,
    notifications_submitted: usize,
    last_replacement: Instant,
    burst_sent: bool,
}

/// Daemon measurements gathered while the workload runs.
#[derive(Default)]
struct Samples {
    cpu_start: Option<f64>,
    rss_peak: u64,
    ipc: Vec<f64>,
}

pub(super) fn measure(input: MeasureInput<'_>) -> Result<Run> {
    let MeasureInput {
        label,
        case,
        repetition,
        binary,
        options,
    } = input;
    let h = Harness::configured(Some(binary.to_owned()), true)?;
    h.setup()?;
    h.enable_os_notifications()?;
    let project = h.project("notify-perf")?;
    let shell = h.shell(&project)?;
    let session = crate::harness::id(&shell).to_owned();

    let mut io = None;
    if case == Case::IoPending {
        let mut stream = h.attach(&shell)?;
        h.write(
            &mut stream,
            "i=0; while [ $i -lt 100000 ]; do printf 'io sample %s\\n' \"$i\"; i=$((i+1)); sleep 0.05; done\n",
        )?;
        io = Some(stream);
    }

    let daemon_pid = h.daemon.as_ref().map(|p| p.0.id()).unwrap_or(0);
    let daemon_version = h.state().ok().and_then(|state| {
        state
            .get("daemon_version")
            .and_then(Value::as_str)
            .or_else(|| state.get("daemon_build").and_then(Value::as_str))
            .map(str::to_owned)
    });
    let mut workload = Workload::new(&h, &session, case, options.burst);
    let mut samples = run_workload(&mut workload, options, daemon_pid, &io)?;
    let cpu_ms = cpu_delta(samples.cpu_start, sample_cpu_ms(daemon_pid));
    samples.ipc.sort_by(f64::total_cmp);
    let (daemon_binary, daemon_binary_len, daemon_binary_mtime) = binary_identity(binary);
    Ok(Run {
        label: label.as_str(),
        case: case.as_str(),
        repetition,
        cpu_ms,
        rss_peak_kib: samples.rss_peak,
        ipc_p50_ms: finite(percentile(&samples.ipc, 50.0)),
        ipc_p95_ms: finite(percentile(&samples.ipc, 95.0)),
        snapshots: samples.ipc.len(),
        notifications_submitted: workload.notifications_submitted,
        daemon_pid,
        daemon_version,
        daemon_binary,
        daemon_binary_len,
        daemon_binary_mtime,
    })
}

/// Drive the workload for warmup plus measurement, sampling the daemon every 100 ms.
fn run_workload<T>(
    workload: &mut Workload<'_>,
    options: &Options,
    daemon_pid: u32,
    io: &Option<T>,
) -> Result<Samples> {
    let h = workload.h;
    let total = Duration::from_secs(options.warmup.saturating_add(options.seconds));
    let started = Instant::now();
    let measure_after = Duration::from_secs(options.warmup);
    let mut samples = Samples::default();
    while started.elapsed() < total {
        let measuring = started.elapsed() >= measure_after;
        workload.step(measuring)?;
        // IPC liveness: one snapshot round trip per iteration.
        let t = Instant::now();
        let _ = h.state()?;
        if measuring {
            samples.ipc.push(t.elapsed().as_secs_f64() * 1000.0);
            if samples.cpu_start.is_none() {
                samples.cpu_start = sample_cpu_ms(daemon_pid);
            }
            if let Some(rss) = sample_rss_kib(daemon_pid) {
                samples.rss_peak = samples.rss_peak.max(rss);
            }
        }
        let _ = io;
        thread::sleep(Duration::from_millis(100).saturating_sub(t.elapsed()));
    }
    Ok(samples)
}

fn cpu_delta(cpu_start: Option<f64>, cpu_end: Option<f64>) -> Option<f64> {
    match (cpu_start, cpu_end) {
        (Some(start), Some(end)) if start.is_finite() && end.is_finite() && end >= start => {
            Some(end - start)
        }
        _ => None,
    }
}

impl<'a> Workload<'a> {
    fn new(h: &'a Harness, session: &'a str, case: Case, burst: usize) -> Self {
        Self {
            h,
            session,
            case,
            burst,
            notifications_submitted: 0,
            last_replacement: Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(Instant::now),
            burst_sent: false,
        }
    }
    fn notify(&mut self, body: &str) -> Result<()> {
        self.h
            .terminal_notify(self.session, "Agent: Waiting", body)?;
        self.notifications_submitted = self.notifications_submitted.saturating_add(1);
        Ok(())
    }
    /// Submit this iteration's notifications for the selected case.
    fn step(&mut self, measuring: bool) -> Result<()> {
        let burst_due = matches!(self.case, Case::Burst);
        match self.case {
            Case::None | Case::IoPending => {}
            Case::Unanswered => {
                if !self.burst_sent && measuring {
                    self.notify("bench unanswered")?;
                    self.burst_sent = true;
                }
            }
            Case::Replacements => {
                if measuring && self.last_replacement.elapsed() >= Duration::from_secs(2) {
                    self.notify("bench replacement")?;
                    self.last_replacement = Instant::now();
                }
            }
            Case::Burst => {
                if measuring && burst_due && !self.burst_sent {
                    for _ in 0..self.burst {
                        self.notify("bench burst")?;
                    }
                    self.burst_sent = true;
                }
            }
        }
        Ok(())
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

/// Identity of the exact executable that ran, so a report can never be
/// attributed to the wrong build.
fn binary_identity(path: &Path) -> (String, u64, Option<u64>) {
    let meta = fs::metadata(path).ok();
    let len = meta.as_ref().map(std::fs::Metadata::len).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    (path.to_string_lossy().into_owned(), len, mtime)
}

/// Parse `ps -o time=` output (`MM:SS.ss`, `HH:MM:SS.ss`, or `D-HH:MM:SS.ss`)
/// into milliseconds.
pub fn parse_cpu_time(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (days, rest) = match raw.split_once('-') {
        Some((days, rest)) => (days.parse::<f64>().ok()?, rest),
        None => (0.0, raw),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [h, m, s] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            s.parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    Some((((days * 24.0 + hours) * 60.0 + minutes) * 60.0 + seconds) * 1000.0)
}

fn sample_cpu_ms(pid: u32) -> Option<f64> {
    if pid == 0 {
        return None;
    }
    let mut command = Command::new("ps");
    command.args(["-o", "time=", "-p", &pid.to_string()]);
    let raw = output(command).ok()?;
    parse_cpu_time(&String::from_utf8_lossy(&raw))
}

fn sample_rss_kib(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    let mut command = Command::new("ps");
    command.args(["-o", "rss=", "-p", &pid.to_string()]);
    let raw = output(command).ok()?;
    String::from_utf8_lossy(&raw).trim().parse::<u64>().ok()
}
