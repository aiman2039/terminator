use super::{Case, Label};
use serde_json::{Value, json};

#[derive(serde::Serialize)]
pub(super) struct Run {
    pub(super) label: &'static str,
    pub(super) case: &'static str,
    pub(super) repetition: u32,
    pub(super) cpu_ms: Option<f64>,
    pub(super) rss_peak_kib: u64,
    pub(super) ipc_p50_ms: Option<f64>,
    pub(super) ipc_p95_ms: Option<f64>,
    pub(super) snapshots: usize,
    pub(super) notifications_submitted: usize,
    pub(super) daemon_pid: u32,
    pub(super) daemon_version: Option<String>,
    pub(super) daemon_binary: String,
    pub(super) daemon_binary_len: u64,
    pub(super) daemon_binary_mtime: Option<u64>,
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

pub(super) fn aggregate(runs: &[Run]) -> Value {
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

pub(super) fn comparison(runs: &[Run], cases: &[Case]) -> Value {
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

pub(super) fn now_rfc3339() -> String {
    // Avoid a time-format dependency: seconds since the epoch is enough to
    // order reports, and the harness never asserts on the timestamp.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}
