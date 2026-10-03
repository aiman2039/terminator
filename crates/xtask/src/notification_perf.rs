//! Isolated A/B benchmark for the daemon notification path.
//!
//! Compares a baseline and a candidate `terminator-daemon` under a fixed set of
//! notification workloads. Each run gets a disposable data directory and an
//! explicit daemon executable, then reports daemon CPU time, peak RSS, IPC
//! snapshot latency, and submission counts. It never attaches to a running
//! generation and never touches the user's Notification Center beyond the
//! fixture-owned identifiers the daemon posts.
use crate::harness::{Harness, bin, output};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Baseline,
    Candidate,
}
impl Label {
    fn as_str(self) -> &'static str {
        match self {
            Label::Baseline => "baseline",
            Label::Candidate => "candidate",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    None,
    Unanswered,
    Replacements,
    Burst,
    IoPending,
}
impl Case {
    fn as_str(self) -> &'static str {
        match self {
            Case::None => "none",
            Case::Unanswered => "unanswered",
            Case::Replacements => "replacements",
            Case::Burst => "burst",
            Case::IoPending => "io_pending",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "none" => Case::None,
            "unanswered" => Case::Unanswered,
            "replacements" => Case::Replacements,
            "burst" => Case::Burst,
            "io_pending" => Case::IoPending,
            _ => return None,
        })
    }
}

pub struct Options {
    pub baseline_bin: Option<PathBuf>,
    pub candidate_bin: Option<PathBuf>,
    pub seconds: u64,
    pub warmup: u64,
    pub repetitions: u32,
    pub burst: usize,
    pub cases: String,
    pub output: Option<PathBuf>,
}

pub fn run(options: Options) -> Result<()> {
    ensure!(
        (1..=600).contains(&options.seconds),
        "Notification-perf duration must be 1-600 seconds"
    );
    ensure!(
        options.repetitions >= 1 && options.repetitions <= 10,
        "Repetitions must be 1-10"
    );
    ensure!(
        (1..=1000).contains(&options.burst),
        "Burst size must be 1-1000"
    );
    let cases = selected_cases(&options.cases)?;
    let baseline = options
        .baseline_bin
        .clone()
        .unwrap_or_else(|| bin().join("terminator-daemon"));
    let candidate = options
        .candidate_bin
        .clone()
        .unwrap_or_else(|| bin().join("terminator-daemon"));
    ensure!(
        baseline.is_file(),
        "baseline daemon missing: {} (build the workspace first)",
        baseline.display()
    );
    ensure!(
        candidate.is_file(),
        "candidate daemon missing: {}",
        candidate.display()
    );

    let mut runs = Vec::new();
    for repetition in 0..options.repetitions {
        for case in &cases {
            // Alternate order across repetitions so a warming host does not
            // systematically favor whichever binary runs first.
            let order = run_order(repetition);
            for label in order {
                let binary = match label {
                    Label::Baseline => &baseline,
                    Label::Candidate => &candidate,
                };
                eprintln!(
                    "notification-perf: {} {} (rep {repetition})",
                    case.as_str(),
                    label.as_str()
                );
                let run = measure(label, *case, repetition, binary, &options)
                    .with_context(|| format!("{} {} run failed", case.as_str(), label.as_str()))?;
                runs.push(run);
            }
        }
    }

    let aggregate = aggregate(&runs);
    let comparison = comparison(&runs, &cases);
    let report = json!({
        "generated_at": now_rfc3339(),
        "seconds": options.seconds,
        "warmup_seconds": options.warmup,
        "repetitions": options.repetitions,
        "burst": options.burst,
        "platform": std::env::consts::OS,
        "baseline_bin": baseline.to_string_lossy(),
        "candidate_bin": candidate.to_string_lossy(),
        "notifications_observable": cfg!(target_os = "macos"),
        "runs": runs,
        "aggregate": aggregate,
        "comparison": comparison,
    });
    if let Some(path) = &options.output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn selected_cases(spec: &str) -> Result<Vec<Case>> {
    if spec == "all" {
        return Ok(vec![
            Case::None,
            Case::Unanswered,
            Case::Replacements,
            Case::Burst,
            Case::IoPending,
        ]);
    }
    let mut cases = Vec::new();
    for name in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        cases.push(Case::parse(name).with_context(|| format!("Unknown case {name:?}"))?);
    }
    ensure!(!cases.is_empty(), "No notification-perf cases selected");
    Ok(cases)
}

/// Baseline first on even repetitions, candidate first on odd ones.
fn run_order(repetition: u32) -> [Label; 2] {
    if repetition.is_multiple_of(2) {
        [Label::Baseline, Label::Candidate]
    } else {
        [Label::Candidate, Label::Baseline]
    }
}

