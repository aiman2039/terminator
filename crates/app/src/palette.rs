//! Search Everywhere across projects, files, symbols, sessions, agents,
//! actions, and settings. File scans and LSP symbol requests stay async.
use super::*;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};
use terminator_native_edit::finder::{MAX_INDEX_FILES, spawn_scan};
use terminator_native_edit::lsp as native_lsp;

const MAX_RESULTS: usize = 80;
const FILE_RESULTS: usize = 40;
const SYMBOL_QUERY_DELAY: Duration = Duration::from_millis(180);

fn compact_path(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.len() <= 4 {
        return path.display().to_string();
    }
    format!(
        "…/{}",
        parts
            .iter()
            .rev()
            .take(4)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("/")
    )
}

pub(crate) struct PaletteCache {
    query: String,
    state_revision: u64,
    file_index_at: Option<Instant>,
    lsp_revision: u64,
    items: Vec<PaletteItem>,
}

#[derive(Clone, Debug)]
pub(crate) enum PaletteItem {
    Project(String, String),
    Session(String, String),
    Agent(String, String, String),
    File(PathBuf),
    Symbol {
        path: PathBuf,
        name: String,
        kind: String,
        line: u32,
        container: Option<String>,
    },
    Settings(SettingsSection),
    Action(&'static str, &'static str),
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
    fn category(&self) -> &'static str {
        match self {
            Self::Project(..) => "Project",
            Self::Session(..) => "Session",
            Self::Agent(..) => "Agent",
            Self::File(..) => "File",
            Self::Symbol { .. } => "Symbol",
            Self::Settings(..) => "Settings",
            Self::ApplyLayout(..) => "Layout",
            _ => "Action",
        }
    }

    fn title(&self) -> String {
        match self {
            Self::Project(_, name) | Self::Session(_, name) => name.clone(),
            Self::Agent(_, name, _) => name.clone(),
            Self::File(path) => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Self::Symbol { name, .. } => name.clone(),
            Self::Settings(section) => section.title().into(),
            Self::Action(_, name) => (*name).into(),
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
            Self::ApplyLayout(name) => name.clone(),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::AddProject
            | Self::NewTerminal
            | Self::NewWorktree
            | Self::OpenPlayer
            | Self::ToggleIdeMode
            | Self::MoveToMain
            | Self::MoveToStrip
            | Self::ToggleIdeSidebarHeight
            | Self::NextAttention
            | Self::SaveLayout => self.title(),
            _ => format!("{}  {}", self.category(), self.title()),
        }
    }

    fn detail(&self, app: &App) -> String {
        match self {
            Self::File(path) | Self::Symbol { path, .. } => {
                let project = app.project_for_palette_path(path);
                let relative = project.and_then(|project| path.strip_prefix(&project.path).ok());
                let name = project.map_or("Other files", |project| project.name.as_str());
                let path = compact_path(relative.unwrap_or(path));
                if let Self::Symbol {
                    kind,
                    line,
                    container,
                    ..
                } = self
                {
                    let container = container
                        .as_deref()
                        .map_or(String::new(), |name| format!(" · {name}"));
                    format!(
                        "{name} · {path}:{} · {kind}{container}",
                        line.saturating_add(1)
                    )
                } else {
                    format!("{name} · {path}")
                }
            }
            Self::Agent(id, _, summary) => {
                let project = app
                    .state
                    .sessions
                    .iter()
                    .find(|session| &session.id == id)
                    .and_then(|session| {
                        app.state
                            .projects
                            .iter()
                            .find(|project| project.id == session.project_id)
                    })
                    .map_or("", |project| project.name.as_str());
                format!("{project} · {summary}")
            }
            Self::Session(id, _) => app
                .state
                .sessions
                .iter()
                .find(|session| &session.id == id)
                .and_then(|session| {
                    app.state
                        .projects
                        .iter()
                        .find(|project| project.id == session.project_id)
                })
                .map_or_else(String::new, |project| project.name.clone()),
            Self::Project(id, _) => app
                .state
                .projects
                .iter()
                .find(|project| &project.id == id)
                .map_or_else(String::new, |project| project.path.display().to_string()),
            Self::Settings(section) => section.keywords().into(),
            Self::Action(key, _) => app.shortcut_label(key),
            _ => String::new(),
        }
    }

    fn search_text(&self, app: &App) -> String {
        let mut text = self.label();
        text.push(' ');
        text.push_str(&self.detail(app));
        if let Self::Symbol { kind, .. } = self {
            text.push(' ');
            text.push_str(kind);
        }
        if let Self::File(path) | Self::Symbol { path, .. } = self {
            text.push(' ');
            text.push_str(&path.to_string_lossy());
        }
        text
    }

    fn kind_rank(&self) -> u8 {
        match self {
            Self::Agent(..) => 0,
            Self::Symbol { .. } => 1,
            Self::File(..) => 2,
            Self::Project(..) => 3,
            Self::Session(..) => 4,
            Self::Settings(..) => 5,
            _ => 6,
        }
    }
}

