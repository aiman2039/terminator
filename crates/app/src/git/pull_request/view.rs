//! Painting the PR tab: number and title (a link), state, review decision,
//! a checks summary, and the review threads. Read only.
use super::fetch::{Checks, PrInfo, PrState, Thread};
use crate::appearance;
use eframe::egui::{self, RichText};
use terminator_core::appearance::AppearanceConfig;

/// What a click in the PR tab asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Pick {
    /// The pull request page in the system browser.
    Open(String),
    /// The pull request page in a Browser tab.
    OpenInTab(String),
    Copy(String),
    /// A review thread's file, at its 1-based line.
    File(String, Option<u32>),
}

/// The Refresh icon row; true when Refresh was clicked.
pub(super) fn refresh_row(ui: &mut egui::Ui, loading: bool) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        let clicked = ui
            .add_enabled_ui(!loading, |ui| {
                appearance::sidebar_action(ui, "RefreshCw", "Refresh the pull request")
            })
            .inner
            .clicked();
        if loading {
            ui.spinner();
        }
        clicked
    })
    .inner
}

pub(super) fn body(ui: &mut egui::Ui, state: &PrState, theme: &AppearanceConfig) -> Option<Pick> {
    match state {
        PrState::Loading => {
            ui.weak("Loading the pull request…");
            None
        }
        PrState::NoPr => {
            ui.weak("This branch has no pull request. Create PR is in the Commit tab.");
            None
        }
        PrState::Hint(hint) => {
            ui.weak(hint);
            None
        }
        PrState::Failed(error) => {
            ui.colored_label(appearance::color(&theme.status_failed), error);
            None
        }
        PrState::Ready(info) => {
            let mut pick = None;
            appearance::sidebar_scroll("git-pull-request").show(ui, |ui| {
                pick = ready(ui, info, theme);
            });
            pick
        }
    }
}

fn ready(ui: &mut egui::Ui, info: &PrInfo, theme: &AppearanceConfig) -> Option<Pick> {
    let mut pick = None;
    let title = ui
        .add(egui::Link::new(format!("#{} {}", info.number, info.title)))
        .on_hover_text(format!("{}\nOpen in the browser", info.url));
    if title.clicked() {
        pick = Some(Pick::Open(info.url.clone()));
    }
    appearance::context_menu(&title, |ui| {
        if ui.button("Open in Browser tab").clicked() {
            pick = Some(Pick::OpenInTab(info.url.clone()));
            ui.close();
        }
        if ui.button("Copy link").clicked() {
            pick = Some(Pick::Copy(info.url.clone()));
            ui.close();
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(state_text(info)).strong());
        if let Some(review) = &info.review {
            ui.weak(review_text(review));
        }
    });
    checks(ui, &info.checks, theme);
    ui.separator();
    if let Some(thread) = threads(ui, info) {
        pick = Some(thread);
    }
    pick
}

/// `Open`, `Draft`, `Merged`, `Closed`.
fn state_text(info: &PrInfo) -> String {
    if info.draft {
        return "Draft".into();
    }
    let lower = info.state.to_ascii_lowercase();
    match lower.as_str() {
        "open" | "opened" => "Open".into(),
        "merged" => "Merged".into(),
        "closed" => "Closed".into(),
        _ => info.state.clone(),
    }
}

fn review_text(review: &str) -> &str {
    match review {
        "APPROVED" => "Approved",
        "CHANGES_REQUESTED" => "Changes requested",
        "REVIEW_REQUIRED" => "Review required",
        other => other,
    }
}

/// `Checks: 5 passed, 1 failed, 2 pending`, then the failed names.
pub(super) fn checks_text(checks: &Checks) -> Option<String> {
    let mut parts = Vec::new();
    for (count, word) in [
        (checks.passed, "passed"),
        (checks.failed, "failed"),
        (checks.pending, "pending"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {word}"));
        }
    }
    (!parts.is_empty()).then(|| format!("Checks: {}", parts.join(", ")))
}

fn checks(ui: &mut egui::Ui, checks: &Checks, theme: &AppearanceConfig) {
    let Some(text) = checks_text(checks) else {
        ui.weak("No checks");
        return;
    };
    let color = if checks.failed > 0 {
        appearance::color(&theme.status_failed)
    } else if checks.pending > 0 {
        appearance::color(&theme.status_waiting)
    } else {
        appearance::color(&theme.git_added)
    };
    ui.colored_label(color, text);
    for name in &checks.failing {
        ui.add(egui::Label::new(RichText::new(format!("✗ {name}")).weak()).truncate());
    }
}

/// Open threads first; each row opens its file at the line.
fn threads(ui: &mut egui::Ui, info: &PrInfo) -> Option<Pick> {
    let open = info
        .threads
        .iter()
        .filter(|thread| !thread.resolved)
        .count();
    let resolved = info.threads.len().saturating_sub(open);
    ui.strong(format!("Review threads: {open} open, {resolved} resolved"));
    if let Some(error) = &info.threads_error {
        ui.weak(format!("Threads could not be read: {error}"));
    }
    let mut pick = None;
    let mut sorted: Vec<&Thread> = info.threads.iter().collect();
    sorted.sort_by_key(|thread| thread.resolved);
    for thread in sorted {
        let place = match thread.line {
            Some(line) => format!("{}:{line}", thread.path),
            None => thread.path.clone(),
        };
        let mut text = RichText::new(format!("{place}  {}: {}", thread.author, thread.body));
        if thread.resolved {
            text = text.weak();
        }
        let row = ui
            .add(egui::Button::selectable(false, text).truncate())
            .on_hover_text(format!("{place}\n{}: {}", thread.author, thread.body));
        if row.clicked() && !thread.path.is_empty() {
            pick = Some(Pick::File(thread.path.clone(), thread.line));
        }
    }
    pick
}
