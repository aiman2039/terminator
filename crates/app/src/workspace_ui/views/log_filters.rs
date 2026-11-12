//! The Commit Log filter bar: Text, Branch, User, Date, and Path filters,
//! Go to Hash, and Reload. Typed edits wait for a pause, then a background
//! query loads the log. The shown log stays until the reply comes.
use super::super::super::*;
use crate::git_log::{Answer, CommitLog, LogBranch, LogDate, LogView};
use std::time::Instant;

const TEXT_WIDTH: f32 = 170.0;
const PATH_WIDTH: f32 = 130.0;
const GOTO_WIDTH: f32 = 110.0;
const MENU_HEIGHT: f32 = 320.0;

/// What the bar asked for this frame.
#[derive(Default)]
struct BarAsk {
    reload: bool,
    goto: bool,
}

impl App {
    /// The filter bar and its status line; sends the loads they ask for.
    pub(super) fn log_filter_bar(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let key = tab.key();
        let view = self.git.logs.entry(key.clone()).or_default();
        let ask = filter_bar(ui, view, &key, Instant::now());
        let shown = match self.git_views.get(&key) {
            Some(GitView::Log(log)) => Some(log),
            _ => None,
        };
        status_line(ui, view, shown, &self.theme);
        if ask.goto {
            self.go_to_hash(tab, &key);
        }
        self.send_log_loads(ui, tab, ask.reload);
    }

    /// The first load, Reload, and filter edits once typing pauses.
    fn send_log_loads(&mut self, ui: &egui::Ui, tab: &Tab, reload: bool) {
        let key = tab.key();
        let now = Instant::now();
        let Some(view) = self.git.logs.get_mut(&key) else {
            return;
        };
        let first = !self.git_views.contains_key(&key);
        let query = if first || reload {
            Some(view.reload())
        } else {
            view.due(now)
        };
        if let Some(wait) = view.wait(now) {
            ui.ctx().request_repaint_after(wait);
        }
        let Some(query) = query else {
            return;
        };
        if first {
            self.git_views.insert(key, GitView::Loading);
        }
        let _ = self.jobs.send(Job::GitLog(tab.clone(), query));
    }

    /// Go to Hash: select a loaded commit, or load the log with it.
    pub(crate) fn go_to_hash(&mut self, tab: &Tab, key: &str) {
        let Some(view) = self.git.logs.get_mut(key) else {
            return;
        };
        let rev = match crate::revision_compare::typed_revision(&view.goto) {
            Ok(rev) => rev.to_owned(),
            Err(note) => {
                view.goto_note = Some(note);
                return;
            }
        };
        let loaded = match self.git_views.get(key) {
            Some(GitView::Log(log)) => log.find_loaded(&rev).map(|commit| commit.hash.clone()),
            _ => None,
        };
        if let Some(hash) = loaded {
            view.picked(&hash);
            self.selected_commit = Some(hash);
            return;
        }
        let query = view.find(rev);
        // This query is also the first load when no log is on screen.
        self.git_views
            .entry(key.to_owned())
            .or_insert(GitView::Loading);
        let _ = self.jobs.send(Job::GitLog(tab.clone(), query));
    }

    /// A Commit Log reply: the view to show, or `None` to keep the shown one.
    pub(crate) fn log_answer(&mut self, key: &str, log: CommitLog) -> Option<GitView> {
        let Some(view) = self.git.logs.get_mut(key) else {
            return Some(GitView::Log(log));
        };
        let showing = matches!(
            self.git_views.get(key),
            Some(GitView::Log(shown)) if shown.error.is_none()
        );
        match view.answer(&log, showing) {
            Answer::Stale | Answer::KeepShown => None,
            Answer::Show(found) => {
                let listed = |hash: &String| log.commits.iter().any(|c| &c.hash == hash);
                if found.is_some() {
                    self.selected_commit = found;
                } else if !self.selected_commit.as_ref().is_some_and(listed) {
                    // The details pane then shows the first commit.
                    self.selected_commit = None;
                }
                Some(GitView::Log(log))
            }
        }
    }