impl App {
    fn project_for_palette_path(&self, path: &Path) -> Option<&Project> {
        self.state
            .projects
            .iter()
            .filter(|project| {
                self.project_available(&project.id) && path.starts_with(&project.path)
            })
            .max_by_key(|project| project.path.components().count())
    }

    pub(super) fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items = Vec::new();
        let mut notices: Vec<_> = self
            .state
            .notifications
            .iter()
            .filter(|notice| {
                matches!(
                    notice.state,
                    AgentState::WaitingInput | AgentState::WaitingPermission
                ) && agent_presence::notice_pending(notice, now())
            })
            .collect();
        notices.sort_by_key(|notice| std::cmp::Reverse(notice.created));
        let mut seen_agents = HashSet::new();
        for notice in notices {
            if let Some(session) = self
                .state
                .sessions
                .iter()
                .find(|session| session.id == notice.session_id && session.lifecycle.live())
                && seen_agents.insert(&session.id)
            {
                let summary: String = notice
                    .summary
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect();
                items.push(PaletteItem::Agent(
                    session.id.clone(),
                    session.label.clone(),
                    summary,
                ));
            }
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
        for (path, symbol) in self.lsp.all_document_symbols() {
            if self.project_for_palette_path(&path).is_some() {
                items.push(PaletteItem::Symbol {
                    path,
                    name: symbol.name,
                    kind: symbol.kind,
                    line: symbol.range.start.line,
                    container: None,
                });
            }
        }
        let mut seen_symbols: HashSet<(PathBuf, String, u32)> = items
            .iter()
            .filter_map(|item| match item {
                PaletteItem::Symbol {
                    path, name, line, ..
                } => Some((path.clone(), name.clone(), *line)),
                _ => None,
            })
            .collect();
        for hit in self.lsp.workspace_symbols() {
            let Some(path) = native_lsp::uri_to_path(&hit.uri) else {
                continue;
            };
            if self.project_for_palette_path(&path).is_some()
                && seen_symbols.insert((path.clone(), hit.name.clone(), hit.range.start.line))
            {
                items.push(PaletteItem::Symbol {
                    path,
                    name: hit.name,
                    kind: hit.kind,
                    line: hit.range.start.line,
                    container: hit.container,
                });
            }
        }
        for section in SettingsSection::ALL {
            items.push(PaletteItem::Settings(section));
        }
        items.extend([
            PaletteItem::AddProject,
            PaletteItem::NewTerminal,
            PaletteItem::NextAttention,
            PaletteItem::OpenPlayer,
            PaletteItem::ToggleIdeMode,
            PaletteItem::MoveToMain,
            PaletteItem::MoveToStrip,
            PaletteItem::ToggleIdeSidebarHeight,
            PaletteItem::SaveLayout,
        ]);
        const EXPLICIT: &[&str] = &[
            "new_terminal",
            "next_attention",
            "toggle_ide_mode",
            "move_to_main",
            "move_to_strip",
            "open_settings",
            "open_palette",
        ];
        const SESSION_ACTIONS: &[&str] = &[
            "select_all",
            "find_in_terminal",
            "search_scrollback",
            "copy_working_directory",
            "rename_terminal",
            "close_session",
            "clear_scrollback",
        ];
        for &(key, name) in shortcuts::ACTIONS {
            if !EXPLICIT.contains(&key)
                && self.shortcut_applies(key)
                && (!SESSION_ACTIONS.contains(&key) || self.active_session.is_some())
            {
                items.push(PaletteItem::Action(key, name));
            }
        }
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
        if self.file_index.is_empty() {
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
        }
        items
    }

