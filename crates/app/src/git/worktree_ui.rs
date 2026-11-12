//! GUI client for `worktrees-v1`. Git still owns the checkout.
pub(crate) use super::task_form::TaskStart;
use crate::*;

fn start_revision(ui: &mut egui::Ui, draft: &mut WorktreeDraft) {
    ui.horizontal(|ui| {
        for start in ["HEAD", "main", "master"] {
            if ui.selectable_label(draft.start == start, start).clicked() {
                draft.start = start.into();
            }
        }
    });
    ui.add(
        appearance::singleline(&mut draft.start)
            .hint_text("revision")
            .desired_width(220.0),
    );
}

/// Destination path field. Returns whether Browse was clicked.
fn destination(ui: &mut egui::Ui, draft: &mut WorktreeDraft) -> bool {
    let mut dest = draft.dest.display().to_string();
    let response = ui.add(appearance::singleline(&mut dest).desired_width(260.0));
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "worktree-dest", response.rect);
    if response.changed() {
        draft.dest = PathBuf::from(dest);
    }
    ui.button("Browse…").clicked()
}

/// Returns `(create, cancel)` clicks.
fn form_buttons(ui: &mut egui::Ui, draft: &WorktreeDraft) -> (bool, bool) {
    // One submission at a time: Create stays off until it answers.
    let create = ui.add_enabled(
        !draft.submitting && !draft.branch.trim().is_empty() && !draft.start.trim().is_empty(),
        egui::Button::new(if draft.submitting {
            "Creating…"
        } else {
            "Create worktree"
        }),
    );
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "worktree-create", create.rect);
    let create = create.clicked();
    (create, ui.button("Cancel").clicked())
}

#[cfg(test)]
use anyhow::Context;

pub(crate) struct WorktreeFormInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) draft: &'a mut WorktreeDraft,
    pub(crate) submit: &'a mut bool,
    pub(crate) browse: &'a mut bool,
    pub(crate) cancel: &'a mut bool,
}

#[derive(Clone, Default)]
pub(crate) struct WorktreeDraft {
    pub source: String,
    pub start: String,
    pub branch: String,
    pub dest: PathBuf,
    pub open_terminal: bool,
    /// Short task name. Editing it sets the branch and destination.
    pub task: String,
    /// Typed into the new terminal without Enter. The user starts the agent.
    pub agent_command: String,
    /// Optional http(s) URL opened in a Browser tab.
    pub browser_url: String,
    /// Identifies this opening of the wizard; a submission's reply only
    /// closes or unlocks the draft it came from.
    pub id: u64,
    /// A Create of this draft is running.
    pub submitting: bool,
}

/// A new draft ID; never 0, the ID of a default draft.
fn next_draft_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn default_branch(name: &str) -> String {
    let leaf = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let leaf = leaf.trim_matches('-');
    format!("terminator/{}", if leaf.is_empty() { "task" } else { leaf })
}

pub(crate) fn default_dest(source: &Path, branch: &str) -> PathBuf {
    let parent = source.parent().unwrap_or(source);
    let leaf = branch.replace('/', "-");
    parent.join(if leaf.is_empty() {
        "terminator-task".into()
    } else {
        leaf
    })
}

/// Resolve the created project on the worker, only after an acknowledged add.
#[cfg(test)]
pub(crate) fn create(
    draft: &WorktreeDraft,
    mut request: impl FnMut(Request) -> Result<Response>,
) -> Result<(Box<State>, String)> {
    anyhow::ensure!(
        matches!(
            request(Request::WorktreeAdd {
                project: draft.source.clone(),
                path: draft.dest.clone(),
                branch: Some(draft.branch.trim().to_owned()).filter(|branch| !branch.is_empty()),
                start: draft.start.trim().to_owned(),
            })?,
            Response::Ok
        ),
        "Unexpected worktree creation response"
    );
    let canonical = draft.dest.canonicalize()?;
    let Response::State(state) = request(Request::Snapshot)? else {
        anyhow::bail!("Expected worktree snapshot");
    };
    let project = state
        .projects
        .iter()
        .find(|project| project.path == canonical)
        .context("Created worktree missing from inventory")?
        .id
        .clone();
    Ok((state, project))
}

