//! Search the current project: files, symbols, sessions, and agents.
//! Actions and settings stay global. The Projects filter lists every
//! project. File scans and LSP symbol requests stay async.
use super::*;
use crate::app::OpenFileInput;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};
use terminator_native_edit::finder::{MAX_INDEX_FILES, spawn_scan};
use terminator_native_edit::lsp as native_lsp;

const MAX_RESULTS: usize = 80;
/// File Structure lists a whole outline, not a ranked top slice.
const MAX_FILE_STRUCTURE: usize = 2000;
const FILE_RESULTS: usize = 40;
/// Recent files shown before sessions when the field is empty.
const RECENT_RESULTS: usize = 8;
const SYMBOL_QUERY_DELAY: Duration = Duration::from_millis(180);

pub(crate) const FILTER_FILES: u8 = 1 << 0;
pub(crate) const FILTER_SYMBOLS: u8 = 1 << 1;
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
    scope: Option<PathBuf>,
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
const HIDDEN_ACTIONS: &[Action] = &[
    Action::OpenSettings,
    Action::OpenPalette,
    Action::GotoSymbol,
    Action::FileStructure,
    Action::RecentFiles,
];
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

mod center;
mod index;
mod items;
mod results;

#[cfg(test)]
mod tests;