#[derive(serde::Serialize)]
struct Run {
    label: &'static str,
    case: &'static str,
    repetition: u32,
    cpu_ms: Option<f64>,
    rss_peak_kib: u64,
    ipc_p50_ms: Option<f64>,
    ipc_p95_ms: Option<f64>,
    snapshots: usize,
    notifications_submitted: usize,
    daemon_pid: u32,
    daemon_version: Option<String>,
    daemon_binary: String,
    daemon_binary_len: u64,
    daemon_binary_mtime: Option<u64>,
}

fn measure(
    label: Label,
    case: Case,
    repetition: u32,
    binary: &Path,
    options: &Options,
) -> Result<Run> {
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

    let total = Duration::from_secs(options.warmup.saturating_add(options.seconds));
    let started = Instant::now();
    let measure_after = Duration::from_secs(options.warmup);
    let mut cpu_start = None::<f64>;
    let mut rss_peak = 0u64;
    let mut ipc = Vec::new();
    let mut notifications_submitted = 0usize;
    let mut last_replacement = Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);
    let burst_due = matches!(case, Case::Burst);
    let mut burst_sent = false;

    while started.elapsed() < total {
        let measuring = started.elapsed() >= measure_after;
        // Workload.
        match case {
            Case::None | Case::IoPending => {}
            Case::Unanswered => {
                if !burst_sent && measuring {
                    h.terminal_notify(&session, "Agent: Waiting", "bench unanswered")?;
                    notifications_submitted = notifications_submitted.saturating_add(1);
                    burst_sent = true;
                }
            }
            Case::Replacements => {
                if measuring && last_replacement.elapsed() >= Duration::from_secs(2) {
                    h.terminal_notify(&session, "Agent: Waiting", "bench replacement")?;
                    notifications_submitted = notifications_submitted.saturating_add(1);
                    last_replacement = Instant::now();
                }
            }
            Case::Burst => {
                if measuring && burst_due && !burst_sent {
                    for _ in 0..options.burst {
                        h.terminal_notify(&session, "Agent: Waiting", "bench burst")?;
                        notifications_submitted = notifications_submitted.saturating_add(1);
                    }
                    burst_sent = true;
                }
            }
        }
        // IPC liveness: one snapshot round trip per iteration.
        let t = Instant::now();
        let _ = h.state()?;
        if measuring {
            ipc.push(t.elapsed().as_secs_f64() * 1000.0);
            if cpu_start.is_none() {
                cpu_start = sample_cpu_ms(daemon_pid);
            }
            if let Some(rss) = sample_rss_kib(daemon_pid) {
                rss_peak = rss_peak.max(rss);
            }
        }
        let _ = &io;
        thread::sleep(Duration::from_millis(100).saturating_sub(t.elapsed()));
    }

    let cpu_end = sample_cpu_ms(daemon_pid);
    let cpu_ms = match (cpu_start, cpu_end) {
        (Some(start), Some(end)) if start.is_finite() && end.is_finite() && end >= start => {
            Some(end - start)
        }
        _ => None,
    };
    ipc.sort_by(f64::total_cmp);
    let (daemon_binary, daemon_binary_len, daemon_binary_mtime) = binary_identity(binary);
    Ok(Run {
        label: label.as_str(),
        case: case.as_str(),
        repetition,
        cpu_ms,
        rss_peak_kib: rss_peak,
        ipc_p50_ms: finite(percentile(&ipc, 50.0)),
        ipc_p95_ms: finite(percentile(&ipc, 95.0)),
        snapshots: ipc.len(),
        notifications_submitted,
        daemon_pid,
        daemon_version,
        daemon_binary,
        daemon_binary_len,
        daemon_binary_mtime,
    })
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

pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let len = u32::try_from(sorted.len()).unwrap_or(u32::MAX);
    let rank = (p / 100.0 * (f64::from(len) - 1.0)).round();
    let rank = if rank.is_finite() && rank > 0.0 {
        format!("{rank:.0}").parse::<usize>().unwrap_or(usize::MAX)
    } else {
        0
    };
    let last = sorted.len().saturating_sub(1);
    sorted.get(rank.min(last)).copied().unwrap_or(f64::NAN)
}

