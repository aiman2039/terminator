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
        /// UTF-16 column of the symbol name, as the server reports it.
        column: u32,
        container: Option<String>,
    },
    Settings(SettingsSection),
    Action(Action),
    AddProject,
    VncProfile(String, String),
    NewWorktree,
    OpenPlayer,
    ToggleIdeSidebarHeight,
    SaveLayout,
    ApplyLayout(String),
}

/// Shortcut actions the palette always lists, with a bare title and no
/// shortcut detail, in display order.
const PINNED_ACTIONS: &[Action] = &[
    Action::NewTerminal,
    Action::NewBrowser,
    Action::NewVnc,
    Action::NextAttention,
];
/// Pinned like [`PINNED_ACTIONS`], listed after "Open player".
const PINNED_IDE_ACTIONS: &[Action] = &[
    Action::ToggleIdeMode,
    Action::MoveToMain,
    Action::MoveToStrip,
];
/// Shortcut actions the palette never lists: they open the palette or its
/// Settings counterpart.
const HIDDEN_ACTIONS: &[Action] = &[Action::OpenSettings, Action::OpenPalette];
/// Shortcut actions listed only while a session is active.
const SESSION_ACTIONS: &[Action] = &[
    Action::SelectAll,
    Action::FindInTerminal,
    Action::SearchScrollback,
    Action::CopyWorkingDirectory,
    Action::RenameTerminal,
    Action::CloseSession,
    Action::ClearScrollback,
];

fn pinned_action(action: Action) -> bool {
    PINNED_ACTIONS.contains(&action) || PINNED_IDE_ACTIONS.contains(&action)
}

/// Palette item kind: one row of category, rank, and filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Agent,
    Symbol,
    File,
    Project,
    Session,
    Settings,
    Layout,
    Vnc,
    Action,
}

impl Kind {
    /// (category, kind rank, filter bit).
    const fn info(self) -> (&'static str, u8, u8) {
        match self {
            Self::Agent => ("Agent", 0, FILTER_AGENTS),
            Self::Symbol => ("Symbol", 1, FILTER_SYMBOLS),
            Self::File => ("File", 2, FILTER_FILES),
            Self::Project => ("Project", 3, FILTER_PROJECTS),
            Self::Session => ("Session", 4, FILTER_SESSIONS),
            Self::Settings => ("Settings", 5, FILTER_SETTINGS),
            Self::Layout => ("Layout", 6, FILTER_ACTIONS),
            Self::Vnc => ("VNC", 6, FILTER_ACTIONS),
            Self::Action => ("Action", 6, FILTER_ACTIONS),
        }
    }
}

impl PaletteItem {
    fn kind(&self) -> Kind {
        match self {
            Self::Project(..) => Kind::Project,
            Self::Session(..) => Kind::Session,
            Self::Agent(..) => Kind::Agent,
            Self::File(..) => Kind::File,
            Self::Symbol { .. } => Kind::Symbol,
            Self::Settings(..) => Kind::Settings,
            Self::ApplyLayout(..) => Kind::Layout,
            Self::VncProfile(..) => Kind::Vnc,
            _ => Kind::Action,
        }
    }

