//! Fuzzy jump list across projects, sessions, files, and settings.
use super::*;

#[derive(Clone, Debug)]
pub(crate) enum PaletteItem {
    Project(String, String),
    Session(String, String),
    File(PathBuf),
    Settings(SettingsSection),
    AddProject,
    NewTerminal,
    NewWorktree,
    OpenPlayer,
}

impl PaletteItem {
    fn label(&self) -> String {
        match self {
            Self::Project(_, name) => format!("Project  {name}"),
            Self::Session(_, label) => format!("Session  {label}"),
            Self::File(path) => format!(
                "File  {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            Self::Settings(section) => format!("Settings  {}", section.title()),
            Self::AddProject => "Add project".into(),
            Self::NewTerminal => "New terminal".into(),
            Self::NewWorktree => "New task worktree".into(),
            Self::OpenPlayer => "Open player".into(),
        }
    }
}

impl App {
    pub(super) fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items = vec![
            PaletteItem::AddProject,
            PaletteItem::NewTerminal,
            PaletteItem::OpenPlayer,
        ];
        if self
            .state
            .capabilities
            .iter()
            .any(|capability| capability == WORKTREES_CAPABILITY)
        {
            items.push(PaletteItem::NewWorktree);
        }
        for section in SettingsSection::ALL {
            items.push(PaletteItem::Settings(section));
        }
        for project in self.visible_projects() {
            items.push(PaletteItem::Project(
                project.id.clone(),
                project.name.clone(),
            ));
        }
        for session in self.state.sessions.iter().filter(|s| s.lifecycle.live()) {
            items.push(PaletteItem::Session(
                session.id.clone(),
                session.label.clone(),
            ));
        }
        let mut files: Vec<_> = self
            .dirs
            .values()
            .flatten()
            .filter(|entry| !entry.directory)
            .map(|entry| entry.path.clone())
            .collect();
        if let Some(context) = &self.context {
            files.extend(context.changes.iter().map(|change| change.path.clone()));
        }
        files.sort();
        files.dedup();
        items.extend(files.into_iter().map(PaletteItem::File));
        items
    }

    pub(super) fn filtered_palette(&self) -> Vec<PaletteItem> {
        let query = self.palette_query.trim().to_lowercase();
        let mut files = 0;
        self.palette_items()
            .into_iter()
            .filter(|item| query.is_empty() || item.label().to_lowercase().contains(&query))
            .filter(|item| {
                if matches!(item, PaletteItem::File(_)) {
                    files += 1;
                    files <= 40
                } else {
                    true
                }
            })
            .collect()
    }

    pub(super) fn palette(&mut self, ctx: &egui::Context) {
        let mut open = true;
        self.popups
            .window(ctx, "Command palette")
            .open(&mut open)
            .collapsible(false)
            .default_size([520.0, 360.0])
            .show(ctx, |ui| {
                ui.label("Jump to a project, session, file, or setting.");
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .hint_text("Filter…")
                        .desired_width(f32::INFINITY),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "palette-search", search.rect);
                if search.changed() {
                    self.palette_index = 0;
                }
                search.request_focus();
                let items = self.filtered_palette();
                if items.is_empty() {
                    ui.weak("No matching commands.");
                    return;
                }
                self.palette_index = self.palette_index.min(items.len() - 1);
                if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
                    self.palette_index = (self.palette_index + 1) % items.len();
                }
                if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
                    self.palette_index = (self.palette_index + items.len() - 1) % items.len();
                }
                let mut chosen = None;
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for (index, item) in items.iter().enumerate() {
                            let selected = index == self.palette_index;
                            let response = ui.selectable_label(selected, item.label());
                            #[cfg(feature = "test-support")]
                            diagnostics::record(
                                ui.ctx(),
                                &format!("palette-item:{}", item.label()),
                                response.rect,
                            );
                            if response.clicked() {
                                chosen = Some(index);
                            }
                        }
                    });
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    chosen = Some(self.palette_index);
                }
                if let Some(index) = chosen
                    && let Some(item) = items.get(index).cloned()
                {
                    self.run_palette(item);
                }
            });
        self.palette_open &= open;
        if !self.palette_open {
            self.palette_query.clear();
            self.palette_index = 0;
        }
    }

    fn run_palette(&mut self, item: PaletteItem) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_index = 0;
        match item {
            PaletteItem::Project(id, _) => self.select_project(id),
            PaletteItem::Session(id, _) => self.go_session(&id),
            PaletteItem::File(path) => self.open_file(path, None, None, false),
            PaletteItem::Settings(section) => {
                self.open_settings();
                self.settings_section = section;
            }
            PaletteItem::AddProject => self.add_project = true,
            PaletteItem::NewTerminal => self.create(None),
            PaletteItem::NewWorktree => self.open_worktree_wizard(),
            PaletteItem::OpenPlayer => {
                if let Some(project) = self.selected.clone() {
                    self.open_player_tab(&project);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_filter_runs_before_the_result_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        app.dirs.insert(
            dir.path().into(),
            (0..50)
                .map(|index| services::Entry {
                    path: dir.path().join(format!("file-{index:02}.rs")),
                    directory: false,
                    ignored: false,
                })
                .collect(),
        );
        assert_eq!(
            app.filtered_palette()
                .iter()
                .filter(|item| matches!(item, PaletteItem::File(_)))
                .count(),
            40
        );
        app.palette_query = "file-49.rs".into();
        let matches = app.filtered_palette();
        assert!(
            matches!(matches.as_slice(), [PaletteItem::File(path)] if path.ends_with("file-49.rs"))
        );
    }

    #[test]
    fn palette_labels_are_stable() {
        assert_eq!(PaletteItem::AddProject.label(), "Add project");
        assert_eq!(
            PaletteItem::Settings(SettingsSection::Terminal).label(),
            "Settings  Terminal & Editor"
        );
    }
}
