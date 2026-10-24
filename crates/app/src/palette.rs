//! Search the current project: files, symbols, sessions, and agents.
//! Actions and settings stay global. The Projects filter lists every
//! project. File scans and LSP symbol requests stay async.
use super::*;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};
use terminator_native_edit::finder::{MAX_INDEX_FILES, spawn_scan};
use terminator_native_edit::lsp as native_lsp;

const MAX_RESULTS: usize = 80;
const FILE_RESULTS: usize = 40;
/// Recent files shown before sessions when the field is empty.
const RECENT_RESULTS: usize = 8;
const SYMBOL_QUERY_DELAY: Duration = Duration::from_millis(180);

const FILTER_FILES: u8 = 1 << 0;
const FILTER_SYMBOLS: u8 = 1 << 1;
const FILTER_PROJECTS: u8 = 1 << 2;
const FILTER_SESSIONS: u8 = 1 << 3;
const FILTER_AGENTS: u8 = 1 << 4;
const FILTER_ACTIONS: u8 = 1 << 5;
const FILTER_SETTINGS: u8 = 1 << 6;

const PALETTE_FILTERS: &[(&str, u8)] = &[
    ("Files", FILTER_FILES),
    ("Symbols", FILTER_SYMBOLS),
    ("Projects", FILTER_PROJECTS),
    ("Sessions", FILTER_SESSIONS),
    ("Agents", FILTER_AGENTS),
    ("Actions", FILTER_ACTIONS),
    ("Settings", FILTER_SETTINGS),
];

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
    filters: u8,
    project: Option<String>,
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

    fn filter_bit(&self) -> u8 {
        match self {
            Self::File(_) => FILTER_FILES,
            Self::Symbol { .. } => FILTER_SYMBOLS,
            Self::Project(..) => FILTER_PROJECTS,
            Self::Session(..) => FILTER_SESSIONS,
            Self::Agent(..) => FILTER_AGENTS,
            Self::Settings(_) => FILTER_SETTINGS,
            _ => FILTER_ACTIONS,
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

    fn palette_project_root(&self) -> Option<&Path> {
        self.selected_project()
            .map(|project| project.path.as_path())
    }

    /// Other projects whose path sits inside the selected project.
    fn palette_exclude_roots(&self) -> Vec<PathBuf> {
        let Some(selected) = self.selected_project() else {
            return Vec::new();
        };
        self.state
            .projects
            .iter()
            .filter(|project| {
                project.id != selected.id
                    && self.project_available(&project.id)
                    && project.path.starts_with(&selected.path)
            })
            .map(|project| project.path.clone())
            .collect()
    }

    /// The file belongs to the selected project, not to a nested one.
    fn path_in_palette_project(&self, path: &Path) -> bool {
        let Some(selected) = self.selected_project() else {
            return false;
        };
        self.project_for_palette_path(path)
            .is_some_and(|owner| owner.id == selected.id)
    }

    /// No chip: every kind in the current project, except the project list.
    /// The Projects chip is the way to open another project.
    fn palette_wants(&self, kind: u8) -> bool {
        if self.palette_filters == 0 {
            kind != FILTER_PROJECTS
        } else {
            self.palette_filters & kind != 0
        }
    }

    fn session_in_palette_project(&self, id: &str) -> bool {
        let Some(project) = self.selected_project() else {
            return false;
        };
        self.state
            .sessions
            .iter()
            .any(|session| session.id == id && session.project_id == project.id)
    }

    fn palette_accept(&self, item: &PaletteItem) -> bool {
        let kind = item.filter_bit();
        if !self.palette_wants(kind) {
            return false;
        }
        // An empty field shows files, sessions, and agents. Actions,
        // settings, and symbols appear after the user types, or when
        // that filter is on.
        if self.palette_query.trim().is_empty()
            && self.palette_filters == 0
            && matches!(kind, FILTER_ACTIONS | FILTER_SETTINGS | FILTER_SYMBOLS)
        {
            return false;
        }
        match item {
            PaletteItem::File(path) | PaletteItem::Symbol { path, .. } => {
                self.path_in_palette_project(path)
            }
            PaletteItem::Session(id, _) | PaletteItem::Agent(id, _, _) => {
                self.session_in_palette_project(id)
            }
            _ => true,
        }
    }

    /// Empty field: a few recent files, then live sessions and agents,
    /// then more files from this project.
    fn idle_palette(&self, items: Vec<PaletteItem>) -> Vec<PaletteItem> {
        let mut out = Vec::new();
        let mut seen_files = HashSet::new();
        if self.palette_wants(FILTER_FILES) {
            for path in &self.preferences.recent_native_files {
                if !self.path_in_palette_project(path) {
                    continue;
                }
                if seen_files.insert(path.clone()) {
                    out.push(PaletteItem::File(path.clone()));
                }
                if seen_files.len() >= RECENT_RESULTS {
                    break;
                }
            }
        }
        out.extend(
            items
                .iter()
                .filter(|item| {
                    self.palette_accept(item)
                        && matches!(item, PaletteItem::Session(..) | PaletteItem::Agent(..))
                })
                .cloned(),
        );
        if self.palette_filters != 0 {
            out.extend(
                items
                    .iter()
                    .filter(|item| {
                        self.palette_accept(item)
                            && !matches!(
                                item,
                                PaletteItem::File(_)
                                    | PaletteItem::Session(..)
                                    | PaletteItem::Agent(..)
                            )
                    })
                    .cloned(),
            );
        }
        if self.palette_wants(FILTER_FILES) {
            let room = FILE_RESULTS.saturating_sub(seen_files.len());
            if self.file_index.is_empty() {
                out.extend(
                    items
                        .into_iter()
                        .filter(|item| {
                            let PaletteItem::File(path) = item else {
                                return false;
                            };
                            self.palette_accept(item) && seen_files.insert(path.clone())
                        })
                        .take(room),
                );
            } else if let Some(root) = self.palette_project_root() {
                let exclude = self.palette_exclude_roots();
                out.extend(
                    self.file_index
                        .query_scored_in(Some(root), &exclude, "", room)
                        .into_iter()
                        .map(|(_, file)| file.path)
                        .filter(|path| seen_files.insert(path.clone()))
                        .map(PaletteItem::File),
                );
            }
        }
        out.truncate(MAX_RESULTS);
        out
    }

    pub(super) fn filtered_palette(&self) -> Vec<PaletteItem> {
        let query = self.palette_query.trim();
        let items = self.palette_items();
        if query.is_empty() {
            return self.idle_palette(items);
        }

        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut needle_buf = Vec::new();
        let needle = nucleo::Utf32Str::new(query, &mut needle_buf);
        let mut haystack_buf = Vec::new();
        let mut ranked: Vec<(u16, u8, String, PaletteItem)> = Vec::new();
        for item in items {
            if !self.palette_accept(&item) {
                continue;
            }
            // A ready index ranks files on its own, under the current project.
            if !self.file_index.is_empty() && matches!(item, PaletteItem::File(_)) {
                continue;
            }
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
        if self.palette_wants(FILTER_FILES)
            && let Some(root) = self.palette_project_root()
            && !self.file_index.is_empty()
        {
            let exclude = self.palette_exclude_roots();
            for (score, file) in
                self.file_index
                    .query_scored_in(Some(root), &exclude, query, FILE_RESULTS)
            {
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
        let project_name = self
            .selected_project()
            .map(|project| project.name.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.strong("Search");
            if !project_name.is_empty() {
                ui.weak(format!("· {project_name}"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.close_palette();
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (name, bit) in PALETTE_FILTERS {
                let on = self.palette_filters & bit != 0;
                let response = ui
                    .add(
                        egui::Button::new(*name)
                            .small()
                            .selected(on)
                            .frame(true)
                            .corner_radius(egui::CornerRadius::same(appearance::WIDGET_RADIUS)),
                    )
                    .on_hover_text(format!("Show {name}"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), &format!("palette-filter:{name}"), response.rect);
                if response.clicked() {
                    if on {
                        self.palette_filters &= !bit;
                    } else {
                        self.palette_filters |= bit;
                    }
                    self.palette_index = 0;
                }
            }
        });
        let hint = if project_name.is_empty() {
            "Search…".to_string()
        } else {
            format!("Search {project_name}…")
        };
        let search = ui.add(
            appearance::singleline(&mut self.palette_query)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "palette-search", search.rect);
        if search.changed() {
            self.palette_index = 0;
            self.palette_query_changed_at = Instant::now();
            self.lsp.clear_workspace_symbols();
        }
        // A pressed pointer is choosing a filter. Focus returns on release
        // so that click is not taken by the field.
        if !ui.input(|input| input.pointer.primary_down()) {
            search.request_focus();
        }
        if self.palette_wants(FILTER_SYMBOLS) {
            if self.palette_query.trim().chars().count() >= 2 {
                if self.palette_query_changed_at.elapsed() >= SYMBOL_QUERY_DELAY {
                    let scoped_path = self.selected_project().map(|project| project.path.clone());
                    if let Some(path) = scoped_path.as_deref() {
                        self.lsp
                            .request_scoped_symbols(&self.palette_query, Some(path));
                    } else {
                        self.lsp.clear_workspace_symbols();
                    }
                } else {
                    ui.ctx().request_repaint_after(SYMBOL_QUERY_DELAY);
                }
            }
        } else {
            self.lsp.clear_workspace_symbols();
        }
        if self.lsp.workspace_symbols_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        let project_id = self.selected_project().map(|project| project.id.clone());
        let stale = self.palette_cache.as_ref().is_none_or(|cache| {
            cache.query != self.palette_query
                || cache.state_revision != self.state.revision
                || cache.file_index_at != self.file_index_at
                || cache.lsp_revision != self.lsp.search_revision()
                || cache.filters != self.palette_filters
                || cache.project != project_id
        });
        if stale {
            self.palette_cache = Some(PaletteCache {
                query: self.palette_query.clone(),
                state_revision: self.state.revision,
                file_index_at: self.file_index_at,
                lsp_revision: self.lsp.search_revision(),
                filters: self.palette_filters,
                project: project_id,
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
        self.palette_filters = 0;
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
        app.state.projects.push(Project {
            id: "p".into(),
            name: "p".into(),
            path: dir.path().into(),
            layout: serde_json::Value::Null,
        });
        app.selected = Some("p".into());
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
        app.state.projects.push(Project {
            id: "p".into(),
            name: "p".into(),
            path: root.clone(),
            layout: serde_json::Value::Null,
        });
        app.selected = Some("p".into());
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
        app.selected = Some("a".into());
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
                path: dir.path().join(id),
                layout: serde_json::Value::Null,
            });
        }
        app.selected = Some("a".into());
        let file = dir.path().join("b/src/main.rs");
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

    fn project_at(app: &mut App, id: &str, path: std::path::PathBuf) {
        app.state.projects.push(Project {
            id: id.into(),
            name: id.into(),
            path,
            layout: serde_json::Value::Null,
        });
    }

    #[test]
    fn search_stays_inside_the_selected_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        for id in ["a", "b"] {
            let path = dir.path().join(id);
            std::fs::create_dir_all(&path).unwrap();
            project_at(&mut app, id, path.clone());
            app.dirs.insert(
                path.clone(),
                vec![terminator_git::Entry {
                    path: path.join("main.rs"),
                    directory: false,
                    ignored: false,
                }],
            );
            app.state.sessions.push(Session {
                id: id.into(),
                project_id: id.into(),
                label: format!("{id} shell"),
                cwd: path,
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
        }
        app.selected = Some("a".into());
        app.palette_query = "main".into();
        let files = app.filtered_palette();
        assert!(
            files
                .iter()
                .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("a/main.rs")))
        );
        assert!(
            !files
                .iter()
                .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("b/main.rs")))
        );
        app.palette_query = "shell".into();
        let sessions = app.filtered_palette();
        assert!(
            sessions
                .iter()
                .any(|item| matches!(item, PaletteItem::Session(id, _) if id == "a"))
        );
        assert!(
            !sessions
                .iter()
                .any(|item| matches!(item, PaletteItem::Session(id, _) if id == "b"))
        );
        app.palette_query.clear();
        assert!(
            !app.filtered_palette()
                .iter()
                .any(|item| matches!(item, PaletteItem::Project(..)))
        );
        app.palette_filters = super::FILTER_PROJECTS;
        let projects = app.filtered_palette();
        assert!(
            projects
                .iter()
                .any(|item| matches!(item, PaletteItem::Project(id, _) if id == "b"))
        );
        assert!(
            projects
                .iter()
                .all(|item| matches!(item, PaletteItem::Project(..)))
        );
    }

    #[test]
    fn file_filter_drops_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        project_at(&mut app, "p", dir.path().into());
        app.selected = Some("p".into());
        app.dirs.insert(
            dir.path().into(),
            vec![terminator_git::Entry {
                path: dir.path().join("font.rs"),
                directory: false,
                ignored: false,
            }],
        );
        app.palette_filters = super::FILTER_FILES;
        app.palette_query = "font".into();
        let items = app.filtered_palette();
        assert!(
            items
                .iter()
                .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("font.rs")))
        );
        assert!(
            items
                .iter()
                .all(|item| matches!(item, PaletteItem::File(_)))
        );
    }

    fn live_shell(id: &str, project: &str, cwd: std::path::PathBuf) -> Session {
        Session {
            id: id.into(),
            project_id: project.into(),
            label: format!("{id} shell"),
            cwd,
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
        }
    }

    #[test]
    fn empty_search_leads_with_recent_files_then_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        project_at(&mut app, "p", root.clone());
        app.selected = Some("p".into());
        let recent = root.join("recent.rs");
        let other = root.join("other.rs");
        app.dirs.insert(
            root.clone(),
            vec![
                terminator_git::Entry {
                    path: other.clone(),
                    directory: false,
                    ignored: false,
                },
                terminator_git::Entry {
                    path: recent.clone(),
                    directory: false,
                    ignored: false,
                },
            ],
        );
        app.preferences.recent_native_files = vec![recent.clone()];
        app.state.sessions.push(live_shell("shell", "p", root));
        let items = app.filtered_palette();
        let recent_at = items
            .iter()
            .position(|item| matches!(item, PaletteItem::File(path) if path == &recent));
        let session_at = items
            .iter()
            .position(|item| matches!(item, PaletteItem::Session(id, _) if id == "shell"));
        let other_at = items
            .iter()
            .position(|item| matches!(item, PaletteItem::File(path) if path == &other));
        assert_eq!(
            (recent_at, session_at, other_at),
            (Some(0), Some(1), Some(2))
        );
        assert!(items.iter().all(|item| {
            matches!(
                item,
                PaletteItem::File(_) | PaletteItem::Session(..) | PaletteItem::Agent(..)
            )
        }));
        app.palette_filters = super::FILTER_ACTIONS;
        assert!(
            app.filtered_palette()
                .iter()
                .any(|item| item.filter_bit() == super::FILTER_ACTIONS)
        );
    }

    #[test]
    fn nested_project_files_stay_in_that_project() {
        use terminator_native_edit::finder::FileIndex;
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("parent");
        let child = parent.join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(parent.join("own.rs"), "").unwrap();
        std::fs::write(child.join("own.rs"), "").unwrap();
        let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
        project_at(&mut app, "parent", parent.clone());
        project_at(&mut app, "child", child.clone());
        app.file_index = FileIndex::scan(&[parent], MAX_INDEX_FILES);
        app.selected = Some("parent".into());
        app.palette_query = "own".into();
        let parent_hits = app.filtered_palette();
        assert!(parent_hits.iter().any(
            |item| matches!(item, PaletteItem::File(path) if path.ends_with("parent/own.rs"))
        ));
        assert!(
            !parent_hits.iter().any(
                |item| matches!(item, PaletteItem::File(path) if path.ends_with("child/own.rs"))
            )
        );
        app.selected = Some("child".into());
        let child_hits = app.filtered_palette();
        assert!(
            child_hits.iter().any(
                |item| matches!(item, PaletteItem::File(path) if path.ends_with("child/own.rs"))
            )
        );
        assert!(!child_hits.iter().any(
            |item| matches!(item, PaletteItem::File(path) if path.ends_with("parent/own.rs"))
        ));
    }
}
