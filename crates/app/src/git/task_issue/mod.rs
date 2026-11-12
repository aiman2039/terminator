//! New task from an issue or pull request: an optional field fetched on an
//! explicit Fetch click (read-only `gh`/`glab`, background, 20-second cap).
//! It fills the task name, and for a pull request the start (its head,
//! fetched only on Create). The issue text is typed after the agent command
//! only when the user ticks it; it is never sent.
mod fetch;
#[cfg(test)]
mod tests;

use crate::merge::{Done, take_done};
use crate::worktree_ui::WorktreeDraft;
use crate::*;
pub(crate) use fetch::{Issue, fetch_head};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

/// Longest issue text typed as the first prompt, in characters.
const MAX_PROMPT: usize = 4000;

/// The form's issue row.
#[derive(Clone, Debug, Default)]
pub(crate) struct IssueDraft {
    /// What the user typed: a number or a URL.
    pub input: String,
    pub fetched: Option<Issue>,
    /// Fetching, or why the last fetch failed.
    pub note: Option<String>,
    /// Type the issue text after the agent command (never sent).
    pub type_prompt: bool,
    /// The Start-from text this row set for a pull request; Create fetches
    /// the head only while Start from still shows it.
    pub pr_start: Option<String>,
    /// The checkout the issue was read in.
    pub root: Option<PathBuf>,
    /// Fetch was clicked this frame.
    pub requested: bool,
}

/// The pending fetch: the draft it belongs to and its answer.
#[derive(Default)]
pub(crate) struct IssueFetch {
    pending: Option<(u64, Done<Issue>)>,
}

impl IssueDraft {
    /// The pull request head to fetch on Create: `(checkout, ref)`, only
    /// while Start from still shows the pull request.
    pub(crate) fn head_to_fetch(&self, start: &str) -> Option<(PathBuf, String)> {
        let issue = self.fetched.as_ref()?;
        (self.pr_start.as_deref() == Some(start))
            .then(|| Some((self.root.clone()?, issue.head_ref.clone()?)))
            .flatten()
    }

    /// The prompt typed after the agent command, if ticked: title and body
    /// on one line, capped.
    pub(crate) fn prompt(&self) -> String {
        let Some(issue) = self.fetched.as_ref().filter(|_| self.type_prompt) else {
            return String::new();
        };
        let text = format!("{} (#{}): {}", issue.title, issue.number, issue.body);
        crate::diff::notes::one_line(&text)
            .chars()
            .take(MAX_PROMPT)
            .collect()
    }
}

/// `12-fix-the-login-page`: the number and the first title words.
pub(crate) fn task_name(issue: &Issue) -> String {
    let words: Vec<String> = issue
        .title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(str::to_ascii_lowercase)
        .collect();
    std::iter::once(issue.number.to_string())
        .chain(words)
        .collect::<Vec<_>>()
        .join("-")
}

impl WorktreeDraft {
    /// Fills the form from a fetched issue or pull request.
    pub(crate) fn apply_issue(&mut self, issue: Issue, source: Option<&Path>) {
        self.task = task_name(&issue);
        if let Some(path) = source {
            self.branch = worktree_ui::default_branch(&self.task);
            self.dest = worktree_ui::default_dest(path, &self.branch);
        }
        if issue.head_ref.is_some() {
            let start = format!("PR #{} head", issue.number);
            self.start.clone_from(&start);
            self.issue.pr_start = Some(start);
        } else {
            self.issue.pr_start = None;
        }
        self.issue.note = None;
        self.issue.fetched = Some(issue);
    }
}

/// The "From issue/PR" row and its prompt checkbox.
pub(crate) fn issue_row(ui: &mut egui::Ui, draft: &mut WorktreeDraft, fetching: bool) {
    settings_controls::settings_row(settings_controls::SettingsRowInput {
        ui,
        label: "From issue/PR",
        description: "Optional. A number or URL, read with gh or glab when you click Fetch.",
        add_control: |ui| {
            ui.add(
                appearance::singleline(&mut draft.issue.input)
                    .hint_text("123 or https://…/issues/123")
                    .desired_width(220.0),
            );
            let enabled = !fetching && !draft.issue.input.trim().is_empty();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(if fetching { "Fetching…" } else { "Fetch" }),
                )
                .clicked()
            {
                draft.issue.requested = true;
            }
        },
    });
    if let Some(note) = &draft.issue.note {
        ui.weak(note);
    }
    let Some(issue) = &draft.issue.fetched else {
        return;
    };
    let kind = if issue.head_ref.is_some() {
        "PR"
    } else {
        "Issue"
    };
    ui.add(egui::Label::new(format!("{kind} #{}: {}", issue.number, issue.title)).truncate());
    if draft.open_terminal {
        ui.checkbox(
            &mut draft.issue.type_prompt,
            "Type the issue text as the first prompt",
        )
        .on_hover_text("Typed after the agent command. You press Enter.");
    }
}

impl App {
    /// The pending fetch of this draft, if any.
    pub(crate) fn issue_fetching(&self, draft: u64) -> bool {
        self.git_extras
            .task_issue
            .pending
            .as_ref()
            .is_some_and(|(id, _)| *id == draft)
    }

    /// Starts the read-only lookup for a Fetch click. A newer click
    /// replaces a lookup that still runs.
    pub(crate) fn fetch_task_issue(&mut self, draft: &mut WorktreeDraft) {
        let Some(root) = self
            .state
            .projects
            .iter()
            .find(|p| p.id == draft.source)
            .map(|p| p.path.clone())
        else {
            draft.issue.note = Some("Fetching needs a local source project.".into());
            return;
        };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let refused = tx.clone();
        let service = self.services.clone();
        let (work_root, input) = (root.clone(), draft.issue.input.clone());
        let submitted = self.services.handle().submit(
            OperationContext::new(
                "git",
                format!("task-issue:{}", root.display()),
                Policy::ReplaceableRead,
            ),
            CancellationToken::new(),
            async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || {
                        anyhow::Ok(fetch::fetch(&work_root, &input, None))
                    })
                    .await
                    .unwrap_or_else(|error| Err(format!("{error:#}")));
                let _ = tx.send(result);
                Ok(Vec::new())
            },
        );
        if let Err(error) = submitted {
            let _ = refused.send(Err(format!("Could not start the lookup: {error}")));
        }
        draft.issue.root = Some(root);
        draft.issue.note = Some("Fetching…".into());
        self.git_extras.task_issue.pending = Some((draft.id, rx));
    }

    /// Takes a finished lookup for this draft.
    pub(crate) fn poll_task_issue(&mut self, draft: &mut WorktreeDraft, ctx: &egui::Context) {
        let fetch = &mut self.git_extras.task_issue;
        let Some((id, rx)) = fetch.pending.as_mut() else {
            return;
        };
        if *id != draft.id {
            fetch.pending = None;
            return;
        }
        let Some(result) = take_done(rx) else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        };
        fetch.pending = None;
        let source = draft.issue.root.clone();
        match result {
            Ok(issue) => draft.apply_issue(issue, source.as_deref()),
            Err(error) => draft.issue.note = Some(error),
        }
    }
}