    /// Watched directory roots driving the file index, sorted.
    fn file_index_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = self
            .visible_projects()
            .iter()
            .map(|project| project.path.clone())
            .collect();
        if roots.is_empty() {
            roots.extend(self.dirs.keys().cloned());
        }
        roots.sort();
        roots.dedup();
        roots
    }

    /// Kick a background index rescan when roots changed or the index is
    /// older than 30s; at most one scan flies at a time.
    pub(super) fn refresh_file_index(&mut self) {
        if self.file_index_rx.is_some() {
            return;
        }
        let roots = self.file_index_roots();
        if roots != self.file_index_roots {
            self.file_index = terminator_native_edit::finder::FileIndex::empty();
            self.file_index_at = None;
        }
        let stale = self
            .file_index_at
            .is_none_or(|at| at.elapsed().as_secs() >= 30);
        if roots == self.file_index_roots && !stale {
            return;
        }
        self.file_index_roots.clone_from(&roots);
        self.file_index_rx = Some(spawn_scan(roots, MAX_INDEX_FILES));
    }

    /// Adopt a finished background scan (non-blocking).
    pub(super) fn poll_file_index(&mut self) {
        if let Some(rx) = self.file_index_rx.as_ref() {
            match rx.try_recv() {
                Ok(index) => {
                    self.file_index = index;
                    self.file_index_at = Some(std::time::Instant::now());
                    self.file_index_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.file_index_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    pub(super) fn filtered_palette(&self) -> Vec<PaletteItem> {
        let query = self.palette_query.trim();
        let items = self.palette_items();
        if query.is_empty() {
            let mut out: Vec<_> = items
                .iter()
                .filter(|item| !matches!(item, PaletteItem::File(_)))
                .take(MAX_RESULTS.saturating_sub(FILE_RESULTS))
                .cloned()
                .collect();
            if self.file_index.is_empty() {
                out.extend(
                    items
                        .into_iter()
                        .filter(|item| matches!(item, PaletteItem::File(_)))
                        .take(FILE_RESULTS),
                );
            } else {
                out.extend(
                    self.file_index
                        .query("", FILE_RESULTS)
                        .into_iter()
                        .map(PaletteItem::File),
                );
            }
            return out;
        }

        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut needle_buf = Vec::new();
        let needle = nucleo::Utf32Str::new(query, &mut needle_buf);
        let mut haystack_buf = Vec::new();
        let mut ranked: Vec<(u16, u8, String, PaletteItem)> = Vec::new();
        for item in items {
            let text = item.search_text(self);
            let haystack = nucleo::Utf32Str::new(&text, &mut haystack_buf);
            if let Some(score) = matcher.fuzzy_match(haystack, needle) {
                let title = item.title();
                let boost = if title.eq_ignore_ascii_case(query) {
                    80
                } else if title.to_lowercase().starts_with(&query.to_lowercase()) {
                    25
                } else {
                    0
                };
                ranked.push((score.saturating_add(boost), item.kind_rank(), title, item));
            }
        }
        if !self.file_index.is_empty() {
            for (score, file) in self.file_index.query_scored(query, FILE_RESULTS) {
                let item = PaletteItem::File(file.path);
                let title = item.title();
                let boost = if title.eq_ignore_ascii_case(query) {
                    80
                } else {
                    0
                };
                ranked.push((score.saturating_add(boost), item.kind_rank(), title, item));
            }
        }
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        ranked
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, _, _, item)| item)
            .collect()
    }

    pub(super) fn palette_center(&mut self, ui: &mut egui::Ui) {
        // Adopt finished background scans while the palette is open.
        self.poll_file_index();
        self.lsp.poll();
        self.lsp.request_open_document_symbols();
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Search Everywhere");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.close_palette();
                }
            });
        });
        ui.add_space(8.0);
        ui.weak("Files · Symbols · Projects · Sessions · Agents · Actions · Settings");
        let search = ui.add(
            appearance::singleline(&mut self.palette_query)
                .hint_text("Search everywhere…")
                .desired_width(f32::INFINITY),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "palette-search", search.rect);
        if search.changed() {
            self.palette_index = 0;
            self.palette_query_changed_at = Instant::now();
            self.lsp.clear_workspace_symbols();
        }
        search.request_focus();
        if self.palette_query.trim().chars().count() >= 2 {
            if self.palette_query_changed_at.elapsed() >= SYMBOL_QUERY_DELAY {
                self.lsp.request_workspace_symbols(&self.palette_query);
            } else {
                ui.ctx().request_repaint_after(SYMBOL_QUERY_DELAY);
            }
        }
        if self.lsp.workspace_symbols_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        let stale = self.palette_cache.as_ref().is_none_or(|cache| {
            cache.query != self.palette_query
                || cache.state_revision != self.state.revision
                || cache.file_index_at != self.file_index_at
                || cache.lsp_revision != self.lsp.search_revision()
        });
        if stale {
            self.palette_cache = Some(PaletteCache {
                query: self.palette_query.clone(),
                state_revision: self.state.revision,
                file_index_at: self.file_index_at,
                lsp_revision: self.lsp.search_revision(),
                items: self.filtered_palette(),
            });
        }
        let items = self
            .palette_cache
            .as_ref()
            .map_or_else(Vec::new, |cache| cache.items.clone());
        if items.is_empty() {
            ui.weak(if self.lsp.workspace_symbols_pending() {
                "Searching symbols…"
            } else {
                "No matches."
            });
            return;
        }
        let len = items.len();
        self.palette_index = self.palette_index.min(len.saturating_sub(1));
        let mut keyboard_moved = false;
        if ui.input(|input| input.key_pressed(egui::Key::ArrowDown))
            && let Some(next) = self
                .palette_index
                .checked_add(1)
                .and_then(|index| index.checked_rem(len))
        {
            self.palette_index = next;
            keyboard_moved = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            self.palette_index = self
                .palette_index
                .checked_sub(1)
                .unwrap_or_else(|| len.saturating_sub(1));
            keyboard_moved = true;
        }
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, item) in items.iter().enumerate() {
                    let selected = index == self.palette_index;
                    let response = ui
                        .horizontal(|ui| {
                            let response = ui.selectable_label(selected, item.label());
                            let detail = item.detail(self);
                            if !detail.is_empty() {
                                ui.weak(egui::RichText::new(detail).small());
                            }
                            response
                        })
                        .inner;
                    if selected && keyboard_moved {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
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
            self.run_palette(ui.ctx(), item);
        }
    }

    fn close_palette(&mut self) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_index = 0;
        self.palette_cache = None;
        self.lsp.clear_workspace_symbols();
    }

    fn run_palette(&mut self, ctx: &egui::Context, item: PaletteItem) {
        self.close_palette();
        match item {
            PaletteItem::Project(id, _) => self.select_project(id),
            PaletteItem::Session(id, _) => self.go_session(&id),
            PaletteItem::Agent(id, _, _) => self.go_session(&id),
            PaletteItem::File(path) => {
                if let Some(project) = self
                    .project_for_palette_path(&path)
                    .map(|project| project.id.clone())
                    && self.selected.as_ref() != Some(&project)
                {
                    self.select_project(project);
                }
                self.open_file(path, None, None, false);
            }
            PaletteItem::Symbol { path, line, .. } => {
                if let Some(project) = self
                    .project_for_palette_path(&path)
                    .map(|project| project.id.clone())
                    && self.selected.as_ref() != Some(&project)
                {
                    self.select_project(project);
                }
                self.open_file(path, line.checked_add(1), None, false);
            }
            PaletteItem::Settings(section) => {
                self.open_settings();
                self.request_settings_section(section);
            }
            PaletteItem::AddProject => self.add_project = true,
            PaletteItem::Action(key, _) => self.run_shortcut(ctx, key),
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
    fn file_search_reaches_entries_after_the_empty_result_limit() {
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
            matches.iter().any(
                |item| matches!(item, PaletteItem::File(path) if path.ends_with("file-49.rs"))
            )
        );
    }

    #[test]
    fn fuzzy_index_ranks_go_to_file() {
        use terminator_native_edit::finder::FileIndex;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();
        std::fs::write(root.join("src/lib.rs"), "").unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        app.file_index = FileIndex::scan(&[root], MAX_INDEX_FILES);
        app.palette_query = "main".into();
        let files: Vec<PathBuf> = app
            .filtered_palette()
            .into_iter()
            .filter_map(|item| match item {
                PaletteItem::File(path) => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("src/main.rs"));
    }

    #[test]
    fn background_refresh_delivers_an_index() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        app.dirs.insert(
            dir.path().into(),
            vec![terminator_git::Entry {
                path: dir.path().join("a.rs"),
                directory: false,
                ignored: false,
            }],
        );
        app.refresh_file_index();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.file_index_rx.is_some() && std::time::Instant::now() < deadline {
            app.poll_file_index();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!app.file_index.is_empty());
        // A second refresh with unchanged roots and a fresh index is a no-op.
        app.refresh_file_index();
        assert!(app.file_index_rx.is_none());
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
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        assert!(app.preferences.ide_sidebars_full_height);
        app.run_palette(&ctx, PaletteItem::ToggleIdeSidebarHeight);
        assert!(!app.preferences.ide_sidebars_full_height);
        app.run_palette(&ctx, PaletteItem::ToggleIdeSidebarHeight);
        assert!(app.preferences.ide_sidebars_full_height);
    }

    #[test]
    fn fuzzy_list_finds_settings_keywords_and_shortcut_actions() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        app.palette_query = "font".into();
        assert!(app.filtered_palette().iter().any(|item| matches!(
            item,
            PaletteItem::Settings(SettingsSection::Appearance | SettingsSection::Terminal)
        )));
        app.palette_query = "open file".into();
        assert!(
            app.filtered_palette()
                .iter()
                .any(|item| matches!(item, PaletteItem::Action("open_file", _)))
        );
    }

    #[test]
    fn waiting_agent_is_a_direct_result_and_resolved_notice_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        app.state.projects.push(Project {
            id: "a".into(),
            name: "Alpha".into(),
            path: "/a".into(),
            layout: serde_json::Value::Null,
        });
        app.state.sessions.push(Session {
            id: "shell".into(),
            project_id: "a".into(),
            label: "Agent terminal".into(),
            cwd: "/a".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            review: false,
            created: 1,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "test".into(),
            pid: Some(1),
            truncated: false,
            cwd_confirmed: true,
        });
        app.state.notifications.push(Notification {
            id: "notice".into(),
            session_id: "shell".into(),
            invocation_id: "inv".into(),
            request_id: Some("request".into()),
            state: AgentState::WaitingPermission,
            summary: "Permission needed".into(),
            details: String::new(),
            created: 2,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        });
        app.palette_query = "permission".into();
        assert!(
            app.filtered_palette()
                .iter()
                .any(|item| matches!(item, PaletteItem::Agent(id, _, _) if id == "shell"))
        );
        app.state.notifications[0].resolved = true;
        assert!(
            !app.filtered_palette()
                .iter()
                .any(|item| matches!(item, PaletteItem::Agent(..)))
        );
    }

    #[test]
    fn file_and_symbol_results_open_in_their_own_project() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        for id in ["a", "b"] {
            app.state.projects.push(Project {
                id: id.into(),
                name: id.into(),
                path: PathBuf::from(format!("/{id}")),
                layout: serde_json::Value::Null,
            });
        }
        app.selected = Some("a".into());
        let file = PathBuf::from("/b/src/main.rs");
        app.run_palette(&ctx, PaletteItem::File(file.clone()));
        assert_eq!(app.selected.as_deref(), Some("b"));
        app.selected = Some("a".into());
        app.state.settings.editor_mode = EditorMode::Native;
        app.run_palette(
            &ctx,
            PaletteItem::Symbol {
                path: file.clone(),
                name: "helper".into(),
                kind: "Function".into(),
                line: 8,
                container: None,
            },
        );
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert_eq!(app.native_pending_line.get(&file), Some(&8));
    }
}
