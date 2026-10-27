use super::git_panel::{GitFileAction, GitPanelOutcome, git_color};
use crate::{appearance, file_actions::FileAction, perforce::Op, services::ContextData};
use eframe::egui::{self, RichText};
use terminator_core::appearance::AppearanceConfig;

pub(super) fn panel(
    ui: &mut egui::Ui,
    context: &ContextData,
    theme: &AppearanceConfig,
    open_shortcut: &str,
) -> GitPanelOutcome {
    let mut outcome = GitPanelOutcome::default();
    let Some(workspace) = &context.perforce else {
        return outcome;
    };
    let id = ui.make_persistent_id(("perforce", &context.root));
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
    ui.label(&workspace.client);
    if !workspace.stream.is_empty() {
        ui.weak(&workspace.stream);
    }
    if let Some(error) = &context.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let mut change = ui.data_mut(|data| data.get_temp::<String>(id).unwrap_or_default());
    ui.add(egui::TextEdit::singleline(&mut change).hint_text("Numbered changelist"));
    let valid = !change.is_empty() && change.bytes().all(|byte| byte.is_ascii_digit());
    let confirmation_id = id.with("submit");
    let mut confirmation = ui.data_mut(|data| data.get_temp::<String>(confirmation_id));
    ui.horizontal(|ui| {
        ui.add_enabled_ui(valid, |ui| {
            if appearance::sidebar_action(ui, "Upload", "Submit this numbered changelist").clicked()
            {
                confirmation = Some(change.clone());
            }
            if appearance::sidebar_action(ui, "Archive", "Shelve this numbered changelist")
                .clicked()
            {
                outcome.perforce.push(Op::Shelve(change.clone()));
            }
            if appearance::sidebar_action(
                ui,
                "ArchiveRestore",
                "Unshelve into the default changelist",
            )
            .clicked()
            {
                outcome.perforce.push(Op::Unshelve(change.clone()));
            }
        });
    });
    if let Some(number) = confirmation.clone() {
        ui.label(format!("Submit all files in changelist {number}?"));
        ui.horizontal(|ui| {
            if ui.button("Submit").clicked() {
                outcome.perforce.push(Op::Submit(number));
                confirmation = None;
            }
            if ui.button("Cancel").clicked() {
                confirmation = None;
            }
        });
    }
    ui.data_mut(|data| {
        data.insert_temp(id, change.clone());
        if let Some(number) = confirmation {
            data.insert_temp(confirmation_id, number);
        } else {
            data.remove::<String>(confirmation_id);
        }
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt(id.with("files"))
        .show(ui, |ui| {
            for pending in &workspace.pending {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(change == pending.0, format!("CL {}", pending.0))
                        .clicked()
                    {
                        ui.data_mut(|data| data.insert_temp(id, pending.0.clone()));
                    }
                    ui.add(egui::Label::new(&pending.1).truncate());
                });
            }
            let mut groups =
                std::collections::BTreeMap::<String, Vec<&crate::perforce::File>>::new();
            for file in &workspace.files {
                let group = if file.opened {
                    format!("Changelist {}", file.change)
                } else {
                    "Not opened".into()
                };
                groups.entry(group).or_default().push(file);
            }
            if groups.is_empty() && context.error.is_none() {
                ui.weak("No changed files");
            }
            for (group, files) in groups {
                egui::CollapsingHeader::new(format!("{group} ({})", files.len()))
                    .id_salt(id.with(&group))
                    .default_open(true)
                    .show(ui, |ui| {
                        for file in files {
                            let relative = context
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
                                        .color(git_color(theme, letter)),
                                )
                                .on_hover_text(format!(
                                    "{} · {}",
                                    file.action,
                                    file.path.display()
                                ));
                            if row.clicked() {
                                outcome.clicked.push(GitFileAction {
                                    action: FileAction::NativeWorkingDiff,
                                    path: file.path.clone(),
                                });
                            }
                            row.context_menu(|ui| {
                                if appearance::menu_item(ui, "Open file", "FileText", open_shortcut)
                                    .clicked()
                                {
                                    outcome.menu.push(GitFileAction {
                                        action: FileAction::Open,
                                        path: file.path.clone(),
                                    });
                                    ui.close();
                                }
                                if !file.opened {
                                    if appearance::menu_item(ui, "Open for edit", "Pencil", "")
                                        .clicked()
                                    {
                                        outcome.perforce.push(Op::Edit(file.path.clone()));
                                        ui.close();
                                    }
                                    if appearance::menu_item(ui, "Reconcile file", "RefreshCw", "")
                                        .clicked()
                                    {
                                        outcome.perforce.push(Op::Reconcile(file.path.clone()));
                                        ui.close();
                                    }
                                }
                                if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
                                    outcome.copy.push(file.path.display().to_string());
                                    ui.close();
                                }
                            });
                        }
                    });
            }
            egui::CollapsingHeader::new("Submitted changes")
                .id_salt(id.with("history"))
                .show(ui, |ui| {
                    for (number, description) in &workspace.history {
                        ui.label(format!("CL {number}  {description}"));
                    }
                });
        });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perforce_panel_renders_changelists_and_confirms_submit() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let context = ContextData {
            cwd: "/repo".into(),
            root: Some("/repo".into()),
            git_dirs: Vec::new(),
            perforce: Some(crate::perforce::Workspace {
                client: "test-client".into(),
                command_root: "/repo".into(),
                stream: "//streams/main".into(),
                pending: vec![("42".into(), "Fix file".into())],
                history: vec![("41".into(), "Initial".into())],
                files: vec![crate::perforce::File {
                    path: "/repo/changed.rs".into(),
                    action: "edit".into(),
                    change: "42".into(),
                    opened: true,
                }],
            }),
            branch: String::new(),
            changes: Vec::new(),
            decorations: std::collections::HashMap::default(),
            stats: std::collections::HashMap::default(),
            error: None,
        };
        let theme = AppearanceConfig::default();
        let draw = |events, confirm| {
            let mut outcome = GitPanelOutcome::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(360.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let id = ui.make_persistent_id(("perforce", &context.root));
                    ui.data_mut(|data| {
                        data.insert_temp(id, "42".to_string());
                        if confirm {
                            data.insert_temp(id.with("submit"), "42".to_string());
                        }
                    });
                    outcome = panel(ui, &context, &theme, "");
                },
            );
            output.textures_delta.clear();
            (output, outcome)
        };
        draw(Vec::new(), false);
        let (output, outcome) = draw(Vec::new(), false);
        assert!(
            outcome.perforce.is_empty(),
            "rendering must not mutate Perforce"
        );
        let text = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some((text.pos, text.galley.text().to_string()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let painted = text(&output);
        assert!(painted.iter().any(|(_, text)| text == "Perforce"));
        assert!(
            painted
                .iter()
                .any(|(_, text)| text.contains("Changelist 42"))
        );
        assert!(!painted.iter().any(|(_, text)| text == "Amend"));
        let (output, outcome) = draw(Vec::new(), true);
        assert!(outcome.perforce.is_empty());
        let button = text(&output)
            .into_iter()
            .find(|(_, text)| text == "Submit")
            .unwrap()
            .0
            + egui::vec2(3.0, 3.0);
        draw(
            vec![
                egui::Event::PointerMoved(button),
                egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            false,
        );
        let (_, outcome) = draw(
            vec![egui::Event::PointerButton {
                pos: button,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            false,
        );
        assert!(matches!(outcome.perforce.as_slice(), [Op::Submit(number)] if number == "42"));
    }
}
