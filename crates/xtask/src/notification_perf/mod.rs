//! Isolated A/B benchmark for the daemon notification path.
//!
//! Compares a baseline and a candidate `terminator-daemon` under a fixed set of
//! notification workloads. Each run gets a disposable data directory and an
//! explicit daemon executable, then reports daemon CPU time, peak RSS, IPC
//! snapshot latency, and submission counts. It never attaches to a running
//! generation and never touches the user's Notification Center beyond the
//! fixture-owned identifiers the daemon posts.
mod measure;
mod report;
#[cfg(test)]
mod tests;

use crate::harness::bin;
use anyhow::{Context, Result, ensure};
pub use measure::parse_cpu_time;
use measure::{MeasureInput, measure};
#[cfg(test)]
use report::percentile;
use report::{Run, aggregate, comparison, now_rfc3339};
use serde_json::json;
use std::{fs, path::PathBuf};

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

#[derive(clap::Args)]
pub struct Options {
    /// Baseline daemon binary (defaults to the built debug daemon).
    #[arg(long)]
    pub baseline_bin: Option<PathBuf>,
    /// Candidate daemon binary (defaults to the built debug daemon).
    #[arg(long)]
    pub candidate_bin: Option<PathBuf>,
    #[arg(long, default_value = "60")]
    pub seconds: u64,
    #[arg(long, default_value = "10")]
    pub warmup: u64,
    #[arg(long, default_value = "3")]
    pub repetitions: u32,
    #[arg(long, default_value = "20")]
    pub burst: usize,
    #[arg(long, default_value = "all")]
    pub cases: String,
    #[arg(long)]
    pub output: Option<PathBuf>,
}
pub fn run(options: Options) -> Result<()> {
    validate(&options)?;
    let cases = selected_cases(&options.cases)?;
    let (baseline, candidate) = binaries(&options)?;
    let runs = measure_all(&options, &cases, (&baseline, &candidate))?;
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

fn validate(options: &Options) -> Result<()> {
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
    Ok(())
}

/// The baseline and candidate daemon executables, defaulting to the built debug daemon.
fn binaries(options: &Options) -> Result<(PathBuf, PathBuf)> {
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
    Ok((baseline, candidate))
}

fn measure_all(
    options: &Options,
    cases: &[Case],
    (baseline, candidate): (&PathBuf, &PathBuf),
) -> Result<Vec<Run>> {
    let mut runs = Vec::new();
    for repetition in 0..options.repetitions {
        for case in cases {
            // Alternate order across repetitions so a warming host does not
            // systematically favor whichever binary runs first.
            let order = run_order(repetition);
            for label in order {
                let binary = match label {
                    Label::Baseline => baseline,
                    Label::Candidate => candidate,
                };
                eprintln!(
                    "notification-perf: {} {} (rep {repetition})",
                    case.as_str(),
                    label.as_str()
                );
                let run = measure(MeasureInput {
                    label,
                    case: *case,
                    repetition,
                    binary,
                    options,
                })
                .with_context(|| format!("{} {} run failed", case.as_str(), label.as_str()))?;
                runs.push(run);
            }
        }
    }
    Ok(runs)
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