fn aggregate(runs: &[Run]) -> Value {
    let mut out = serde_json::Map::new();
    for label in [Label::Baseline, Label::Candidate] {
        for case in [
            Case::None,
            Case::Unanswered,
            Case::Replacements,
            Case::Burst,
            Case::IoPending,
        ] {
            let matching: Vec<&Run> = runs
                .iter()
                .filter(|r| r.label == label.as_str() && r.case == case.as_str())
                .collect();
            if matching.is_empty() {
                continue;
            }
            let mut cpu: Vec<f64> = matching.iter().filter_map(|r| r.cpu_ms).collect();
            cpu.sort_by(f64::total_cmp);
            let ipc: Vec<f64> = matching.iter().filter_map(|r| r.ipc_p50_ms).collect();
            out.entry(label.as_str())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap()
                .insert(
                    case.as_str().into(),
                    json!({
                        "runs": matching.len(),
                        "cpu_ms_median": percentile(&cpu, 50.0),
                        "ipc_p50_ms_median": percentile(&ipc, 50.0),
                        "rss_peak_kib_max": matching.iter().map(|r| r.rss_peak_kib).max(),
                        "notifications_submitted": matching.iter().map(|r| r.notifications_submitted).sum::<usize>(),
                    }),
                );
        }
    }
    Value::Object(out)
}

fn comparison(runs: &[Run], cases: &[Case]) -> Value {
    let mut out = serde_json::Map::new();
    for case in cases {
        let cpu = |label: Label| -> Option<f64> {
            let mut values: Vec<f64> = runs
                .iter()
                .filter(|r| r.label == label.as_str() && r.case == case.as_str())
                .filter_map(|r| r.cpu_ms)
                .collect();
            values.sort_by(f64::total_cmp);
            (!values.is_empty()).then(|| percentile(&values, 50.0))
        };
        let baseline = cpu(Label::Baseline);
        let candidate = cpu(Label::Candidate);
        let ratio = match (baseline, candidate) {
            (Some(b), Some(c)) if b > 0.0 => Some(c / b),
            _ => None,
        };
        out.insert(
            case.as_str().into(),
            json!({
                "baseline_cpu_ms_median": baseline,
                "candidate_cpu_ms_median": candidate,
                "candidate_over_baseline": ratio,
            }),
        );
    }
    Value::Object(out)
}

fn now_rfc3339() -> String {
    // Avoid a time-format dependency: seconds since the epoch is enough to
    // order reports, and the harness never asserts on the timestamp.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_time_parses_ps_formats() {
        assert_eq!(parse_cpu_time("0:00.00"), Some(0.0));
        assert_eq!(parse_cpu_time("0:01.50"), Some(1500.0));
        assert_eq!(parse_cpu_time("1:02.25"), Some(62_250.0));
        assert_eq!(parse_cpu_time("2:03:04.00"), Some(7_384_000.0));
        assert_eq!(parse_cpu_time("1-00:00:01.00"), Some(86_401_000.0));
        assert_eq!(parse_cpu_time("  "), None);
        assert_eq!(parse_cpu_time("garbage"), None);
    }

    #[test]
    fn percentile_handles_empty_and_bounds() {
        assert!(percentile(&[], 50.0).is_nan());
        let sorted = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&sorted, 0.0), 1.0);
        assert_eq!(percentile(&sorted, 100.0), 4.0);
        assert_eq!(percentile(&sorted, 50.0), 3.0);
    }

    #[test]
    fn run_order_alternates_to_cancel_host_bias() {
        assert!(matches!(run_order(0), [Label::Baseline, Label::Candidate]));
        assert!(matches!(run_order(1), [Label::Candidate, Label::Baseline]));
        assert!(matches!(run_order(2), [Label::Baseline, Label::Candidate]));
    }

    #[test]
    fn case_selection_parses_names_and_rejects_unknown() {
        assert_eq!(selected_cases("all").unwrap().len(), 5);
        let subset = selected_cases("none, burst").unwrap();
        assert_eq!(subset, vec![Case::None, Case::Burst]);
        assert!(selected_cases("nope").is_err());
        assert!(selected_cases("").is_err());
    }

    #[test]
    fn comparison_reports_candidate_over_baseline() {
        let runs = vec![
            Run {
                label: "baseline",
                case: "none",
                repetition: 0,
                cpu_ms: Some(100.0),
                rss_peak_kib: 1,
                ipc_p50_ms: Some(1.0),
                ipc_p95_ms: Some(2.0),
                snapshots: 1,
                notifications_submitted: 0,
                daemon_pid: 1,
                daemon_version: None,
                daemon_binary: "baseline".into(),
                daemon_binary_len: 1,
                daemon_binary_mtime: None,
            },
            Run {
                label: "candidate",
                case: "none",
                repetition: 0,
                cpu_ms: Some(50.0),
                rss_peak_kib: 1,
                ipc_p50_ms: Some(1.0),
                ipc_p95_ms: Some(2.0),
                snapshots: 1,
                notifications_submitted: 0,
                daemon_pid: 2,
                daemon_version: None,
                daemon_binary: "candidate".into(),
                daemon_binary_len: 1,
                daemon_binary_mtime: None,
            },
        ];
        let value = comparison(&runs, &[Case::None]);
        assert_eq!(value["none"]["candidate_over_baseline"], 0.5);
    }
}