    fn category(&self) -> &'static str {
        self.kind().info().0
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
            Self::Action(action) => action.label().into(),
            Self::AddProject => "Add project".into(),
            Self::VncProfile(_, name) => name.clone(),
            Self::NewWorktree => "New task worktree".into(),
            Self::OpenPlayer => "Open player".into(),
            Self::ToggleIdeSidebarHeight => "Toggle IDE sidebar height".into(),
            Self::SaveLayout => "Save layout".into(),
            Self::ApplyLayout(name) => name.clone(),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::AddProject
            | Self::NewWorktree
            | Self::OpenPlayer
            | Self::ToggleIdeSidebarHeight
            | Self::SaveLayout => self.title(),
            Self::Action(action) if pinned_action(*action) => self.title(),
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
            Self::Action(action) if !pinned_action(*action) => app.shortcut_label(*action),
            Self::VncProfile(id, _) => app
                .state
                .vnc_profiles
                .iter()
                .find(|p| &p.id == id)
                .map_or_else(String::new, |p| p.endpoint()),
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
        self.kind().info().1
    }

    fn filter_bit(&self) -> u8 {
        self.kind().info().2
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
                    line: symbol.selection.start.line,
                    column: symbol.selection.start.character,
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
                    column: hit.range.start.character,
                    container: hit.container,
                });
            }
        }
        for section in SettingsSection::ALL {
            items.push(PaletteItem::Settings(section));
        }
        items.push(PaletteItem::AddProject);
        items.extend(PINNED_ACTIONS.iter().copied().map(PaletteItem::Action));
        items.push(PaletteItem::OpenPlayer);
        items.extend(PINNED_IDE_ACTIONS.iter().copied().map(PaletteItem::Action));
        items.extend([PaletteItem::ToggleIdeSidebarHeight, PaletteItem::SaveLayout]);
        for profile in &self.state.vnc_profiles {
            items.push(PaletteItem::VncProfile(
                profile.id.clone(),
                profile.display_name(),
            ));
        }
        for &action in Action::ALL {
            if !pinned_action(action)
                && !HIDDEN_ACTIONS.contains(&action)
                && self.shortcut_applies(action)
                && (!SESSION_ACTIONS.contains(&action) || self.active_session.is_some())
            {
                items.push(PaletteItem::Action(action));
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
        if self.palette.filters == 0 {
            kind != FILTER_PROJECTS
        } else {
            self.palette.filters & kind != 0
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
        if self.palette.query.trim().is_empty()
            && self.palette.filters == 0
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
        if self.palette.filters != 0 {
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
        let query = self.palette.query.trim();
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
                let on = self.palette.filters & bit != 0;
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
                        self.palette.filters &= !bit;
                    } else {
                        self.palette.filters |= bit;
                    }
                    self.palette.index = 0;
                }
            }
        });
        let hint = if project_name.is_empty() {
            "Search…".to_string()
        } else {
            format!("Search {project_name}…")
        };
        let search = ui.add(
            appearance::singleline(&mut self.palette.query)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "palette-search", search.rect);
        if search.changed() {
            self.palette.index = 0;
            self.palette.query_changed_at = Instant::now();
            self.lsp.clear_workspace_symbols();
        }
        // A pressed pointer is choosing a filter. Focus returns on release
        // so that click is not taken by the field.
        if !ui.input(|input| input.pointer.primary_down()) {
            search.request_focus();
        }
        if self.palette_wants(FILTER_SYMBOLS) {
            if self.palette.query.trim().chars().count() >= 2 {
                if self.palette.query_changed_at.elapsed() >= SYMBOL_QUERY_DELAY {
                    let scoped_path = self.selected_project().map(|project| project.path.clone());
                    if let Some(path) = scoped_path.as_deref() {
                        self.lsp
                            .request_scoped_symbols(&self.palette.query, Some(path));
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
        let stale = self.palette.cache.as_ref().is_none_or(|cache| {
            cache.query != self.palette.query
                || cache.state_revision != self.state.revision
                || cache.file_index_at != self.file_index_at
                || cache.lsp_revision != self.lsp.search_revision()
                || cache.filters != self.palette.filters
                || cache.project != project_id
        });
        if stale {
            self.palette.cache = Some(PaletteCache {
                query: self.palette.query.clone(),
                state_revision: self.state.revision,
                file_index_at: self.file_index_at,
                lsp_revision: self.lsp.search_revision(),
                filters: self.palette.filters,
                project: project_id,
                items: self.filtered_palette(),
            });
        }
        let items = self
            .palette
            .cache
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
        self.palette.index = self.palette.index.min(len.saturating_sub(1));
        let mut keyboard_moved = false;
        if ui.input(|input| input.key_pressed(egui::Key::ArrowDown))
            && let Some(next) = self
                .palette
                .index
                .checked_add(1)
                .and_then(|index| index.checked_rem(len))
        {
            self.palette.index = next;
            keyboard_moved = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            self.palette.index = self
                .palette
                .index
                .checked_sub(1)
                .unwrap_or_else(|| len.saturating_sub(1));
            keyboard_moved = true;
        }
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, item) in items.iter().enumerate() {
                    let selected = index == self.palette.index;
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
            chosen = Some(self.palette.index);
        }
        if let Some(index) = chosen
            && let Some(item) = items.get(index).cloned()
        {
            self.run_palette(ui.ctx(), item);
        }
    }

    fn close_palette(&mut self) {
        self.palette.open = false;
        self.palette.query.clear();
        self.palette.index = 0;
        self.palette.filters = 0;
        self.palette.cache = None;
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
            PaletteItem::Symbol {
                path, line, column, ..
            } => {
                if let Some(project) = self
                    .project_for_palette_path(&path)
                    .map(|project| project.id.clone())
                    && self.selected.as_ref() != Some(&project)
                {
                    self.select_project(project);
                }
                self.open_file_at(
                    path,
                    native_lsp::Position {
                        line,
                        character: column,
                    },
                );
            }
            PaletteItem::Settings(section) => {
                self.open_settings();
                self.request_settings_section(section);
            }
            PaletteItem::AddProject => self.add_project = true,
            PaletteItem::Action(action) => self.run_shortcut(ctx, action),
            PaletteItem::VncProfile(id, _) => {
                if let Some(profile) = self.state.vnc_profiles.iter().find(|p| p.id == id).cloned()
                {
                    self.connect_vnc_profile(&profile, None);
                }
            }
            PaletteItem::NewWorktree => self.open_worktree_wizard(),
            PaletteItem::OpenPlayer => self.open_player(),
            PaletteItem::ToggleIdeSidebarHeight => self.toggle_ide_sidebar_height(),
            PaletteItem::SaveLayout => self.save_layout_dialog(),
            PaletteItem::ApplyLayout(name) => {
                self.preferences.apply_layout(&name);
            }
        }
    }
}

#[cfg(test)]
mod tests;
