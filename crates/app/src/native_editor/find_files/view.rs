//! The Find in Files window: query, toggles, file mask, and results
//! grouped by file. It stays open until closed; a click opens the file at
//! the match (Back returns).
use super::*;

const TITLE: &str = "Find in Files";

impl App {
    pub(crate) fn show_find_in_files_window(&mut self, ctx: &egui::Context) {
        self.drive_find_in_files();
        let Some(find) = self.find_in_files.as_mut() else {
            return;
        };
        let mut open = true;
        let mut escape = false;
        let mut picked: Option<NavLocation> = None;
        self.popups
            .window(ctx, TITLE)
            .open(&mut open)
            .collapsible(false)
            .default_width(620.0)
            .show(ctx, |ui| {
                escape = find_inputs(ui, find);
                status_line(ui, find);
                picked = results(ui, find);
            });
        if find.running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(target) = picked {
            self.record_jump_origin();
            self.open_location(&target);
        }
        if !open || escape {
            self.find_in_files = None;
        }
    }
}

/// Query row with toggles, then the file mask. True when Escape asks to
/// close.
fn find_inputs(ui: &mut egui::Ui, find: &mut FindInFiles) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let field = ui.add(
            appearance::singleline(&mut find.query)
                .hint_text("Text to find")
                .desired_width((ui.available_width() - 90.0).max(160.0)),
        );
        if find.focus {
            field.request_focus();
            find.focus = false;
        }
        if field.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        for (flag, icon, tip) in [
            (&mut find.case, "CaseSensitive", "Match case"),
            (&mut find.word, "WholeWord", "Words"),
            (&mut find.regex, "Regex", "Regex"),
        ] {
            if appearance::selectable_icon(appearance::SelectableIconInput {
                ui,
                icon,
                tip,
                selected: *flag,
            })
            .clicked()
            {
                *flag = !*flag;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.weak("File mask");
        ui.add(
            appearance::singleline(&mut find.mask)
                .hint_text("*.rs, src/**")
                .desired_width(ui.available_width()),
        );
    });
    close
}

fn status_line(ui: &mut egui::Ui, find: &FindInFiles) {
    match (&find.outcome, find.running) {
        (_, true) => {
            ui.weak("Searching…");
        }
        (Some(Err(error)), false) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        (Some(Ok(outcome)), false) => {
            let plural = |n: usize| if n == 1 { "" } else { "es" };
            let more = if outcome.truncated {
                format!(" (first {RESULT_LIMIT})")
            } else {
                String::new()
            };
            ui.weak(format!(
                "{} match{} in {} file{}{more}",
                outcome.total,
                plural(outcome.total),
                outcome.files.len(),
                if outcome.files.len() == 1 { "" } else { "s" },
            ));
        }
        (None, false) => {
            ui.weak("Type to search in the project");
        }
    }
}

/// Results grouped by file; returns the clicked match.
fn results(ui: &mut egui::Ui, find: &FindInFiles) -> Option<NavLocation> {
    let Some(Ok(outcome)) = &find.outcome else {
        return None;
    };
    let mut picked = None;
    egui::ScrollArea::vertical()
        .max_height(440.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for file in &outcome.files {
                ui.horizontal(|ui| {
                    ui.strong(crate::workspace_ops::relative_display(
                        &find.root, &file.path,
                    ));
                    ui.weak(format!("({})", file.hits.len()));
                });
                for hit in &file.hits {
                    let label = egui::RichText::new(format!(
                        "{:>5}  {}",
                        hit.line.saturating_add(1),
                        hit.preview
                    ))
                    .monospace();
                    if ui
                        .add(egui::Button::selectable(false, label).truncate())
                        .clicked()
                    {
                        picked = Some(NavLocation {
                            path: file.path.clone(),
                            line: hit.line,
                            character: hit.character,
                        });
                    }
                }
                ui.add_space(4.0);
            }
        });
    picked
}
