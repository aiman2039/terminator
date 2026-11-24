//! Read-only Checks view for a flow run's worktree branch (#347): CI status,
//! PR comments (`gh`/`glab`, 20-second cap, gated on `fetch_pr_metadata`),
//! and added TODO/FIXME lines from `git diff`.
use crate::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{Receiver, TryRecvError, channel},
    time::Duration,
};

const CAP: Duration = Duration::from_secs(20);
const MAX_TODOS: usize = 15;

#[derive(Clone, Debug, Default)]
pub(crate) struct RunChecks {
    pub(crate) pr_number: Option<u64>,
    pub(crate) pr_url: Option<String>,
    pub(crate) pr_title: Option<String>,
    pub(crate) comments: usize,
    pub(crate) checks: Vec<(String, String)>,
    pub(crate) todos: Vec<String>,
    pub(crate) error: Option<String>,
}

#[derive(Default)]
pub(crate) struct ChecksState {
    loaded: HashMap<PathBuf, RunChecks>,
    loading: HashMap<PathBuf, Receiver<RunChecks>>,
}

impl ChecksState {
    pub(crate) fn poll(&mut self) {
        let mut done = Vec::new();
        for (path, rx) in &self.loading {
            match rx.try_recv() {
                Ok(checks) => done.push((path.clone(), Some(checks))),
                Err(TryRecvError::Disconnected) => done.push((path.clone(), None)),
                Err(TryRecvError::Empty) => {}
            }
        }
        for (path, checks) in done {
            self.loading.remove(&path);
            if let Some(checks) = checks {
                self.loaded.insert(path, checks);
            }
        }
    }

    pub(crate) fn loading(&self, path: &Path) -> bool {
        self.loading.contains_key(path)
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&RunChecks> {
        self.loaded.get(path)
    }
}

impl App {
    /// Start a replaceable read-only background read of CI checks, PR
    /// comments, and diff TODOs for `worktree`.
    pub(crate) fn request_flow_checks(&mut self, worktree: PathBuf) {
        let fetch_pr = self.state.settings.pr_metadata;
        let (tx, rx) = channel();
        let target = worktree.clone();
        if std::thread::Builder::new()
            .name("flow-checks".into())
            .spawn(move || {
                let _ = tx.send(collect_checks(&target, fetch_pr));
            })
            .is_ok()
        {
            self.flows.checks.loading.insert(worktree, rx);
        }
    }

    /// Render the Checks view for `worktree` when loaded or loading.
    pub(super) fn flow_checks_ui(&mut self, ui: &mut egui::Ui, worktree: &Path) {
        if self.flows.checks.loading(worktree) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Loading checks…");
            });
            ui.ctx().request_repaint_after(Duration::from_millis(200));
            return;
        }
        let Some(checks) = self.flows.checks.get(worktree).cloned() else {
            return;
        };
        egui::Frame::new()
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .corner_radius(appearance::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Checks");
                    if let Some(num) = checks.pr_number {
                        let label = format!("PR #{num} ({} comments)", checks.comments);
                        if let Some(url) = &checks.pr_url {
                            if ui
                                .link(&label)
                                .on_hover_text(checks.pr_title.as_deref().unwrap_or(url))
                                .clicked()
                            {
                                let _ = self.jobs.send(Job::Browser(url.clone()));
                            }
                        } else {
                            ui.weak(label);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if appearance::sidebar_action(ui, "RefreshCw", "Refresh checks").clicked() {
                            self.request_flow_checks(worktree.to_path_buf());
                        }
                    });
                });
                if let Some(error) = &checks.error {
                    ui.weak(error);
                }
                for (name, state) in &checks.checks {
                    ui.label(
                        egui::RichText::new(format!("• {name}: {state}"))
                            .size(appearance::FONT_CAPTION),
                    );
                }
                if !checks.todos.is_empty() {
                    ui.weak(format!("Diff TODOs ({}):", checks.todos.len()));
                    for line in &checks.todos {
                        ui.label(
                            egui::RichText::new(line)
                                .size(appearance::FONT_CAPTION)
                                .weak(),
                        );
                    }
                }
            });
    }
}