impl App {
    pub(crate) fn has_worktrees(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|capability| capability == WORKTREES_CAPABILITY)
    }

    pub(crate) fn managed_worktree<'a>(
        &'a self,
        project_id: &str,
    ) -> Option<&'a terminator_core::worktrees::Registration> {
        self.state
            .worktrees
            .iter()
            .find(|worktree| worktree.project_id == project_id && !worktree.removed)
    }

    pub(crate) fn open_worktree_wizard(&mut self) {
        if !self.has_worktrees() {
            self.info = Some(
                "The running session service does not support worktrees. Update it after finishing live sessions."
                    .into(),
            );
            return;
        }
        let Some(project) = self.selected_project() else {
            self.info = Some("Select a git project before creating a worktree.".into());
            return;
        };
        // The daemon name, not a GUI display rename.
        let name = self
            .project_names
            .base
            .get(&project.id)
            .unwrap_or(&project.name);
        let branch = default_branch(name);
        let dest = default_dest(&project.path, &branch);
        let draft = WorktreeDraft {
            source: project.id.clone(),
            start: "HEAD".into(),
            branch,
            dest,
            open_terminal: true,
            task: String::new(),
            agent_command: self.preferences.task_agent_command.clone(),
            browser_url: String::new(),
            id: next_draft_id(),
            submitting: false,
        };
        // `git rev-parse` runs on a worker; the form opens when it answers.
        let _ = self
            .jobs
            .send(Job::WorktreeProbe(project.path.clone(), draft));
    }

    /// Open the wizard once the worker confirmed a git checkout.
    pub(crate) fn apply_worktree_probe(&mut self, draft: WorktreeDraft, git: bool) {
        if !git {
            self.info = Some("This folder is not a git checkout.".into());
        } else if self.worktree_draft.is_none() {
            // The list page shows the form above it; else the form page.
            if self.center != crate::CenterPage::Worktrees {
                self.show_center(crate::CenterPage::WorktreeDraft);
            }
            self.worktree_draft = Some(draft);
        }
    }

    pub(crate) fn worktree_center(&mut self, ui: &mut egui::Ui) {
        let Some(mut draft) = self.worktree_draft.clone() else {
            self.hide_center_overlay();
            return;
        };
        let mut submit = false;
        let mut browse = false;
        let mut cancel = false;
        let style = appearance::CenterCardStyle::new(&self.theme);
        let ((), dismissed) = appearance::center_card(ui, style, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.strong("New task worktree");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if appearance::sidebar_action(ui, "X", "Close").clicked() {
                        cancel = true;
                    }
                });
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .id_salt("worktree-form")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.worktree_form(WorktreeFormInput {
                        ui,
                        draft: &mut draft,
                        submit: &mut submit,
                        browse: &mut browse,
                        cancel: &mut cancel,
                    });
                });
        });
        cancel |= dismissed;
        if browse {
            self.request_pick(PickKind::Browse(BrowseTarget::WorktreeDest));
        }
        if submit {
            self.submit_worktree(&mut draft);
        }
        // Keep the form after Create until the worker reports success, so a
        // failed creation can be corrected and retried.
        if cancel {
            self.hide_center_overlay();
        } else {
            self.worktree_draft = Some(draft);
        }
    }

    pub(crate) fn worktree_form(&self, input: WorktreeFormInput<'_>) {
        let WorktreeFormInput {
            ui,
            draft,
            submit,
            browse,
            cancel,
        } = input;
        ui.weak(
            "Creates an isolated git checkout as a new project. You start agents in its terminal.",
        );
        ui.add_space(8.0);
        let source = self
            .state
            .projects
            .iter()
            .find(|project| project.id == draft.source);
        let source_name = source.map_or_else(|| "Project".into(), |project| project.name.clone());
        let source_path = source.map(|project| project.path.clone());
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Source project",
            description: "Must already be a git checkout.",
            add_control: |ui| {
                ui.label(source_name);
            },
        });
        super::task_form::task_name(ui, draft, source_path.as_deref());
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Start from",
            description: "HEAD, a branch, or another git revision.",
            add_control: |ui| start_revision(ui, draft),
        });
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Branch name",
            description: "Created on the new checkout. Git rejects invalid names.",
            add_control: |ui| {
                ui.add(appearance::singleline(&mut draft.branch).desired_width(260.0));
            },
        });
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Destination",
            description: "Folder must not already exist.",
            add_control: |ui| {
                if destination(ui, draft) {
                    *browse = true;
                }
            },
        });
        ui.checkbox(
            &mut draft.open_terminal,
            "Create a terminal in the new project",
        );
        super::task_form::task_extras(ui, draft, &self.agent_launchers());
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let (create, close) = form_buttons(ui, draft);
            if create {
                *submit = true;
            }
            if close {
                *cancel = true;
            }
        });
    }

    /// Sends the draft once; it stays `submitting` until its reply
    /// (`WorktreeCreated` or `WorktreeFailed`). A refused send answers
    /// `WorktreeFailed` at once.
    pub(crate) fn submit_worktree(&mut self, draft: &mut WorktreeDraft) {
        if draft.submitting || !self.accept_task(draft) {
            return;
        }
        draft.submitting = self.jobs.send(Job::CreateWorktree(draft.clone())).is_ok();
    }

    pub(crate) fn confirm_remove_worktree(&mut self, project_id: &str) {
        self.worktree_remove = Some(project_id.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn draft(dest: PathBuf) -> WorktreeDraft {
        WorktreeDraft {
            source: "source".into(),
            start: "HEAD".into(),
            branch: "task".into(),
            dest,
            open_terminal: true,
            ..Default::default()
        }
    }

    #[test]
    fn failed_creation_never_looks_up_an_existing_project() {
        let mut requests = 0;
        let result = create(&draft("/existing".into()), |request| {
            requests += 1;
            assert!(matches!(request, Request::WorktreeAdd { .. }));
            anyhow::bail!("Worktree destination already exists")
        });
        assert!(result.unwrap_err().to_string().contains("already exists"));
        assert_eq!(requests, 1);
    }

    #[test]
    fn creation_resolves_a_symlink_destination_to_its_project() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        let alias = dir.path().join("alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&checkout, &alias).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&checkout, &alias).unwrap();
        let state = State {
            projects: vec![Project {
                id: "created".into(),
                name: "task".into(),
                path: checkout.canonicalize().unwrap(),
                layout: serde_json::Value::Null,
            }],
            ..Default::default()
        };
        let (_, project) = create(&draft(alias.clone()), |request| match request {
            Request::WorktreeAdd { path, .. } => {
                assert_eq!(path, alias);
                Ok(Response::Ok)
            }
            Request::Snapshot => Ok(Response::State(Box::new(state.clone()))),
            _ => panic!("Unexpected request"),
        })
        .unwrap();
        assert_eq!(project, "created");
    }

    #[test]
    fn branch_and_destination_are_stable() {
        assert_eq!(default_branch("My App"), "terminator/My-App");
        assert_eq!(
            default_dest(Path::new("/src/app"), "terminator/task"),
            PathBuf::from("/src/terminator-task")
        );
    }
}