    /// A Commit Log job ended without a reply: it was cancelled or failed.
    /// The bar says why, because the list no longer matches its filters.
    pub(crate) fn log_job_ended(&mut self, key: &str, generation: u64, error: Option<String>) {
        let Some(view) = self.git.logs.get_mut(key) else {
            return;
        };
        let error = error.unwrap_or_else(|| "Loading stopped. Click Reload.".into());
        let Some(slot) = self.git_views.get_mut(key) else {
            view.ended(generation, None);
            return;
        };
        // With no log on screen, the error shows in its place, not twice.
        let first = matches!(slot, GitView::Loading);
        if !view.ended(generation, (!first).then(|| error.clone())) || !first {
            return;
        }
        *slot = GitView::Log(CommitLog {
            error: Some(error),
            ..CommitLog::default()
        });
    }
}

/// Text, Branch, User, Date, and Path filters, then Go to Hash and Reload.
fn filter_bar(ui: &mut egui::Ui, view: &mut LogView, salt: &str, now: Instant) -> BarAsk {
    let mut ask = BarAsk::default();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        let text = ui
            .add(
                appearance::singleline(&mut view.draft.text)
                    .id_salt((salt, "log-text"))
                    .hint_text("Message or hash")
                    .desired_width(TEXT_WIDTH),
            )
            .on_hover_text("Commits with this text in the message (any case), or a commit hash");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "log-text", text.rect);
        edited(ui, &text, view, now);
        branch_menu(ui, view);
        user_menu(ui, view);
        date_menu(ui, view);
        let path = ui
            .add(
                appearance::singleline(&mut view.draft.path)
                    .id_salt((salt, "log-path"))
                    .hint_text("Path")
                    .desired_width(PATH_WIDTH),
            )
            .on_hover_text(
                "Only commits that change this file or folder (from the repository root)",
            );
        edited(ui, &path, view, now);
        if !view.draft.is_empty()
            && appearance::sidebar_action(ui, "Eraser", "Clear filters").clicked()
        {
            view.clear();
        }
        if view.loading {
            ui.spinner();
        }
        ui.separator();
        let goto = ui
            .add(
                appearance::singleline(&mut view.goto)
                    .id_salt((salt, "log-goto"))
                    .hint_text("Go to hash")
                    .desired_width(GOTO_WIDTH),
            )
            .on_hover_text("A commit hash, branch, or tag. Press Enter.");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "log-goto", goto.rect);
        let locate = appearance::sidebar_action(ui, "LocateFixed", "Go to hash").clicked();
        ask.goto = locate || submitted(ui, &goto);
        ask.reload = appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked();
    });
    ask
}

/// Typing waits for a pause; Enter loads at once.
fn edited(ui: &egui::Ui, response: &egui::Response, view: &mut LogView, now: Instant) {
    if response.changed() {
        view.typed(now);
    }
    if submitted(ui, response) {
        view.apply();
    }
}

