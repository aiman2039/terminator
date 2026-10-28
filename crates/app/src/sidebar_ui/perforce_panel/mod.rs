//! The Perforce sidebar panel: workspace, changelist actions, and files.
use super::git_panel::{GitFileAction, GitPanelOutcome, git_color};
use crate::{appearance, file_actions::FileAction, perforce::Op, services::ContextData};
use eframe::egui::{self, RichText};
use terminator_core::appearance::AppearanceConfig;

pub(super) struct PerforcePanelInput<'a> {
    pub(super) context: &'a ContextData,
    pub(super) theme: &'a AppearanceConfig,
    pub(super) open_shortcut: &'a str,
}

pub(super) fn panel(ui: &mut egui::Ui, input: PerforcePanelInput<'_>) -> GitPanelOutcome {
    let PerforcePanelInput {
        context,
        theme,
        open_shortcut,
    } = input;
    let mut outcome = GitPanelOutcome::default();
    let Some(workspace) = &context.perforce else {
        return outcome;
    };
    let view = PerforceView {
        context,
        workspace,
        theme,
        open_shortcut,
        id: ui.make_persistent_id(("perforce", &context.root)),
    };
    view.header(ui, &mut outcome);
    let change = view.changelist_controls(ui, &mut outcome);
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt(view.id.with("files"))
        .show(ui, |ui| {
            view.pending(ui, &change);
            view.files(ui, &mut outcome);
            view.history(ui);
        });
    outcome
}

/// One paint of the panel for a Perforce workspace.
struct PerforceView<'a> {
    context: &'a ContextData,
    workspace: &'a crate::perforce::Workspace,
    theme: &'a AppearanceConfig,
    open_shortcut: &'a str,
    id: egui::Id,
}