fn collect_checks(worktree: &Path, fetch_pr: bool) -> RunChecks {
    let mut out = RunChecks {
        todos: diff_todos(worktree),
        ..RunChecks::default()
    };
    if !fetch_pr {
        out.error = Some("Enable Fetch PR metadata in Settings for CI/PR checks.".into());
        return out;
    }
    let gitlab = origin_is_gitlab(worktree);
    let result = if gitlab {
        read_glab_mr(worktree)
    } else {
        read_gh_pr(worktree)
    };
    match result {
        Ok(pr) => {
            out.pr_number = pr.pr_number;
            out.pr_url = pr.pr_url;
            out.pr_title = pr.pr_title;
            out.comments = pr.comments;
            out.checks = pr.checks;
        }
        Err(error) => out.error = Some(error),
    }
    out
}

fn origin_is_gitlab(worktree: &Path) -> bool {
    let mut cmd = Command::new("git");
    cmd.args([
        "-C",
        &worktree.to_string_lossy(),
        "remote",
        "get-url",
        "origin",
    ]);
    terminator_core::run_command(
        cmd,
        terminator_core::CommandOptions {
            timeout: Duration::from_secs(5),
            ..terminator_core::CommandOptions::default()
        },
    )
    .ok()
    .filter(|o| o.status.success())
    .is_some_and(|o| {
        String::from_utf8_lossy(&o.stdout)
            .to_ascii_lowercase()
            .contains("gitlab")
    })
}

fn read_gh_pr(worktree: &Path) -> Result<RunChecks, String> {
    let mut cmd = Command::new("gh");
    cmd.current_dir(worktree).args([
        "pr",
        "view",
        "--json",
        "number,title,url,comments,statusCheckRollup",
    ]);
    let output = terminator_core::run_command(
        cmd,
        terminator_core::CommandOptions {
            timeout: CAP,
            ..terminator_core::CommandOptions::default()
        },
    )
    .map_err(|e| format!("gh: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let val: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("gh JSON: {e}"))?;
    let checks = val
        .get("statusCheckRollup")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|item| {
                    let name = item
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| item.get("context").and_then(serde_json::Value::as_str))
                        .unwrap_or("check")
                        .to_owned();
                    let status = item
                        .get("conclusion")
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                        .or_else(|| item.get("status").and_then(serde_json::Value::as_str))
                        .or_else(|| item.get("state").and_then(serde_json::Value::as_str))
                        .unwrap_or("pending")
                        .to_owned();
                    (name, status)
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(RunChecks {
        pr_number: val.get("number").and_then(serde_json::Value::as_u64),
        pr_url: val
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        pr_title: val
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        comments: val
            .get("comments")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len),
        checks,
        ..RunChecks::default()
    })
}

fn read_glab_mr(worktree: &Path) -> Result<RunChecks, String> {
    let mut cmd = Command::new("glab");
    cmd.current_dir(worktree).args(["mr", "view", "-F", "json"]);
    let output = terminator_core::run_command(
        cmd,
        terminator_core::CommandOptions {
            timeout: CAP,
            ..terminator_core::CommandOptions::default()
        },
    )
    .map_err(|e| format!("glab: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let val: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("glab JSON: {e}"))?;
    let mut checks = Vec::new();
    if let Some(status) = val
        .get("head_pipeline")
        .and_then(|p| p.get("status"))
        .and_then(serde_json::Value::as_str)
    {
        checks.push(("pipeline".into(), status.to_owned()));
    }
    Ok(RunChecks {
        pr_number: val.get("iid").and_then(serde_json::Value::as_u64),
        pr_url: val
            .get("web_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        pr_title: val
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        comments: usize::try_from(
            val.get("user_notes_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0),
        checks,
        ..RunChecks::default()
    })
}

fn diff_todos(worktree: &Path) -> Vec<String> {
    let mut cmd = Command::new("git");
    cmd.args(["-C", &worktree.to_string_lossy(), "diff", "-U0", "HEAD"]);
    let Ok(output) = terminator_core::run_command(
        cmd,
        terminator_core::CommandOptions {
            timeout: Duration::from_secs(5),
            ..terminator_core::CommandOptions::default()
        },
    ) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let added = line.strip_prefix('+')?;
            if added.starts_with("++") {
                return None;
            }
            let upper = added.to_ascii_uppercase();
            (upper.contains("TODO") || upper.contains("FIXME"))
                .then(|| added.trim().chars().take(120).collect::<String>())
        })
        .take(MAX_TODOS)
        .collect()
}