fn submitted(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

/// The Branch menu: All, HEAD, then local and remote branches.
fn branch_values(view: &LogView) -> Vec<(String, LogBranch)> {
    let mut values = vec![
        ("All".to_owned(), LogBranch::All),
        ("HEAD".to_owned(), LogBranch::Head),
    ];
    for (full, short) in &view.branches {
        values.push((short.clone(), LogBranch::Ref(full.clone())));
    }
    values
}

fn branch_menu(ui: &mut egui::Ui, view: &mut LogView) {
    let title = match &view.draft.branch {
        LogBranch::All => "All",
        LogBranch::Head => "HEAD",
        LogBranch::Ref(full) => full
            .strip_prefix("refs/heads/")
            .or_else(|| full.strip_prefix("refs/remotes/"))
            .unwrap_or(full),
    };
    let title = format!("Branch: {title}");
    let picked = pick_menu(ui, &title, "GitBranch", || {
        checked(branch_values(view), &view.draft.branch)
    });
    if let Some((_, branch)) = picked.and_then(|index| branch_values(view).into_iter().nth(index)) {
        view.draft.branch = branch;
        view.apply();
    }
}

/// The User menu: every user, "me", then the authors seen so far.
fn user_values(view: &LogView) -> Vec<(String, Option<String>)> {
    let mut values = vec![("All users".to_owned(), None)];
    if let Some(me) = &view.me {
        values.push((format!("me ({me})"), Some(me.clone())));
    }
    for name in &view.authors {
        values.push((name.clone(), Some(name.clone())));
    }
    values
}

fn user_menu(ui: &mut egui::Ui, view: &mut LogView) {
    let title = match &view.draft.author {
        None => "All".to_owned(),
        Some(name) if view.me.as_ref() == Some(name) => "me".to_owned(),
        Some(name) => name.clone(),
    };
    let picked = pick_menu(ui, &format!("User: {title}"), "Circle", || {
        checked(user_values(view), &view.draft.author)
    });
    if let Some((_, author)) = picked.and_then(|index| user_values(view).into_iter().nth(index)) {
        view.draft.author = author;
        view.apply();
    }
}

fn date_menu(ui: &mut egui::Ui, view: &mut LogView) {
    let title = format!("Date: {}", view.draft.date.label());
    let picked = pick_menu(ui, &title, "History", || {
        let values = LogDate::ALL.map(|date| (date.label().to_owned(), date));
        checked(values.into(), &view.draft.date)
    });
    if let Some(&date) = picked.and_then(|index| LogDate::ALL.get(index)) {
        view.draft.date = date;
        view.apply();
    }
}

/// Menu rows: each label, and whether its value is the chosen one.
fn checked<T: PartialEq>(values: Vec<(String, T)>, chosen: &T) -> Vec<(String, bool)> {
    values
        .into_iter()
        .map(|(label, value)| {
            let on = value == *chosen;
            (label, on)
        })
        .collect()
}

/// A text menu of rows with a check mark on the chosen one; returns the
/// row that was picked. The rows are built only while the menu is open.
fn pick_menu(
    ui: &mut egui::Ui,
    title: &str,
    icon: &str,
    rows: impl FnOnce() -> Vec<(String, bool)>,
) -> Option<usize> {
    let mut picked = None;
    appearance::text_menu_button(ui, title, |ui| {
        let rows = rows();
        egui::ScrollArea::vertical()
            .max_height(MENU_HEIGHT)
            .show(ui, |ui| {
                for (index, (label, on)) in rows.iter().enumerate() {
                    if appearance::menu_item(appearance::MenuItemInput {
                        ui,
                        label,
                        icon,
                        shortcut: if *on { "✓" } else { "" },
                    })
                    .clicked()
                    {
                        picked = Some(index);
                        ui.close();
                    }
                }
            });
    });
    picked
}

/// Errors, Go to Hash notes, and how many commits the filters show.
fn status_line(
    ui: &mut egui::Ui,
    view: &LogView,
    shown: Option<&CommitLog>,
    theme: &AppearanceConfig,
) {
    let failed = appearance::color(&theme.status_failed);
    for note in [&view.error, &view.goto_note].into_iter().flatten() {
        ui.colored_label(failed, note);
    }
    if let Some(rev) = &view.finding {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak(format!("Finding {rev}…"));
        });
    }
    let Some(log) = shown.filter(|log| log.error.is_none() && !log.query.filter.is_empty()) else {
        return;
    };
    let mut text = match log.commits.len() {
        0 => "No commits match the filters.".to_owned(),
        1 => "1 commit matches the filters.".to_owned(),
        count if count < 1000 => format!("{count} commits match the filters."),
        _ => "The newest 1000 matching commits are shown.".to_owned(),
    };
    if log.query.filter.hides_commits() {
        text.push_str(" Lines join only the commits in the list.");
    }
    ui.weak(text);
}
