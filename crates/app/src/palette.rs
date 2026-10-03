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
    ToggleIdeMode,
    MoveToMain,
    MoveToStrip,
    ToggleIdeSidebarHeight,
    NextAttention,
    SaveLayout,
    ApplyLayout(String),
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
            Self::ToggleIdeMode => "Toggle IDE mode".into(),
            Self::MoveToMain => "Move terminal to main pane".into(),
            Self::MoveToStrip => "Move terminal to lower pane".into(),
            Self::ToggleIdeSidebarHeight => "Toggle IDE sidebar height".into(),
            Self::NextAttention => "Next agent needing attention".into(),
            Self::SaveLayout => "Save layout".into(),
            Self::ApplyLayout(name) => format!("Layout  {name}"),
        }
    }
}

impl App {
    pub(super) fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items = vec![
            PaletteItem::AddProject,
            PaletteItem::NewTerminal,
            PaletteItem::NextAttention,
            PaletteItem::OpenPlayer,
            PaletteItem::ToggleIdeMode,
            PaletteItem::MoveToMain,
            PaletteItem::MoveToStrip,
            PaletteItem::ToggleIdeSidebarHeight,
            PaletteItem::SaveLayout,
        ];
        for name in self.preferences.named_layouts.keys() {
            items.push(PaletteItem::ApplyLayout(name.clone()));
        }
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
        let mut files: i32 = 0;
        self.palette_items()
            .into_iter()
            .filter(|item| query.is_empty() || item.label().to_lowercase().contains(&query))
            .filter(|item| {
                if matches!(item, PaletteItem::File(_)) {
                    files = files.saturating_add(1);
                    files <= 40
                } else {
                    true
                }
            })
            .collect()
    }

    pub(super) fn palette_center(&mut self, ui: &mut egui::Ui) {
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Command palette");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.close_palette();
                }
            });
        });
        ui.add_space(8.0);
        ui.label("Jump to a project, session, file, or setting.");
        let search = ui.add(
            appearance::singleline(&mut self.palette_query)
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
        let len = items.len();
        self.palette_index = self.palette_index.min(len.saturating_sub(1));
        if ui.input(|input| input.key_pressed(egui::Key::ArrowDown))
            && let Some(next) = self
                .palette_index
                .checked_add(1)
                .and_then(|index| index.checked_rem(len))
        {
            self.palette_index = next;
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            self.palette_index = self
                .palette_index
                .checked_sub(1)
                .unwrap_or_else(|| len.saturating_sub(1));
        }
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
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
    }

    fn close_palette(&mut self) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_index = 0;
    }

    fn run_palette(&mut self, item: PaletteItem) {
        self.close_palette();
        match item {
            PaletteItem::Project(id, _) => self.select_project(id),
            PaletteItem::Session(id, _) => self.go_session(&id),
            PaletteItem::File(path) => self.open_file(path, None, None, false),
            PaletteItem::Settings(section) => {
                self.open_settings();
                self.request_settings_section(section);
            }
            PaletteItem::AddProject => self.add_project = true,
            PaletteItem::NewTerminal => self.create(None),
            PaletteItem::NextAttention => self.next_attention(),
            PaletteItem::NewWorktree => self.open_worktree_wizard(),
            PaletteItem::OpenPlayer => self.open_player(),
            PaletteItem::ToggleIdeMode => self.toggle_ide_mode(),
            PaletteItem::MoveToMain => {
                if let Some(sid) = self.focused_strip_shell() {
                    self.move_strip_session_to_main(&sid);
                }
            }
            PaletteItem::MoveToStrip => {
                if self.preferences.ide_mode
                    && let Some(sid) = self.focused_main_shell()
                {
                    self.move_main_session_to_strip(&sid);
                }
            }
            PaletteItem::ToggleIdeSidebarHeight => self.toggle_ide_sidebar_height(),
            PaletteItem::SaveLayout => self.save_layout_dialog(),
            PaletteItem::ApplyLayout(name) => {
                self.preferences.apply_layout(&name);
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
                .map(|index| terminator_git::Entry {
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
        assert_eq!(PaletteItem::ToggleIdeMode.label(), "Toggle IDE mode");
        assert_eq!(
            PaletteItem::MoveToMain.label(),
            "Move terminal to main pane"
        );
        assert_eq!(
            PaletteItem::MoveToStrip.label(),
            "Move terminal to lower pane"
        );
        assert_eq!(
            PaletteItem::ToggleIdeSidebarHeight.label(),
            "Toggle IDE sidebar height"
        );
        assert_eq!(
            PaletteItem::NextAttention.label(),
            "Next agent needing attention"
        );
        assert_eq!(
            PaletteItem::Settings(SettingsSection::Terminal).label(),
            "Settings  Terminal & Editor"
        );
    }

    #[test]
    fn palette_toggles_ide_sidebar_height() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        assert!(app.preferences.ide_sidebars_full_height);
        app.run_palette(PaletteItem::ToggleIdeSidebarHeight);
        assert!(!app.preferences.ide_sidebars_full_height);
        app.run_palette(PaletteItem::ToggleIdeSidebarHeight);
        assert!(app.preferences.ide_sidebars_full_height);
    }
}
