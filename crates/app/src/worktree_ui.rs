//! GUI client for `worktrees-v1`. Git still owns the checkout.
use super::*;

#[derive(Clone)]
pub(crate) struct WorktreeDraft {
    pub source: String,
    pub start: String,
    pub branch: String,
    pub dest: PathBuf,
    pub open_terminal: bool,
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
pub(super) fn create(
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
    pub(super) fn has_worktrees(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|capability| capability == WORKTREES_CAPABILITY)
    }

    pub(super) fn managed_worktree<'a>(
        &'a self,
        project_id: &str,
    ) -> Option<&'a terminator_core::worktrees::Registration> {
        self.state
            .worktrees
            .iter()
            .find(|worktree| worktree.project_id == project_id && !worktree.removed)
    }

    pub(super) fn worktree_children<'a>(
        &'a self,
        project: &Project,
    ) -> Vec<&'a terminator_core::worktrees::Registration> {
        if self.managed_worktree(&project.id).is_some() {
            return Vec::new();
        }
        self.state
            .worktrees
            .iter()
            .filter(|worktree| {
                !worktree.removed
                    && worktree.project_id != project.id
                    && (worktree.common_dir.starts_with(&project.path)
                        || worktree.common_dir.parent() == Some(project.path.as_path()))
            })
            .collect()
    }

    pub(super) fn open_worktree_wizard(&mut self) {
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
        if terminator_core::worktrees::common_dir(&project.path).is_err() {
            self.info = Some("This folder is not a git checkout.".into());
            return;
        }
        let branch = default_branch(&project.name);
        let dest = default_dest(&project.path, &branch);
        self.worktree_draft = Some(WorktreeDraft {
            source: project.id.clone(),
            start: "HEAD".into(),
            branch,
            dest,
            open_terminal: true,
        });
    }

    pub(super) fn worktree_center(&mut self, ui: &mut egui::Ui) {
        let Some(mut draft) = self.worktree_draft.clone() else {
            return;
        };
        let mut submit = false;
        let mut browse = false;
        let mut cancel = false;
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
        self.worktree_form(ui, &mut draft, &mut submit, &mut browse, &mut cancel);
        if browse {
            self.browse_target = Some(BrowseTarget::WorktreeDest);
        }
        if submit {
            self.submit_worktree(&draft);
        }
        if submit || cancel {
            self.worktree_draft = None;
        } else {
            self.worktree_draft = Some(draft);
        }
    }

    fn worktree_form(
        &self,
        ui: &mut egui::Ui,
        draft: &mut WorktreeDraft,
        submit: &mut bool,
        browse: &mut bool,
        cancel: &mut bool,
    ) {
                ui.weak("Creates an isolated git checkout as a new project. You start agents in its terminal.");
                ui.add_space(8.0);
                let source_name = self
                    .state
                    .projects
                    .iter()
                    .find(|project| project.id == draft.source)
                    .map(|project| project.name.clone())
                    .unwrap_or_else(|| "Project".into());
                settings_controls::settings_row(ui, "Source project", "Must already be a git checkout.", |ui| {
                    ui.label(source_name);
                });
                settings_controls::settings_row(
                    ui,
                    "Start from",
                    "HEAD, a branch, or another git revision.",
                    |ui| {
                        ui.horizontal(|ui| {
                            for start in ["HEAD", "main", "master"] {
                                if ui
                                    .selectable_label(draft.start == start, start)
                                    .clicked()
                                {
                                    draft.start = start.into();
                                }
                            }
                        });
                        ui.add(
                            egui::TextEdit::singleline(&mut draft.start)
                                .hint_text("revision")
                                .desired_width(220.0),
                        );
                    },
                );
                settings_controls::settings_row(
                    ui,
                    "Branch name",
                    "Created on the new checkout. Git rejects invalid names.",
                    |ui| {
                        ui.add(egui::TextEdit::singleline(&mut draft.branch).desired_width(260.0));
                    },
                );
                settings_controls::settings_row(
                    ui,
                    "Destination",
                    "Folder must not already exist.",
                    |ui| {
                        let mut dest = draft.dest.display().to_string();
                        let response = ui.add(egui::TextEdit::singleline(&mut dest).desired_width(260.0));
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), "worktree-dest", response.rect);
                        if response.changed() {
                            draft.dest = PathBuf::from(dest);
                        }
                        if ui.button("Browse…").clicked() {
                            *browse = true;
                        }
                    },
                );
                ui.checkbox(&mut draft.open_terminal, "Create a terminal in the new project");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let create = ui.add_enabled(
                        !draft.branch.trim().is_empty() && !draft.start.trim().is_empty(),
                        egui::Button::new("Create worktree"),
                    );
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "worktree-create", create.rect);
                    if create.clicked() {
                        *submit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        *cancel = true;
                    }
                });
    }

    fn submit_worktree(&mut self, draft: &WorktreeDraft) {
        let _ = self.jobs.send(Job::CreateWorktree(draft.clone()));
    }

    pub(super) fn confirm_remove_worktree(&mut self, project_id: &str) {
        self.worktree_remove = Some(project_id.into());
    }

    pub(super) fn worktree_remove_dialog(&mut self, ctx: &egui::Context) {
        let Some(project_id) = self.worktree_remove.clone() else {
            return;
        };
        let Some(record) = self.managed_worktree(&project_id).cloned() else {
            self.worktree_remove = None;
            return;
        };
        let name = self
            .state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "Worktree".into());
        let mut open = true;
        self.popups
            .window(ctx, "Remove worktree?")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Remove {name} at {}? Git refuses dirty or locked checkouts. Live sessions must be stopped first. The branch is kept.",
                    record.path.display()
                ));
                ui.horizontal(|ui| {
                    if ui.button("Remove worktree").clicked() {
                        self.send(Request::WorktreeRemove {
                            project: project_id.clone(),
                        });
                        self.worktree_remove = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.worktree_remove = None;
                    }
                });
            });
        if !open {
            self.worktree_remove = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(dest: PathBuf) -> WorktreeDraft {
        WorktreeDraft {
            source: "source".into(),
            start: "HEAD".into(),
            branch: "task".into(),
            dest,
            open_terminal: true,
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
        std::os::unix::fs::symlink(&checkout, &alias).unwrap();
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
