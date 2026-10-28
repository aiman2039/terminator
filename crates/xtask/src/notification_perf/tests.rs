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