impl PerforceView<'_> {
    /// Title with refresh and sync, client, stream, and any error.
    fn header(&self, ui: &mut egui::Ui, outcome: &mut GitPanelOutcome) {
        ui.horizontal(|ui| {
            ui.strong("Perforce");
            if appearance::sidebar_action(ui, "RefreshCw", "Refresh Perforce status").clicked() {
                outcome.refresh = true;
            }
            if appearance::sidebar_action(ui, "Download", "Sync this folder to the latest revision")
                .clicked()
            {
                outcome.perforce.push(Op::Sync);
            }
        });
        ui.label(&self.workspace.client);
        if !self.workspace.stream.is_empty() {
            ui.weak(&self.workspace.stream);
        }
        if let Some(error) = &self.context.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    /// Changelist field, submit/shelve/unshelve, and the submit
    /// confirmation. Returns the changelist typed this frame.
    fn changelist_controls(&self, ui: &mut egui::Ui, outcome: &mut GitPanelOutcome) -> String {
        let id = self.id;
        let mut change = ui.data_mut(|data| data.get_temp::<String>(id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut change).hint_text("Numbered changelist"));
        let valid = !change.is_empty() && change.bytes().all(|byte| byte.is_ascii_digit());
        let confirmation_id = id.with("submit");
        let mut confirmation = ui.data_mut(|data| data.get_temp::<String>(confirmation_id));
        ui.horizontal(|ui| {
            ui.add_enabled_ui(valid, |ui| {
                if let Some(asked) = Self::changelist_actions(ui, &change, outcome) {
                    confirmation = Some(asked);
                }
            });
        });
        if let Some(number) = confirmation.clone() {
            confirmation = Self::confirm_submit(ui, number, outcome);
        }
        ui.data_mut(|data| {
            data.insert_temp(id, change.clone());
            if let Some(number) = confirmation {
                data.insert_temp(confirmation_id, number);
            } else {
                data.remove::<String>(confirmation_id);
            }
        });
        change
    }

    /// Submit (asks first), shelve, and unshelve. Returns a changelist to
    /// confirm.
    fn changelist_actions(
        ui: &mut egui::Ui,
        change: &str,
        outcome: &mut GitPanelOutcome,
    ) -> Option<String> {
        let mut confirm = None;
        if appearance::sidebar_action(ui, "Upload", "Submit this numbered changelist").clicked() {
            confirm = Some(change.to_owned());
        }
        if appearance::sidebar_action(ui, "Archive", "Shelve this numbered changelist").clicked() {
            outcome.perforce.push(Op::Shelve(change.to_owned()));
        }
        if appearance::sidebar_action(ui, "ArchiveRestore", "Unshelve into the default changelist")
            .clicked()
        {
            outcome.perforce.push(Op::Unshelve(change.to_owned()));
        }
        confirm
    }

    /// The pending submit prompt. Returns the confirmation still pending.
    fn confirm_submit(
        ui: &mut egui::Ui,
        number: String,
        outcome: &mut GitPanelOutcome,
    ) -> Option<String> {
        ui.label(format!("Submit all files in changelist {number}?"));
        let mut confirmation = Some(number.clone());
        ui.horizontal(|ui| {
            if ui.button("Submit").clicked() {
                outcome.perforce.push(Op::Submit(number));
                confirmation = None;
            }
            if ui.button("Cancel").clicked() {
                confirmation = None;
            }
        });
        confirmation
    }

    /// Pending changelists; picking one fills the changelist field.
    fn pending(&self, ui: &mut egui::Ui, change: &str) {
        for pending in &self.workspace.pending {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(change == pending.0, format!("CL {}", pending.0))
                    .clicked()
                {
                    ui.data_mut(|data| data.insert_temp(self.id, pending.0.clone()));
                }
                ui.add(egui::Label::new(&pending.1).truncate());
            });
        }
    }

    /// Changed files grouped by changelist (or not opened).
    fn files(&self, ui: &mut egui::Ui, outcome: &mut GitPanelOutcome) {
        let mut groups = std::collections::BTreeMap::<String, Vec<&crate::perforce::File>>::new();
        for file in &self.workspace.files {
            let group = if file.opened {
                format!("Changelist {}", file.change)
            } else {
                "Not opened".into()
            };
            groups.entry(group).or_default().push(file);
        }
        if groups.is_empty() && self.context.error.is_none() {
            ui.weak("No changed files");
        }
        for (group, files) in groups {
            egui::CollapsingHeader::new(format!("{group} ({})", files.len()))
                .id_salt(self.id.with(&group))
                .default_open(true)
                .show(ui, |ui| {
                    for file in files {
                        self.file_row(ui, file, outcome);
                    }
                });
        }
    }

    /// Status letter and path; click opens the diff.
    fn file_row(
        &self,
        ui: &mut egui::Ui,
        file: &crate::perforce::File,
        outcome: &mut GitPanelOutcome,
    ) {
        let relative = self
            .context
            .root
            .as_ref()
            .and_then(|root| file.path.strip_prefix(root).ok())
            .unwrap_or(&file.path);
        let letter = match file.action.as_str() {
            "add" | "branch" | "move/add" => 'A',
            "delete" | "move/delete" => 'D',
            _ => 'M',
        };
        let row = ui
            .selectable_label(
                false,
                RichText::new(format!("{letter}  {}", relative.display()))
                    .color(git_color(self.theme, letter)),
            )
            .on_hover_text(format!("{} · {}", file.action, file.path.display()));
        if row.clicked() {
            outcome.clicked.push(GitFileAction {
                action: FileAction::NativeWorkingDiff,
                path: file.path.clone(),
            });
        }
        row.context_menu(|ui| self.file_menu(ui, file, outcome));
    }

    fn file_menu(
        &self,
        ui: &mut egui::Ui,
        file: &crate::perforce::File,
        outcome: &mut GitPanelOutcome,
    ) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Open file",
            icon: "FileText",
            shortcut: self.open_shortcut,
        })
        .clicked()
        {
            outcome.menu.push(GitFileAction {
                action: FileAction::Open,
                path: file.path.clone(),
            });
            ui.close();
        }
        if !file.opened {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Open for edit",
                icon: "Pencil",
                shortcut: "",
            })
            .clicked()
            {
                outcome.perforce.push(Op::Edit(file.path.clone()));
                ui.close();
            }
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Reconcile file",
                icon: "RefreshCw",
                shortcut: "",
            })
            .clicked()
            {
                outcome.perforce.push(Op::Reconcile(file.path.clone()));
                ui.close();
            }
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Copy path",
            icon: "Copy",
            shortcut: "",
        })
        .clicked()
        {
            outcome.copy.push(file.path.display().to_string());
            ui.close();
        }
    }

    fn history(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Submitted changes")
            .id_salt(self.id.with("history"))
            .show(ui, |ui| {
                for (number, description) in &self.workspace.history {
                    ui.label(format!("CL {number}  {description}"));
                }
            });
    }
}

#[cfg(test)]
mod tests;
