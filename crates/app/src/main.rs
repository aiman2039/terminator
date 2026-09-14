mod daemon_connection;
use daemon_connection::can_retire_daemon;
mod exit;
mod installation;
mod installation_ui;
mod updater;
mod workspace_ui;
use workspace_ui::Viewer;
mod dialogs_ui;
mod sidebar_ui;
use sidebar_ui::{AttentionAction, AttentionCard, attention_card};
mod appearance;
mod external_editor;
mod file_actions;
mod icons;
mod image_preview;
mod markdown;
mod markdown_images;
mod metadata_refresh;
mod nvim_rpc;
mod refresh;
mod settings_ui;
mod ui_control;
use file_actions::FileAction;
mod editor_close;
mod popup;
mod preferences;
mod workspace;
use preferences::{SidebarTool, UiPreferences};
use terminator_core::appearance::{AppearanceConfig, AppearanceFile, config_path};
use workspace::Workspace;
mod clipboard;
#[cfg(feature = "test-support")]
mod diagnostics;
mod services;
use anyhow::{Context, Result};
use eframe::egui::{self, Color32, RichText};
#[cfg(test)]
use egui_dock::DockState;
use egui_dock::tab_viewer::OnCloseResponse;
use egui_dock::{DockArea, NodeIndex, TabViewer};
use egui_term::{PtyEvent, TerminalBackend, TerminalView};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub(crate) enum Tab {
    Image {
        path: PathBuf,
    },
    Terminal(String),
    Diff {
        cwd: PathBuf,
        path: PathBuf,
        staged: bool,
    },
}
impl Tab {
    fn key(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RenameSurface {
    Workspace,
    Pane,
    Sidebar,
}

#[derive(Clone)]
enum After {
    None,
    Create(Option<String>),
    CreateAt(Vec<Tab>, Option<String>),
    Workspace(String, Vec<Tab>),
    Text(String),
}
enum Job {
    RepairInstallation(String, exit::Checkpoint),
    Control(Box<Request>, After),
    OpenProject(PathBuf, u64),
    Preferences(UiPreferences),
    MigrateTypography,
    MigrateAttention,
    ExitDrain(u64, u64),
    ExitSave(u64, exit::Checkpoint),
    SaveAppearance(Box<AppearanceConfig>, String),
    HookStatus,
    CloseEditors(editor_close::Target, Vec<String>, editor_close::Mode),
    ResolveTarget(String, String, PathBuf),
    Browser(String),
    PasteClipboard(String),

    Diff(Tab),
    Install(String, bool),
    External(PathBuf),
    TestExternal(PathBuf, String, Vec<String>),
}
impl Job {
    fn rpc(request: Request, after: After) -> Self {
        Self::Control(Box::new(request), after)
    }
}
enum Update {
    InstallationRepaired(Result<Box<State>, String>),
    ExitDrained(u64, u64),
    ExitSaved(u64, Result<(), String>),
    UiRequest(
        terminator_core::ui_control::Request,
        mpsc::SyncSender<Result<serde_json::Value, String>>,
        Instant,
    ),
    Metadata(u64, metadata::Metadata),
    OpenImage(String, PathBuf, After),
    Image(PathBuf, u64, Result<egui::ColorImage, String>),
    TestPickerClosed,
    PickedProject(Option<PathBuf>, u64),
    OpenedProject(Box<State>, String, u64),
    TypographyMigrated,
    AttentionMigrated(Result<(), String>),
    Activation(String),
    Appearance(Box<AppearanceFile>),
    HookStatus(HashMap<String, bool>),
    EditorsClosed(editor_close::Target, Vec<String>, Result<(), String>),
    ResolvedTarget(String, Option<services::Target>),
    PreferencesSaved(Result<UiPreferences, String>),
    PickedFile {
        path: Option<PathBuf>,
        project: Option<String>,
        cwd: PathBuf,
    },
    State(Box<State>),
    Created(Session, Option<String>, Option<Vec<Tab>>),
    WorkspaceCreated(Session, String, Vec<Tab>),
    Text(String, String),
    Refresh(
        u64,
        services::ContextData,
        Vec<(PathBuf, Vec<services::Entry>)>,
        bool,
    ),
    ClipboardPaste(String, String),
    Error(String),
    Info(String),
}
fn begin_native_window_gesture(ctx: &egui::Context, command: egui::ViewportCommand) {
    ctx.send_viewport_cmd(command);
    // The window manager grabs pointer input and may consume mouse-up. Clear
    // egui's drag state at the handoff so the next control/edge can be pressed.
    ctx.stop_dragging();
    ctx.input_mut(|input| input.pointer = Default::default());
}
fn window_resize_edges(ui: &mut egui::Ui) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if ui.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    }) {
        return;
    }
    let r = ui.ctx().content_rect();
    let edge = 4.0;
    let corner = 8.0;
    let regions = [
        (
            egui::Rect::from_min_max(r.min, r.min + egui::vec2(corner, corner)),
            D::NorthWest,
            C::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_max(
                r.right_top() - egui::vec2(corner, 0.0),
                r.right_top() + egui::vec2(0.0, corner),
            ),
            D::NorthEast,
            C::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_max(
                r.left_bottom() - egui::vec2(0.0, corner),
                r.left_bottom() + egui::vec2(corner, 0.0),
            ),
            D::SouthWest,
            C::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_max(r.max - egui::vec2(corner, corner), r.max),
            D::SouthEast,
            C::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_max(
                r.min + egui::vec2(corner, 0.0),
                r.right_top() + egui::vec2(-corner, edge),
            ),
            D::North,
            C::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                r.left_bottom() + egui::vec2(corner, -edge),
                r.max - egui::vec2(corner, 0.0),
            ),
            D::South,
            C::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                r.min + egui::vec2(0.0, corner),
                r.left_bottom() + egui::vec2(edge, -corner),
            ),
            D::West,
            C::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(
                r.right_top() + egui::vec2(-edge, corner),
                r.max - egui::vec2(0.0, corner),
            ),
            D::East,
            C::ResizeHorizontal,
        ),
    ];
    // Panels own their full rectangles. Put only the narrow resize hit regions
    // above them, so the status bar cannot swallow edge drags.
    for (index, (rect, direction, cursor)) in regions.into_iter().enumerate() {
        egui::Area::new(egui::Id::new(("window-resize", index)))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .movable(false)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                let (_, response) = ui.allocate_exact_size(rect.size(), egui::Sense::drag());
                let response = response.on_hover_cursor(cursor);
                #[cfg(feature = "test-support")]
                {
                    diagnostics::record(ui.ctx(), &format!("window-resize-{index}"), response.rect);
                    if std::env::var_os("TERMINATOR_TEST_NATIVE_INPUT").is_some()
                        && response.hovered()
                        && ui.input(|i| i.pointer.any_down())
                    {
                        eprintln!(
                            "Fixture resize input {index}: started={} dragged={}",
                            response.drag_started(),
                            response.dragged()
                        );
                    }
                }
                if response.drag_started() {
                    begin_native_window_gesture(
                        ui.ctx(),
                        egui::ViewportCommand::BeginResize(direction),
                    );
                }
            });
    }
}
fn header_drag_space(ui: &mut egui::Ui) {
    let response = ui.allocate_response(
        egui::vec2(ui.available_width().max(0.0), 30.0),
        egui::Sense::drag(),
    );
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "header-drag", response.rect);
    if response.drag_started() {
        begin_native_window_gesture(ui.ctx(), egui::ViewportCommand::StartDrag);
    }
}
fn launch_external(
    program: &str,
    args: &[String],
    path: &std::path::Path,
    tx: Sender<Update>,
    ctx: egui::Context,
    test: bool,
) -> Result<()> {
    let updates = tx.clone();
    external_editor::launch(program, args, path, move |result| {
        if let Err(error) = result {
            let _ = updates.send(Update::Error(format!("{error:#}")));
        }
        ctx.request_repaint();
    })?;
    if test {
        let _ = tx.send(Update::Info(
            "External editor launched with draft settings. Check the selected file in the editor."
                .into(),
        ));
    }
    Ok(())
}
fn worker(paths: Paths, ctx: egui::Context, rx: Receiver<Job>, tx: Sender<Update>) {
    // Model-level tests inject responses explicitly. Keep their command channel
    // alive without starting dozens of native config watchers or probing a daemon.
    // The xtask PTY/GUI fixtures run the normal binary and exercise real I/O.
    if cfg!(test) {
        for job in rx {
            match job {
                Job::ExitDrain(id, serial) => {
                    let _ = tx.send(Update::ExitDrained(id, serial));
                }
                Job::ExitSave(id, _) => {
                    let _ = tx.send(Update::ExitSaved(id, Ok(())));
                }
                _ => {}
            }
        }
        return;
    }
    let mut last = Instant::now() - Duration::from_secs(2);
    let mut revision = None;
    let config = config_path(&paths).ok();
    let (config_events, config_rx) = mpsc::channel();
    let watch_target = config.clone();
    let mut config_watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.as_ref().map_or(true, |e| {
                !matches!(e.kind, notify::EventKind::Access(_))
                    && e.paths.iter().any(|path| {
                        watch_target
                            .as_ref()
                            .is_some_and(|target| path.file_name() == target.file_name())
                    })
            }) {
                let _ = config_events.send(());
            }
        })
        .ok();
    if let (Some(watcher), Some(path)) = (&mut config_watcher, &config) {
        use notify::Watcher;
        let mut parent = path.parent().unwrap_or(path);
        while !parent.exists() {
            let Some(next) = parent.parent() else { break };
            parent = next;
        }
        if watcher
            .watch(parent, notify::RecursiveMode::Recursive)
            .is_err()
        {
            config_watcher = None;
        }
    }
    let mut config_changed = None::<Instant>;
    let mut config_source: Option<String> = None;
    let mut config_check = Instant::now() - Duration::from_secs(2);
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(job) => {
                let result = (|| -> Result<()> {
                    match job {
                        Job::RepairInstallation(generation, checkpoint) => {
                            let result = (|| -> Result<Box<State>> {
                                checkpoint.save(&paths)?;
                                daemon_connection::repair(
                                    &paths,
                                    &std::env::current_exe()?,
                                    &generation,
                                )
                            })()
                            .map_err(|e| format!("{e:#}"));
                            revision = None;
                            tx.send(Update::InstallationRepaired(result))?;
                        }
                        Job::ResolveTarget(key, text, cwd) => {
                            tx.send(Update::ResolvedTarget(
                                key,
                                services::resolve_target(&text, &cwd).ok(),
                            ))?;
                        }
                        Job::PasteClipboard(session) => {
                            if let Some(text) = clipboard::read_paste()? {
                                tx.send(Update::ClipboardPaste(session, text))?;
                            }
                        }
                        Job::Browser(url) => {
                            open::that(url)?;
                        }
                        Job::CloseEditors(target, ids, mode) => {
                            let result = editor_close::close(&paths, &ids, mode)
                                .map_err(|e| format!("{e:#}"));
                            tx.send(Update::EditorsClosed(target, ids, result))?;
                        }
                        Job::HookStatus => {
                            let home = std::env::var_os("HOME")
                                .map(PathBuf::from)
                                .unwrap_or_default();
                            tx.send(Update::HookStatus(
                                terminator_integrations::AGENTS
                                    .iter()
                                    .map(|kind| {
                                        (
                                            (*kind).to_string(),
                                            terminator_integrations::installed(&home, kind),
                                        )
                                    })
                                    .collect(),
                            ))?;
                        }
                        Job::SaveAppearance(theme, expected) => {
                            let path = config.as_ref().context("No configuration path")?;
                            let file = AppearanceFile::save(path, &theme, &expected)?;
                            config_source = Some(file.source.clone());
                            tx.send(Update::Appearance(Box::new(file)))?;
                        }
                        Job::ExitDrain(id, serial) => {
                            tx.send(Update::ExitDrained(id, serial))?;
                        }
                        Job::ExitSave(id, checkpoint) => {
                            let result = checkpoint.save(&paths).map_err(|e| format!("{e:#}"));
                            tx.send(Update::ExitSaved(id, result))?;
                        }
                        Job::Preferences(prefs) => {
                            let result = prefs
                                .save(&paths.data)
                                .map(|()| prefs)
                                .map_err(|e| format!("Save UI preferences: {e:#}"));
                            tx.send(Update::PreferencesSaved(result))?;
                        }
                        Job::MigrateAttention => {
                            let result = (|| -> Result<()> {
                                let Response::State(mut state) = rpc(&paths, Request::Snapshot)?
                                else {
                                    anyhow::bail!("Expected settings snapshot");
                                };
                                state.settings.notifications_side = true;
                                state
                                    .settings
                                    .keybindings
                                    .entry("open_file".into())
                                    .or_insert_with(|| "command+O".into());
                                anyhow::ensure!(
                                    matches!(
                                        rpc(&paths, Request::Settings(state.settings))?,
                                        Response::Ok
                                    ),
                                    "Settings not acknowledged"
                                );
                                Ok(())
                            })()
                            .map_err(|e| format!("{e:#}"));
                            tx.send(Update::AttentionMigrated(result))?;
                            if let Response::State(state) = rpc(&paths, Request::Snapshot)? {
                                tx.send(Update::State(state))?;
                            }
                        }
                        Job::MigrateTypography => {
                            let Response::State(mut state) = rpc(&paths, Request::Snapshot)? else {
                                anyhow::bail!("Expected settings snapshot");
                            };
                            state.settings.font_size = 13.0;
                            anyhow::ensure!(
                                matches!(
                                    rpc(&paths, Request::Settings(state.settings))?,
                                    Response::Ok
                                ),
                                "Settings not acknowledged"
                            );
                            tx.send(Update::TypographyMigrated)?;
                            if let Response::State(state) = rpc(&paths, Request::Snapshot)? {
                                tx.send(Update::State(state))?;
                            }
                        }
                        Job::OpenProject(path, generation) => {
                            let path = path.canonicalize()?;
                            anyhow::ensure!(path.is_dir(), "Project must be a directory");
                            // Reopening a removed project keeps its identity and sessions,
                            // even when its saved path uses a symlink to the selected folder.
                            let Response::State(state) = rpc(&paths, Request::Snapshot)? else {
                                anyhow::bail!("Expected project inventory");
                            };
                            let (state, project) = if let Some(project) =
                                services::project_for_directory(&state.projects, &path)
                            {
                                let project = project.id.clone();
                                (state, project)
                            } else {
                                rpc(&paths, Request::AddProject { path: path.clone() })?;
                                let Response::State(state) = rpc(&paths, Request::Snapshot)? else {
                                    anyhow::bail!("Expected project inventory");
                                };
                                let project =
                                    services::project_for_directory(&state.projects, &path)
                                        .context("Opened project missing from inventory")?
                                        .id
                                        .clone();
                                (state, project)
                            };
                            tx.send(Update::OpenedProject(state, project, generation))?;
                        }
                        Job::Control(req, after) => match rpc(&paths, *req)? {
                            Response::Created(session) => {
                                if let After::Create(split) = after {
                                    tx.send(Update::Created(session, split, None))?;
                                } else if let After::CreateAt(anchors, split) = after {
                                    tx.send(Update::Created(session, split, Some(anchors)))?;
                                } else if let After::Workspace(id, anchors) = after {
                                    tx.send(Update::WorkspaceCreated(session, id, anchors))?;
                                }
                            }
                            Response::Text(text) => {
                                if let After::Text(key) = after {
                                    tx.send(Update::Text(key, text))?;
                                }
                            }
                            _ => {}
                        },
                        Job::Diff(tab) => {
                            if let Tab::Diff { cwd, path, staged } = &tab {
                                let content = services::diff(cwd, path, *staged)?;
                                tx.send(Update::Text(tab.key(), content))?;
                            }
                        }
                        Job::Install(kind, remove) => {
                            anyhow::ensure!(
                                remove || find_executable(&kind).is_some(),
                                "Install the {kind} CLI before configuring its hooks"
                            );
                            let home = std::env::var_os("HOME").context("No home directory")?;
                            let helper = std::env::current_exe()?.with_file_name("terminator-hook");
                            let path = terminator_integrations::install_at(
                                std::path::Path::new(&home),
                                &kind,
                                &helper,
                                remove,
                                &paths,
                            )?;
                            tx.send(Update::Info(format!(
                                "{} hooks: {}",
                                if remove { "Removed" } else { "Installed" },
                                path.display()
                            )))?;
                        }
                        Job::External(path) => {
                            let Response::State(state) = rpc(&paths, Request::Snapshot)? else {
                                anyhow::bail!("Expected editor settings snapshot");
                            };
                            launch_external(
                                &state.settings.external_editor,
                                &state.settings.external_args,
                                &path,
                                tx.clone(),
                                ctx.clone(),
                                false,
                            )?;
                        }
                        Job::TestExternal(path, program, args) => {
                            launch_external(&program, &args, &path, tx.clone(), ctx.clone(), true)?;
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    let error = format!("{e:#}");
                    if daemon_connection::is_connection_error(&error) {
                        revision = None;
                    }
                    let _ = tx.send(Update::Error(error));
                }
                ctx.request_repaint();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        while config_rx.try_recv().is_ok() {
            config_changed = Some(Instant::now());
        }
        if config_source.is_none()
            || config_changed.is_some_and(|at| at.elapsed() >= Duration::from_millis(250))
            || config_check.elapsed()
                >= Duration::from_secs(if config_watcher.is_some() { 30 } else { 3 })
        {
            config_changed = None;
            if let Some(path) = &config {
                let source = fs::read_to_string(path).unwrap_or_default();
                if config_source.as_ref() != Some(&source) {
                    config_source = Some(source);
                    match AppearanceFile::load(path) {
                        Ok(file) => {
                            let _ = tx.send(Update::Appearance(Box::new(file)));
                        }
                        Err(e) => {
                            let _ = tx.send(Update::Error(format!("{}: {e:#}", path.display())));
                        }
                    }
                    ctx.request_repaint();
                }
            }
            config_check = Instant::now();
        }
        if last.elapsed() >= Duration::from_millis(100) {
            if let Ok(notice) = fs::read_to_string(paths.runtime.join("activation")) {
                let _ = fs::remove_file(paths.runtime.join("activation"));
                let _ = tx.send(Update::Activation(notice));
                ctx.request_repaint();
            }
            match conditional_snapshot(&paths, revision.clone()) {
                Ok(Response::State(state)) => {
                    if revision.as_ref() != Some(&state.snapshot_hint()) {
                        revision = Some(state.snapshot_hint());
                        let _ = tx.send(Update::State(state));
                        ctx.request_repaint();
                    }
                }
                Err(e) => {
                    // Obtain a full snapshot on reconnect even if the daemon's
                    // generation/revision did not change during the outage.
                    revision = None;
                    let _ = tx.send(Update::Error(format!("Reconnecting: {e}")));
                    ctx.request_repaint();
                }
                _ => {}
            }
            last = Instant::now();
        }
    }
}
struct App {
    installation_error: Option<String>,
    repair_pending: bool,
    exit: exit::Exit,
    exit_attempt: u64,
    updater: updater::Updater,
    #[cfg(feature = "test-support")]
    diagnostics: diagnostics::Diagnostics,
    paths: Paths,
    preferences: UiPreferences,
    preferences_saved: UiPreferences,
    preferences_writable: bool,
    preferences_pending: bool,
    project_width: f32,
    migration_requested: bool,
    attention_requested: Option<Instant>,
    attention_pending: bool,
    selection_generation: u64,
    state: State,
    state_loaded: bool,
    layouts: HashMap<String, Workspace>,
    layout_readonly: HashSet<String>,
    close_workspace: Option<(String, String)>,
    workspace_visible: Option<(String, String)>,
    layout_saved: HashMap<String, String>,
    selected: Option<String>,
    active_session: Option<String>,
    images: HashMap<PathBuf, image_preview::Preview>,
    markdown: markdown::Previews,
    visible_images: HashSet<PathBuf>,
    image_generation: u64,
    image_jobs: mpsc::SyncSender<(PathBuf, u64)>,
    backends: HashMap<String, TerminalBackend>,
    visible_sessions: HashSet<String>,
    backend_ids: HashMap<u64, String>,
    next_backend: u64,
    pty_tx: Sender<(u64, PtyEvent)>,
    pty_rx: Receiver<(u64, PtyEvent)>,
    jobs: exit::JobQueue,
    updates: Receiver<Update>,
    update_tx: Sender<Update>,
    picker_active: bool,
    add_tab: Option<(egui_dock::NodePath, Option<String>)>,
    pane_by_tab: HashMap<String, egui_dock::NodePath>,

    pane_tabs: HashMap<egui_dock::NodePath, Vec<Tab>>,
    focus_tab: Option<Tab>,
    terminal_context: HashMap<String, String>,
    texts: HashMap<String, String>,
    loading: HashSet<String>,
    dirs: HashMap<PathBuf, Vec<services::Entry>>,
    context: Option<services::ContextData>,
    metadata: Option<metadata::Metadata>,
    metadata_jobs: Sender<Option<metadata_refresh::Request>>,
    metadata_request: Option<metadata_refresh::Request>,
    metadata_generation: u64,
    context_path: Option<PathBuf>,
    error: Option<String>,
    info: Option<String>,
    add_project: bool,
    settings_open: bool,
    editor_preset: usize,
    test_editor: bool,
    settings_draft: Settings,
    settings_section: usize,
    theme: AppearanceConfig,
    theme_committed: AppearanceConfig,
    theme_draft: AppearanceConfig,
    theme_source: String,
    theme_conflict: bool,
    hook_status: HashMap<String, bool>,
    detail: Option<String>,
    close_session: Option<String>,
    popups: popup::Popups,
    editor_close_sessions: HashSet<String>,
    editor_close_prompts: Vec<(editor_close::Target, Vec<String>, String)>,
    rename_session: Option<(String, String)>,
    rename_focus: bool,
    rename_surface: RenameSurface,
    editor_origins: HashMap<String, Vec<Tab>>,
    search: String,
    search_session: Option<String>,
    open_path: bool,
    path_text: String,
    last_save: Instant,
    refresh: Sender<Option<refresh::Request>>,
    refresh_request: Option<refresh::Request>,
    refresh_generation: u64,
    visible_dirs: Vec<PathBuf>,
    expanded_dirs: HashSet<PathBuf>,
    watch_fallback: bool,
    targets: HashMap<String, Option<services::Target>>,
    hover: Option<(String, Instant)>,
    hover_popup: Option<(String, services::Target, egui::Rect)>,
    pending_target_action: Option<(String, Session)>,
    last_heartbeat: Instant,
    last_focus: Option<String>,
    highlight_session: Option<String>,
    highlight_since: Instant,
    connected: bool,
    control_server: Option<ui_control::Server>,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, paths: Paths) -> Self {
        let mut app = Self::with_context(&cc.egui_ctx, paths.clone());
        match ui_control::spawn(paths, app.update_tx.clone(), cc.egui_ctx.clone()) {
            Ok(server) => app.control_server = Some(server),
            Err(error) => app.error = Some(format!("GUI control server: {error:#}")),
        }
        app
    }
    fn with_context(ctx: &egui::Context, paths: Paths) -> Self {
        appearance::install(ctx);
        let markdown = markdown::Previews::new(ctx);
        let loaded = UiPreferences::load(&paths.data);
        let preferences_writable = loaded.is_ok();
        let preference_error = loaded
            .as_ref()
            .err()
            .map(|e| format!("UI preferences: {e:#}"));
        let preferences = loaded.unwrap_or_default();
        let (jobs, rx) = mpsc::channel();
        let (tx, updates) = mpsc::channel();
        let refresh = refresh::spawn(tx.clone(), ctx.clone());
        let metadata_jobs = metadata_refresh::spawn(tx.clone(), ctx.clone());
        let update_tx = tx.clone();
        let (image_jobs, image_requests) = mpsc::sync_channel::<(PathBuf, u64)>(8);
        let image_updates = tx.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            while let Ok((path, generation)) = image_requests.recv() {
                let result = image_preview::decode(&path).map_err(|e| format!("{e:#}"));
                if image_updates
                    .send(Update::Image(path, generation, result))
                    .is_err()
                {
                    break;
                }
                repaint.request_repaint();
            }
        });
        let p = paths.clone();
        let context = ctx.clone();
        thread::spawn(move || worker(p, context, rx, tx));
        let _ = jobs.send(Job::HookStatus);
        let (pty_tx, pty_rx) = mpsc::channel();
        Self {
            exit: Default::default(),
            exit_attempt: 0,
            updater: updater::Updater::new(ctx),
            installation_error: None,
            repair_pending: false,
            #[cfg(feature = "test-support")]
            diagnostics: Default::default(),
            preferences_saved: preferences.clone(),
            preferences,
            preferences_writable,
            preferences_pending: false,
            project_width: 225.0,
            migration_requested: false,
            attention_requested: None,
            attention_pending: false,
            selection_generation: 0,
            paths,
            state: State::default(),
            state_loaded: false,
            layouts: HashMap::new(),
            layout_readonly: HashSet::new(),
            close_workspace: None,
            workspace_visible: None,
            layout_saved: HashMap::new(),
            selected: None,
            active_session: None,
            images: HashMap::new(),
            markdown,
            visible_images: HashSet::new(),
            image_generation: 0,
            image_jobs,
            backends: HashMap::new(),
            visible_sessions: HashSet::new(),
            backend_ids: HashMap::new(),
            next_backend: 0,
            pty_tx,
            pty_rx,
            jobs: jobs.into(),
            updates,
            update_tx,
            picker_active: false,
            add_tab: None,
            pane_by_tab: HashMap::new(),

            pane_tabs: HashMap::new(),
            focus_tab: None,
            terminal_context: HashMap::new(),
            texts: HashMap::new(),
            loading: HashSet::new(),
            dirs: HashMap::new(),
            context: None,
            metadata: None,
            metadata_jobs,
            metadata_request: None,
            metadata_generation: 0,
            context_path: None,
            error: preference_error,
            info: None,
            add_project: false,
            settings_open: false,
            editor_preset: external_editor::CUSTOM,
            test_editor: false,
            settings_draft: Settings::default(),
            settings_section: 0,
            theme: AppearanceConfig::default(),
            theme_committed: AppearanceConfig::default(),
            theme_draft: AppearanceConfig::default(),
            theme_source: String::new(),
            theme_conflict: false,
            hook_status: HashMap::new(),
            detail: None,
            close_session: None,
            popups: popup::Popups::default(),
            editor_close_sessions: HashSet::new(),
            editor_close_prompts: Vec::new(),
            rename_session: None,
            rename_focus: false,
            rename_surface: RenameSurface::Sidebar,
            editor_origins: HashMap::new(),
            search: String::new(),
            search_session: None,
            open_path: false,
            path_text: String::new(),
            last_save: Instant::now(),
            refresh,
            refresh_request: None,
            refresh_generation: 0,
            visible_dirs: Vec::new(),
            expanded_dirs: HashSet::new(),
            watch_fallback: false,
            targets: HashMap::new(),
            hover: None,
            hover_popup: None,
            pending_target_action: None,
            last_heartbeat: Instant::now(),
            last_focus: None,
            highlight_session: None,
            highlight_since: Instant::now(),
            connected: false,
            control_server: None,
        }
    }
    fn fixture_rect(&self, ctx: &egui::Context, name: &str) -> Option<[f32; 4]> {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
            .map(|r| [r.min.x, r.min.y, r.width(), r.height()])
    }
    fn ui_request(
        &mut self,
        ctx: &egui::Context,
        request: terminator_core::ui_control::Request,
    ) -> Result<serde_json::Value> {
        use terminator_core::ui_control::Request as Ui;
        request.validate()?;
        anyhow::ensure!(!self.exit.active(), "Terminator is saving before closing");
        let gui_ppp = ctx.pixels_per_point();
        match request {
            Ui::Ping => {
                return Ok(
                    serde_json::json!({"capabilities":[terminator_core::ui_control::CAPABILITY]}),
                );
            }
            Ui::Snapshot => {
                #[allow(unused_mut)]
                let mut snapshot = serde_json::json!({"selected_project":self.selected,"active_session":self.active_session,"workspaces":self.layouts,
                    "controls":{
                        "header-drag":self.fixture_rect(ctx,"header-drag"),
                        "project-add":self.fixture_rect(ctx,"project-add"),
                        "resize-se":self.fixture_rect(ctx,"window-resize-3")
                    },
                    "window":ctx.input(|i|serde_json::json!({"inner":i.viewport().inner_rect.map(|r|[r.min.x,r.min.y,r.width(),r.height()]),"outer":i.viewport().outer_rect.map(|r|[r.min.x,r.min.y,r.width(),r.height()]),"maximized":i.viewport().maximized,"minimized":i.viewport().minimized,"gui_ppp":gui_ppp,"native_ppp":i.viewport().native_pixels_per_point}))});
                #[cfg(feature = "test-support")]
                {
                    snapshot["updater_available"] = serde_json::json!(self.updater.available());
                    snapshot["installation"] = serde_json::json!({
                        "connected":self.connected,
                        "problem":self.installation_problem(),
                        "repair_pending":self.repair_pending,
                        "can_repair":self.connected && can_retire_daemon(&self.state),
                        "settings_visible":self.settings_open && self.settings_section == 6,
                        "generation":self.state.generation,
                        "error":self.error,
                    });
                    snapshot["left_agents"] = serde_json::json!(self.preferences.left_agents);
                    snapshot["agent_bar_badge"] = serde_json::json!(ctx.data(|data| {
                        data.get_temp::<String>(egui::Id::new("agent-bar-badge"))
                    }));
                    snapshot["markdown"] = self.markdown.diagnostics();
                    snapshot["markdown_modes"] =
                        serde_json::to_value(&self.preferences.markdown_modes)?;
                    snapshot["visible_terminals"] = serde_json::to_value(&self.visible_sessions)?;
                    snapshot["editor_rect"] =
                        serde_json::to_value(self.fixture_rect(ctx, "editor-terminal"))?;
                    snapshot["sidebar_projects"] = serde_json::json!(
                        self.state
                            .projects
                            .iter()
                            .filter(|p| !self.preferences.hidden_projects.contains(&p.id))
                            .map(|p| &p.id)
                            .collect::<Vec<_>>()
                    );
                    snapshot["markdown_header"] = serde_json::json!({
                        "title":self.fixture_rect(ctx,"markdown-title"),
                        "edit":self.fixture_rect(ctx,"markdown-mode:Edit"),
                        "preview":self.fixture_rect(ctx,"markdown-mode:Preview"),
                        "split":self.fixture_rect(ctx,"markdown-mode:Split"),
                        "refresh":self.fixture_rect(ctx,"markdown-refresh")
                    });
                }
                return Ok(snapshot);
            }
            Ui::Focus { session } => {
                anyhow::ensure!(
                    self.state.sessions.iter().any(|s| s.id == session),
                    "Unknown session"
                );
                self.go_session(&session);
            }
            Ui::ShowSession {
                session,
                anchor,
                split,
            } => {
                let record = self
                    .state
                    .sessions
                    .iter()
                    .find(|s| s.id == session && s.lifecycle.live())
                    .context("Unknown live session")?
                    .clone();
                anyhow::ensure!(
                    !self.layout_readonly.contains(&record.project_id),
                    "Project has an unsupported layout version"
                );
                let tab = Tab::Terminal(session.clone());
                if self
                    .layouts
                    .get(&record.project_id)
                    .is_some_and(|d| d.contains(&tab))
                {
                    self.go_session(&session);
                    return Ok(serde_json::json!({"accepted":true,"existing":true}));
                }
                if let Some(anchor) = anchor {
                    anyhow::ensure!(
                        self.state
                            .sessions
                            .iter()
                            .any(|s| s.id == anchor && s.project_id == record.project_id),
                        "Anchor belongs to a different project"
                    );
                    let dock = self
                        .layouts
                        .get_mut(&record.project_id)
                        .context("Missing project layout")?;
                    let anchor = Tab::Terminal(anchor);
                    anyhow::ensure!(
                        dock.activate_containing(&anchor),
                        "Anchor is not in a visible workspace"
                    );
                    let path = dock.find_tab(&anchor).context("Missing anchor pane")?;
                    dock.set_focused_node_and_surface(path.node_path());
                    self.insert(&record.project_id, tab, split.as_deref());
                } else {
                    self.layouts
                        .entry(record.project_id.clone())
                        .or_insert_with(Workspace::empty)
                        .add(id(), tab);
                }
                self.select_project(record.project_id);
                self.active_session = Some(session);
            }
            Ui::OpenFile {
                project,
                path,
                as_text,
            } => {
                anyhow::ensure!(
                    self.state.projects.iter().any(|p| p.id == project),
                    "Unknown project"
                );
                anyhow::ensure!(
                    !self.layout_readonly.contains(&project),
                    "Project has an unsupported layout version"
                );
                self.select_project(project);
                self.open_file_mode(path, None, None, false, as_text);
            }
            Ui::OpenBrowser { url } => {
                let _ = self.jobs.send(Job::Browser(metadata::http_url(&url)?));
            }
            Ui::Window { action } => ctx.send_viewport_cmd(match action.as_str() {
                "minimize" => egui::ViewportCommand::Minimized(true),
                "maximize" => egui::ViewportCommand::Maximized(true),
                "restore" => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    egui::ViewportCommand::Maximized(false)
                }
                "close" => egui::ViewportCommand::Close,
                _ => egui::ViewportCommand::Focus,
            }),
        }
        self.save_layouts();
        ctx.request_repaint();
        Ok(serde_json::json!({"accepted":true}))
    }
    fn send(&self, request: Request) {
        let _ = self.jobs.send(Job::rpc(request, After::None));
    }
    fn process_updates(&mut self, ctx: &egui::Context) {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Update::InstallationRepaired(result) => {
                    self.repair_pending = false;
                    match result {
                        Ok(state) => {
                            self.apply_state(*state);
                            self.installation_error = None;
                            self.error = None;
                            self.info =
                                Some("Installation repaired. New terminals can be opened.".into());
                        }
                        Err(error) => {
                            self.error = Some(format!("Could not repair installation: {error}"))
                        }
                    }
                }
                Update::ExitDrained(id, serial) => {
                    if let exit::Exit::Draining(started, current) = self.exit
                        && current == id
                    {
                        if serial != self.jobs.serial() {
                            let _ = self.jobs.send(Job::ExitDrain(id, self.jobs.serial()));
                            continue;
                        }
                        match self.exit_checkpoint() {
                            Ok(checkpoint) => {
                                self.exit = exit::Exit::Saving(started, id);
                                if self.jobs.send(Job::ExitSave(id, checkpoint)).is_err() {
                                    self.cancel_exit("Worker disconnected".into());
                                }
                            }
                            Err(e) => self.cancel_exit(format!("{e:#}")),
                        }
                    }
                }
                Update::ExitSaved(id, result) => {
                    if matches!(self.exit, exit::Exit::Saving(_, current) if current == id) {
                        match result {
                            Ok(()) => {
                                self.exit = exit::Exit::Ready;
                                updater::complete_termination(ctx);
                            }
                            Err(e) => self.cancel_exit(e),
                        }
                    }
                }
                Update::PreferencesSaved(result) => {
                    self.preferences_pending = false;
                    match result {
                        Ok(prefs) => self.preferences_saved = prefs,
                        Err(error) => {
                            if self.exit.active() {
                                self.cancel_exit(error);
                            } else {
                                self.error = Some(error);
                            }
                        }
                    }
                }
                Update::ResolvedTarget(key, target) => {
                    self.loading.remove(&key);
                    if self.targets.len() > 256 {
                        self.targets.clear();
                    }
                    if let Some((pending, session)) = self.pending_target_action.take() {
                        if pending == key {
                            if let Some(target) = &target {
                                self.terminal_action(ctx, &session, target, FileAction::Open);
                            } else {
                                self.error = Some("Target no longer exists".into());
                            }
                        } else {
                            self.pending_target_action = Some((pending, session));
                        }
                    }
                    self.targets.insert(key, target);
                }
                Update::EditorsClosed(target, ids, result) => {
                    self.editors_closed(target, ids, result);
                }
                Update::UiRequest(request, reply, deadline) => {
                    let result = if Instant::now() >= deadline {
                        Err("GUI request expired before processing; no action was performed".into())
                    } else {
                        self.ui_request(ctx, request).map_err(|e| format!("{e:#}"))
                    };
                    let _ = reply.send(result);
                }
                Update::Metadata(generation, data) => {
                    if generation == self.metadata_generation {
                        self.metadata = Some(data);
                    }
                }
                Update::OpenImage(project, path, after) => {
                    let tab = Tab::Image { path };
                    match after {
                        After::CreateAt(anchors, direction) => {
                            let previous = self.layouts.get(&project).map(|d| d.active.clone());
                            if let Some(dock) = self.layouts.get_mut(&project) {
                                if let Some(anchor) = anchors.iter().find(|t| dock.contains(t)) {
                                    dock.activate_containing(anchor);
                                }
                                if let Some(path) = anchors.iter().find_map(|t| dock.find_tab(t)) {
                                    dock.set_focused_node_and_surface(path.node_path());
                                }
                            }
                            let same =
                                previous.as_ref() == self.layouts.get(&project).map(|d| &d.active);
                            self.insert(&project, tab, direction.as_deref());
                            if !same
                                && let Some(previous) = previous
                                && let Some(dock) = self.layouts.get_mut(&project)
                            {
                                dock.active = previous;
                            }
                            if same && self.selected.as_deref() == Some(&project) {
                                self.active_session = None;
                            }
                        }
                        _ => {
                            self.layouts
                                .entry(project.clone())
                                .or_insert_with(Workspace::empty)
                                .add(id(), tab);
                            if self.selected.as_deref() == Some(&project) {
                                self.active_session = None;
                            }
                        }
                    }
                }
                Update::Image(path, generation, result) => {
                    let used: usize = self
                        .images
                        .values()
                        .filter_map(|p| p.texture.as_ref())
                        .map(|t| t.size()[0] * t.size()[1] * 4)
                        .sum();
                    if let Some(preview) = self
                        .images
                        .get_mut(&path)
                        .filter(|p| p.generation == generation)
                    {
                        preview.loading = false;
                        match result {
                            Ok(image) if used + image.pixels.len() * 4 <= 128 * 1024 * 1024 => {
                                preview.texture = Some(ctx.load_texture(
                                    format!("preview:{}:{generation}", path.display()),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                            Ok(_) => {
                                preview.error = Some(
                                    "Preview memory limit reached; close another image and retry."
                                        .into(),
                                )
                            }
                            Err(error) => preview.error = Some(error),
                        }
                    }
                }
                Update::TestPickerClosed => self.picker_active = false,
                Update::HookStatus(status) => self.hook_status = status,
                Update::Appearance(file) => {
                    let dirty = self.settings_open && self.theme_draft != self.theme_committed;
                    self.theme_conflict = dirty && file.config != self.theme_draft;
                    self.theme_committed = file.config.clone();
                    self.theme_source = file.source;
                    if !self.theme_conflict {
                        self.theme_draft = file.config.clone();
                        self.theme = file.config;
                        appearance::apply(ctx, &self.theme);
                    }
                }
                Update::Activation(notice) => {
                    self.detail = Some(notice);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                Update::AttentionMigrated(result) => {
                    self.attention_pending = false;
                    match result {
                        Ok(()) => self.preferences.attention_migrated = true,
                        Err(error) => {
                            self.error = Some(format!("Attention settings migration: {error}"))
                        }
                    }
                }
                Update::TypographyMigrated => {
                    self.preferences.typography_migrated = true;
                }
                Update::OpenedProject(state, project, generation) => {
                    self.apply_state(*state);
                    if generation == self.selection_generation {
                        self.select_project(project);
                    } else if let Some(project) = self.selected.clone() {
                        // Undo older AddProject selection side effects on the daemon.
                        self.send(Request::SelectProject { project });
                    }
                }
                Update::PickedProject(path, generation) => {
                    #[cfg(feature = "test-support")]
                    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                        eprintln!("Fixture native project picker selected={}", path.is_some());
                    }
                    self.picker_active = false;
                    if let Some(path) = path
                        && generation == self.selection_generation
                    {
                        let _ = self.jobs.send(Job::OpenProject(path, generation));
                    }
                }
                Update::PickedFile { path, project, cwd } => {
                    #[cfg(feature = "test-support")]
                    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                        eprintln!("Fixture native file picker selected={}", path.is_some());
                    }
                    self.picker_active = false;
                    if let Some(path) = path {
                        if image_preview::supported(&path)
                            && let Some(project) = &project
                        {
                            self.open_image(project, path, None);
                        } else if self.state.settings.editor_mode == EditorMode::External {
                            let _ = self.jobs.send(Job::External(path));
                        } else if let Some(project) = project {
                            let after = self.editor_target(&project, None, None);
                            let _ = self.jobs.send(Job::rpc(
                                Request::Create {
                                    project,
                                    cwd: Some(cwd),
                                    file: Some(path),
                                    line: None,
                                    column: None,
                                    editor: true,
                                },
                                after,
                            ));
                        }
                    }
                }
                Update::State(state) => {
                    self.apply_state(*state);
                }
                Update::WorkspaceCreated(session, id, anchors) => {
                    let project = session.project_id.clone();
                    if self.selected.as_ref() == Some(&project) {
                        self.finish_rename(true);
                    }
                    if session.kind == SessionKind::Editor {
                        self.editor_origins.insert(session.id.clone(), anchors);
                    }
                    self.layouts
                        .entry(project.clone())
                        .or_insert_with(Workspace::empty)
                        .add(id, Tab::Terminal(session.id.clone()));
                    if self.selected.as_ref() == Some(&project) {
                        self.active_session = Some(session.id.clone());
                    }
                    if !self.state.sessions.iter().any(|s| s.id == session.id) {
                        self.state.sessions.push(session);
                    }
                }
                Update::Created(session, split, target) => {
                    let previous_workspace = self
                        .layouts
                        .get(&session.project_id)
                        .map(|workspace| workspace.active.clone());
                    #[cfg(feature = "test-support")]
                    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                        eprintln!("Fixture created {:?}, split {:?}", session.kind, split);
                    }
                    if session.kind == SessionKind::Editor {
                        let anchors = target.clone().unwrap_or_default();
                        self.editor_origins.insert(session.id.clone(), anchors);
                    }
                    if let Some(target) = target
                        && let Some(dock) = self.layouts.get_mut(&session.project_id)
                    {
                        if let Some(tab) = target.iter().find(|tab| dock.contains(tab)) {
                            dock.activate_containing(tab);
                        }
                        if let Some(path) = target.iter().find_map(|tab| dock.find_tab(tab)) {
                            dock.set_focused_node_and_surface(path.node_path());
                        }
                    }
                    let same_workspace = previous_workspace.as_ref()
                        == self
                            .layouts
                            .get(&session.project_id)
                            .map(|workspace| &workspace.active);
                    if same_workspace && self.selected.as_ref() == Some(&session.project_id) {
                        self.active_session = Some(session.id.clone());
                    }
                    if !self.state.sessions.iter().any(|s| s.id == session.id) {
                        self.state.sessions.push(session.clone());
                    }
                    self.insert(
                        &session.project_id,
                        Tab::Terminal(session.id),
                        split.as_deref(),
                    );
                    if !same_workspace
                        && let Some(previous) = previous_workspace
                        && let Some(workspace) = self.layouts.get_mut(&session.project_id)
                        && workspace.tabs.iter().any(|tab| tab.id == previous)
                    {
                        workspace.active = previous;
                    }
                }
                Update::Text(key, text) => {
                    self.loading.remove(&key);
                    self.texts.insert(key, text);
                }
                Update::Refresh(generation, context, directories, fallback) => {
                    if generation == self.refresh_generation
                        && self
                            .refresh_request
                            .as_ref()
                            .is_some_and(|r| r.cwd == context.cwd)
                    {
                        self.context = Some(context);
                        self.watch_fallback = fallback;
                        for (path, entries) in directories {
                            self.dirs.insert(path, entries);
                        }
                    }
                }
                Update::ClipboardPaste(session, text) => {
                    // Resolve by captured identity, never by current focus.
                    if let Some(backend) = self.backends.get_mut(&session) {
                        backend
                            .process_command(egui_term::BackendCommand::Write(text.into_bytes()));
                    }
                }
                Update::Error(e) => {
                    if installation::is_helper_error(&e) {
                        self.installation_error = Some(e.clone());
                    }
                    if self.exit.active() {
                        self.cancel_exit(e.clone());
                    }
                    #[cfg(feature = "test-support")]
                    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                        eprintln!("Fixture error {e}");
                    }
                    if daemon_connection::is_connection_error(&e) {
                        self.connected = false;
                    }
                    self.error = Some(e);
                }
                Update::Info(i) => self.info = Some(i),
            }
        }
        while let Ok((id, event)) = self.pty_rx.try_recv() {
            if let PtyEvent::ClipboardStore(_, ref text) = event {
                ctx.copy_text(text.clone());
            }
            if let PtyEvent::Exit = event
                && let Some(session) = self.backend_ids.remove(&id)
                && self.backends.get(&session).is_some_and(|b| b.id() == id)
            {
                self.backends.remove(&session);
            }
        }
    }
    fn apply_state(&mut self, mut state: State) {
        if self
            .error
            .as_deref()
            .is_some_and(daemon_connection::is_connection_error)
        {
            self.error = None;
        }
        if state.generation != self.state.generation
            || state.attachment_helper_available == Some(true)
        {
            self.installation_error = None;
            if self
                .error
                .as_deref()
                .is_some_and(installation::is_helper_error)
            {
                self.error = None;
            }
        }
        self.connected = true;
        let initial = !self.state_loaded;
        self.state_loaded = true;
        let ended_sessions: Vec<_> = state
            .sessions
            .iter()
            .filter(|session| {
                session.lifecycle == Lifecycle::Ended
                    && (initial
                        || self
                            .state
                            .sessions
                            .iter()
                            .any(|old| old.id == session.id && old.lifecycle.live()))
            })
            .cloned()
            .collect();
        for p in &state.projects {
            if !self.layouts.contains_key(&p.id) {
                let dock = match Workspace::load(p.layout.clone()) {
                    Ok(workspace) => workspace,
                    Err(error) => {
                        self.error = Some(format!(
                            "{}: {error:#}. Layout will not be overwritten.",
                            p.name
                        ));
                        self.layout_readonly.insert(p.id.clone());
                        Workspace::empty()
                    }
                };
                self.layout_saved.insert(p.id.clone(), p.layout.to_string());
                self.layouts.insert(p.id.clone(), dock);
            }
        }
        for ended in ended_sessions {
            if self
                .rename_session
                .as_ref()
                .is_some_and(|(sid, _)| sid == &ended.id)
            {
                self.rename_session = None;
            }

            #[cfg(feature = "test-support")]
            if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                eprintln!("Fixture ended {:?}", ended.kind);
            }
            let was_active = self.active_session.as_ref() == Some(&ended.id);
            let old_group = self
                .layouts
                .get(&ended.project_id)
                .and_then(|workspace| {
                    workspace.tabs.iter().find(|tab| {
                        tab.layout
                            .find_tab(&Tab::Terminal(ended.id.clone()))
                            .is_some()
                    })
                })
                .map(|tab| tab.id.clone());
            let anchors = self.editor_origins.remove(&ended.id).unwrap_or_default();
            self.remove_tab(&ended.id);
            if was_active
                && self.selected.as_ref() == Some(&ended.project_id)
                && let Some(dock) = self.layouts.get_mut(&ended.project_id)
            {
                let live_tab = |tab: &Tab| match tab {
                    Tab::Terminal(sid) => state
                        .sessions
                        .iter()
                        .any(|s| &s.id == sid && s.lifecycle.live()),
                    Tab::Diff { .. } | Tab::Image { .. } => true,
                };
                let survives = old_group
                    .as_ref()
                    .is_some_and(|id| dock.tabs.iter().any(|tab| &tab.id == id));
                if survives {
                    dock.active = old_group.unwrap();
                } else if let Some(tab) = anchors
                    .iter()
                    .find(|tab| live_tab(tab) && dock.contains(tab))
                {
                    dock.activate_containing(tab);
                }
                let target = if survives {
                    anchors
                        .iter()
                        .filter(|tab| live_tab(tab))
                        .find_map(|tab| dock.find_tab(tab))
                } else {
                    None
                }
                .or_else(|| {
                    dock.active_pane()
                        .filter(|tab| live_tab(tab))
                        .and_then(|tab| dock.find_tab(tab))
                })
                .or_else(|| {
                    dock.iter_all_tabs()
                        .find(|(_, tab)| live_tab(tab))
                        .map(|(path, _)| path)
                });
                if let Some(path) = target {
                    let _ = dock.set_active_tab(path);
                    dock.set_focused_node_and_surface(path.node_path());
                    self.active_session = dock
                        .leaf(path.node_path())
                        .ok()
                        .and_then(|leaf| leaf.tabs.get(leaf.active.0))
                        .and_then(|tab| match tab {
                            Tab::Terminal(sid) => Some(sid.clone()),
                            _ => None,
                        });
                }
            }
        }
        if self.selected.is_none() {
            self.selected = state
                .selected_project
                .clone()
                .filter(|id| !self.preferences.hidden_projects.contains(id))
                .or_else(|| {
                    state
                        .projects
                        .iter()
                        .find(|p| !self.preferences.hidden_projects.contains(&p.id))
                        .map(|p| p.id.clone())
                });
        }
        state
            .settings
            .keybindings
            .entry("open_file".into())
            .or_insert_with(|| "command+O".into());
        self.preferences.markdown_modes.retain(|sid, _| {
            state
                .sessions
                .iter()
                .any(|s| &s.id == sid && markdown::available(s))
        });
        self.state = state;
        self.migrate_attention();

        if self.preferences_writable
            && !self.preferences.typography_migrated
            && !self.migration_requested
        {
            self.migration_requested = true;
            let _ = self.jobs.send(Job::MigrateTypography);
        }
    }
    fn migrate_attention(&mut self) {
        if self.preferences_writable
            && !self.preferences.attention_migrated
            && !self.attention_pending
            && self
                .attention_requested
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
        {
            self.attention_requested = Some(Instant::now());
            self.attention_pending = self.jobs.send(Job::MigrateAttention).is_ok();
        }
    }
    fn select_project(&mut self, project: String) {
        // Explicit navigation or reopening a folder restores its sidebar entry.
        self.preferences.hidden_projects.remove(&project);
        if self.selected.as_ref() != Some(&project) {
            self.finish_rename(true);
        }
        self.selection_generation = self.selection_generation.wrapping_add(1);
        self.selected = Some(project.clone());
        self.active_session = self
            .layouts
            .get_mut(&project)
            .and_then(|d| d.main_surface_mut().find_active_focused())
            .and_then(|(_, tab)| match tab {
                Tab::Terminal(id) => Some(id.clone()),
                _ => None,
            });
        self.send(Request::SelectProject { project });
    }
    fn hide_project(&mut self, project: &str) {
        if !self.state.projects.iter().any(|p| p.id == project) {
            return;
        }
        self.preferences.hidden_projects.insert(project.into());
        // Also invalidate an outstanding folder-picker result, including when
        // a background project was removed through its context menu.
        self.selection_generation = self.selection_generation.wrapping_add(1);
        if self.selected.as_deref() == Some(project) {
            self.finish_rename(true);
            self.selected = None;
            self.active_session = None;
            if let Some(next) = self
                .state
                .projects
                .iter()
                .find(|p| !self.preferences.hidden_projects.contains(&p.id))
                .map(|p| p.id.clone())
            {
                self.select_project(next);
            }
        }
        self.info = Some("Project removed from the sidebar. Add the folder again to restore it; its files and sessions are kept.".into());
    }
    fn insert(&mut self, project: &str, tab: Tab, split: Option<&str>) {
        let dock = self
            .layouts
            .entry(project.into())
            .or_insert_with(Workspace::empty);
        if matches!(tab, Tab::Image { .. }) {
            dock.version = 3;
        }
        if let Some(path) = dock.find_tab(&tab) {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
            return;
        }
        if let Some(direction) = split {
            let tree = dock.main_surface_mut();
            if !tree.is_empty() {
                let node = tree.focused_leaf().unwrap_or(NodeIndex::root());
                let result = match direction {
                    "left" => tree.split_left(node, 0.5, vec![tab]),
                    "up" => tree.split_above(node, 0.5, vec![tab]),
                    "down" => tree.split_below(node, 0.5, vec![tab]),
                    _ => tree.split_right(node, 0.5, vec![tab]),
                };
                tree.set_focused_node(result[1]);
                return;
            }
        }
        dock.push_to_focused_leaf(tab);
    }
    fn selected_project(&self) -> Option<&Project> {
        self.state
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected.as_ref())
    }
    fn context_session(&self) -> Option<&Session> {
        self.state
            .sessions
            .iter()
            .find(|s| {
                Some(&s.id) == self.active_session.as_ref()
                    && s.kind == SessionKind::Shell
                    && Some(&s.project_id) == self.selected.as_ref()
            })
            .or_else(|| {
                self.selected
                    .as_ref()
                    .and_then(|p| self.terminal_context.get(p))
                    .and_then(|id| self.state.sessions.iter().find(|s| &s.id == id))
            })
    }
    fn cwd(&self) -> Option<PathBuf> {
        self.context_session()
            .map(|s| s.cwd.clone())
            .or_else(|| self.selected_project().map(|p| p.path.clone()))
    }
    fn dialog_directory(&self) -> PathBuf {
        self.cwd()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| "/".into())
    }
    fn create(&self, split: Option<&str>) {
        if let Some(project) = &self.selected {
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project: project.clone(),
                    cwd: self.cwd(),
                    file: None,
                    line: None,
                    column: None,
                    editor: false,
                },
                if split.is_none() {
                    After::Workspace(id(), Vec::new())
                } else {
                    self.editor_target(project, None, split)
                },
            ));
        }
    }
    fn editor_target(&self, project: &str, origin: Option<&Tab>, split: Option<&str>) -> After {
        let mut anchors = Vec::new();

        if let Some(dock) = self.layouts.get(project) {
            let path = origin
                .and_then(|tab| dock.find_tab(tab).map(|path| path.node_path()))
                .or_else(|| {
                    dock.main_surface()
                        .focused_leaf()
                        .map(|node| egui_dock::NodePath {
                            surface: egui_dock::SurfaceIndex::main(),
                            node,
                        })
                });
            if let Some(path) = path
                && let Ok(leaf) = dock.leaf(path)
            {
                if let Some(tab) = leaf.tabs.get(leaf.active.0) {
                    anchors.push(tab.clone());
                }
                anchors.extend(
                    leaf.tabs
                        .iter()
                        .filter(|tab| !anchors.contains(tab))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            }
        } else if self.selected.as_deref() == Some(project)
            && let Some(origin) = origin
            && let Some(path) = self.pane_by_tab.get(&origin.key())
        {
            anchors.push(origin.clone());
            anchors.extend(
                self.pane_tabs
                    .get(path)
                    .into_iter()
                    .flatten()
                    .filter(|tab| *tab != origin)
                    .cloned(),
            );
        }
        if let Some(split) = split {
            After::CreateAt(anchors, Some(split.into()))
        } else {
            After::Workspace(id(), anchors)
        }
    }
    fn begin_rename(&mut self, sid: &str, surface: RenameSurface) {
        if let Some(session) = self.state.sessions.iter().find(|s| s.id == sid) {
            self.rename_session = Some((sid.into(), session.label.clone()));
            self.rename_focus = true;
            self.rename_surface = surface;
        }
    }
    fn finish_rename(&mut self, save: bool) {
        if let Some((sid, title)) = self.rename_session.take()
            && save
            && !title.trim().is_empty()
            && title.trim().len() <= 256
        {
            self.send(Request::Rename {
                session: sid,
                label: title.trim().into(),
            });
        }
    }
    fn renaming(&self, sid: &str, surface: RenameSurface) -> bool {
        self.rename_surface == surface
            && self
                .rename_session
                .as_ref()
                .is_some_and(|(target, _)| target == sid)
    }
    fn open_file(&mut self, path: PathBuf, line: Option<u32>, split: Option<&str>, external: bool) {
        self.open_file_mode(path, line, split, external, false);
    }
    fn open_file_mode(
        &mut self,
        path: PathBuf,
        line: Option<u32>,
        split: Option<&str>,
        external: bool,
        text: bool,
    ) {
        if !external && !text && image_preview::supported(&path) {
            if let Some(project) = self.selected.clone() {
                self.open_image(&project, path, split);
            }
            return;
        }
        if external || self.state.settings.editor_mode == EditorMode::External {
            let _ = self.jobs.send(Job::External(path));
            return;
        }
        if let Some(project) = &self.selected {
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project: project.clone(),
                    cwd: self.cwd(),
                    file: Some(path),
                    line,
                    column: None,
                    editor: true,
                },
                self.editor_target(project, None, split),
            ));
        }
    }
    fn open_image(&mut self, project: &str, path: PathBuf, split: Option<&str>) {
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), split);
        let _ = self.update_tx.send(Update::OpenImage(
            project.into(),
            std::path::absolute(&path).unwrap_or(path),
            after,
        ));
    }
    fn go_session(&mut self, sid: &str) {
        self.finish_rename(true);
        if let Some(s) = self.state.sessions.iter().find(|s| s.id == sid).cloned() {
            self.select_project(s.project_id.clone());
            let pane = Tab::Terminal(sid.into());
            let workspace = self
                .layouts
                .entry(s.project_id.clone())
                .or_insert_with(Workspace::empty);
            if !workspace.activate_containing(&pane) {
                workspace.add(id(), pane.clone());
            }
            self.insert(&s.project_id, pane, None);
            self.active_session = Some(s.id.clone());
            self.send(Request::SelectProject {
                project: s.project_id,
            });
            self.send(Request::Focus { session: s.id });
        }
    }
    fn save_layouts(&mut self) {
        for (project, dock) in &self.layouts {
            if self.layout_readonly.contains(project) {
                continue;
            }
            if let Ok(value) = serde_json::to_value(dock) {
                let value = sanitize_layout(value);
                let text = value.to_string();
                if self.layout_saved.get(project) != Some(&text) {
                    #[cfg(feature = "test-support")]
                    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                        eprintln!("Fixture save {} tabs", dock.iter_all_tabs().count());
                    }
                    self.send(Request::SaveLayout {
                        project: project.clone(),
                        layout: value,
                    });
                    self.layout_saved.insert(project.clone(), text);
                }
            }
        }
    }
    fn remove_tab(&mut self, sid: &str) {
        for workspace in self.layouts.values_mut() {
            workspace.remove_session(sid);
        }
        self.backends.remove(sid);
        if self.active_session.as_deref() == Some(sid) {
            self.active_session = None;
        }
    }
    fn terminal_action(
        &mut self,
        ctx: &egui::Context,
        session: &Session,
        target: &services::Target,
        action: FileAction,
    ) {
        if action == FileAction::Copy {
            ctx.copy_text(target.display());
            return;
        }
        match target {
            services::Target::Url(url) => {
                let _ = self.jobs.send(Job::Browser(url.clone()));
            }
            services::Target::File(path, line, column) => {
                if image_preview::supported(path)
                    && matches!(action, FileAction::Open | FileAction::Split)
                {
                    self.open_image(
                        &session.project_id,
                        path.clone(),
                        (action == FileAction::Split).then_some("right"),
                    );
                    return;
                }
                if action == FileAction::External
                    || self.state.settings.editor_mode == EditorMode::External
                {
                    let _ = self.jobs.send(Job::External(path.clone()));
                } else {
                    let origin = Tab::Terminal(session.id.clone());
                    let after = self.editor_target(
                        &session.project_id,
                        Some(&origin),
                        (action == FileAction::Split).then_some("right"),
                    );
                    let _ = self.jobs.send(Job::rpc(
                        Request::Create {
                            project: session.project_id.clone(),
                            cwd: Some(session.cwd.clone()),
                            file: Some(path.clone()),
                            line: *line,
                            column: *column,
                            editor: true,
                        },
                        after,
                    ));
                }
            }
        }
    }
    fn file_action(
        &mut self,
        ui: &egui::Ui,
        action: FileAction,
        path: &std::path::Path,
        line: Option<u32>,
    ) {
        match action {
            FileAction::Open => self.open_file(path.into(), line, None, false),
            FileAction::Text => self.open_file_mode(path.into(), line, None, false, true),
            FileAction::Split => self.open_file(path.into(), line, Some("right"), false),
            FileAction::External => self.open_file(path.into(), line, None, true),
            FileAction::Copy => ui.ctx().copy_text(path.display().to_string()),
            FileAction::StagedDiff | FileAction::WorkingDiff => {
                if let Some(root) = self.context.as_ref().and_then(|c| c.root.clone()) {
                    self.add_diff(root, path.into(), action == FileAction::StagedDiff);
                }
            }
            FileAction::Browser => {}
        }
    }
    fn add_diff(&mut self, cwd: PathBuf, path: PathBuf, staged: bool) {
        if let Some(project) = self.selected.clone() {
            if !self
                .state
                .capabilities
                .iter()
                .any(|c| c == NVIM_REVIEW_CAPABILITY)
            {
                let tab = Tab::Diff { cwd, path, staged };
                self.layouts
                    .entry(project)
                    .or_insert_with(Workspace::empty)
                    .add(id(), tab.clone());
                self.active_session = None;
                self.loading.insert(tab.key());
                self.error = None;
                self.info = Some("Using built-in diff. Neovim review needs the updated daemon; restart it after finishing your live sessions.".into());
                let _ = self.jobs.send(Job::Diff(tab));
                return;
            }
            let _ = self.jobs.send(Job::rpc(
                Request::CreateReview {
                    project: project.clone(),
                    cwd,
                    path,
                    staged,
                },
                After::Workspace(id(), vec![]),
            ));
        }
    }
    fn editors_only(&self, ids: &[String]) -> bool {
        !ids.is_empty()
            && !self.state.agents.iter().any(|agent| {
                ids.contains(&agent.session_id)
                    && !matches!(
                        agent.state,
                        AgentState::Completed | AgentState::Failed | AgentState::Stopped
                    )
            })
            && ids.iter().all(|id| {
                self.state
                    .sessions
                    .iter()
                    .any(|s| &s.id == id && s.kind == SessionKind::Editor)
            })
    }
    fn editor_close_busy(&self, ids: &[String]) -> bool {
        ids.iter().any(|id| self.editor_close_sessions.contains(id))
    }

    fn editor_close_prompted(&self, ids: &[String]) -> bool {
        self.editor_close_prompts
            .iter()
            .any(|(_, prompted, _)| prompted.iter().any(|id| ids.contains(id)))
    }

    fn skip_editor_close_request(&self, ids: &[String]) -> bool {
        self.editor_close_busy(ids) || self.editor_close_prompted(ids)
    }

    fn unsaved_close_prompt(
        &self,
        sid: &str,
    ) -> Option<(editor_close::Target, Vec<String>, String)> {
        self.editor_close_prompts
            .iter()
            .find(|(_, ids, _)| ids.iter().any(|id| id == sid))
            .cloned()
    }

    fn upsert_unsaved_close(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        error: String,
    ) {
        if let Some(prompt) = self
            .editor_close_prompts
            .iter_mut()
            .find(|(_, prompted, _)| prompted == &ids || prompted.iter().any(|id| ids.contains(id)))
        {
            *prompt = (target, ids, error);
            return;
        }
        self.editor_close_prompts.push((target, ids, error));
    }

    fn apply_unsaved_close_choice(
        &mut self,
        choice: appearance::UnsavedCloseChoice,
        target: editor_close::Target,
        ids: Vec<String>,
    ) {
        match choice {
            appearance::UnsavedCloseChoice::Cancel => {
                self.editor_close_prompts
                    .retain(|(_, prompted, _)| prompted != &ids);
            }
            appearance::UnsavedCloseChoice::Save => {
                self.close_editors(target, ids, editor_close::Mode::Save);
            }
            appearance::UnsavedCloseChoice::Discard => {
                self.close_editors(target, ids, editor_close::Mode::Discard);
            }
        }
    }

    fn editors_closed(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        result: Result<(), String>,
    ) {
        for id in &ids {
            self.editor_close_sessions.remove(id);
        }
        match result {
            Ok(()) => {
                self.editor_close_prompts
                    .retain(|(_, prompted, _)| !prompted.iter().any(|id| ids.contains(id)));
                match target {
                    editor_close::Target::Workspace(project, id) => {
                        if let Some(workspace) = self.layouts.get_mut(&project) {
                            workspace.close(&id);
                        }
                    }
                    editor_close::Target::Pane(sid) => self.remove_tab(&sid),
                }
            }
            Err(error) => self.upsert_unsaved_close(target, ids, error),
        }
    }

    fn close_editors(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        mode: editor_close::Mode,
    ) {
        if self.editor_close_busy(&ids) {
            return;
        }
        self.editor_close_sessions.extend(ids.iter().cloned());
        let _ = self.jobs.send(Job::CloseEditors(target, ids, mode));
    }
}
fn shortcut(ctx: &egui::Context, value: &str) -> bool {
    let parts = value.to_lowercase();
    let pieces = parts.split('+').collect::<Vec<_>>();
    let Some(key) = pieces.last().and_then(|s| egui::Key::from_name(s)) else {
        return false;
    };
    let command = pieces.contains(&"command");
    let modifiers = egui::Modifiers {
        alt: pieces.contains(&"alt"),
        ctrl: pieces.contains(&"ctrl") || command && !cfg!(target_os = "macos"),
        shift: pieces.contains(&"shift") || command && !cfg!(target_os = "macos"),
        mac_cmd: command && cfg!(target_os = "macos"),
        command,
    };
    ctx.input_mut(|i| i.consume_key(modifiers, key))
}
impl eframe::App for App {
    #[cfg(feature = "test-support")]
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.diagnostics.input(ctx, input);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // eframe calls logic even while hidden/minimized; ui is rendering-only.
        // IPC and exit checkpoints must not depend on a visible window.
        self.process_updates(ctx);
        if updater::termination_cancelled() {
            self.native_installation_cancelled();
        }
        if (updater::termination_requested() || ctx.input(|i| i.viewport().close_requested()))
            && !matches!(self.exit, exit::Exit::Ready)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.begin_exit();
        }
        self.advance_exit(ctx);
        if !self.exit.active() && self.last_heartbeat.elapsed() > Duration::from_secs(1) {
            self.send(Request::Heartbeat {
                focused: ctx.input(|i| {
                    i.viewport().focused.unwrap_or(false)
                        && !i.viewport().minimized.unwrap_or(false)
                }),
            });
            self.last_heartbeat = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.popups.begin_frame(&ctx);
        if self.exit.active() {
            ui.centered_and_justified(|ui| {
                ui.label("Saving workspace before closing…");
            });
            return;
        }
        self.visible_dirs.clear();
        self.visible_sessions.clear();
        self.visible_images.clear();
        self.markdown.begin_frame();
        #[cfg(feature = "test-support")]
        self.diagnostics.frame(&ctx);
        for (action, key) in self.state.settings.keybindings.clone() {
            if shortcut(&ctx, &key) {
                match action.as_str() {
                    "open_file" => self.open_path = true,
                    "new_terminal" => self.create(None),
                    "split_right" => self.create(Some("right")),
                    "split_down" => self.create(Some("down")),
                    "next_pane" => {
                        if let Some(d) =
                            self.selected.as_ref().and_then(|p| self.layouts.get_mut(p))
                        {
                            let nodes = d
                                .main_surface()
                                .iter()
                                .enumerate()
                                .filter(|(_, n)| n.is_leaf())
                                .map(|(i, _)| NodeIndex(i))
                                .collect::<Vec<_>>();
                            if !nodes.is_empty() {
                                let current = d.main_surface().focused_leaf();
                                let idx = nodes
                                    .iter()
                                    .position(|n| Some(*n) == current)
                                    .map(|i| (i + 1) % nodes.len())
                                    .unwrap_or(0);
                                d.main_surface_mut().set_focused_node(nodes[idx]);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if self.state_loaded {
            self.migrate_attention();
        }
        egui::Panel::top("window-header")
            .exact_size(40.0)
            .frame(egui::Frame::NONE.fill(appearance::color(&self.theme.surface)))
            .show(ui, |ui| self.window_header(ui));
        if !self.state.settings.notifications_side {
            egui::Panel::top("attention").show(ui, |ui| self.notifications(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(
                    appearance::color(if self.connected {
                        &self.theme.status_running
                    } else {
                        &self.theme.status_waiting
                    }),
                    if self.connected {
                        "● Connected"
                    } else {
                        "○ Connecting"
                    },
                )
                .on_hover_text(format!(
                    "GUI {}\nDaemon {}\nDaemon executable: {}\nAttachment helper: {}",
                    env!("CARGO_PKG_VERSION"),
                    self.state
                        .daemon_version
                        .as_deref()
                        .unwrap_or("unknown (older daemon)"),
                    self.state
                        .daemon_executable
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "not reported by this daemon".into()),
                    match self.state.attachment_helper_available {
                        Some(true) => "available",
                        Some(false) => "unavailable",
                        None => "not reported by this daemon",
                    }
                ));
                ui.separator();
                if self.installation_problem() {
                    ui.colored_label(
                        appearance::color(&self.theme.status_failed),
                        "Terminal helper needs repair. Existing sessions are preserved.",
                    );
                    let repair = ui.small_button("Fix installation…");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "fix-installation", repair.rect);
                    if repair.clicked() {
                        self.open_installation_settings();
                    }
                } else if let Some(error) = self.error.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(appearance::color(&self.theme.status_failed), error);
                        if ui.small_button("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                } else if let Some(message) = &self.state.degraded {
                    ui.colored_label(appearance::color(&self.theme.status_waiting), message);
                } else if self.state_loaded
                    && (self.state.daemon_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
                        || !self
                            .state
                            .capabilities
                            .iter()
                            .any(|c| c == STABLE_HELPER_CAPABILITY))
                {
                    ui.weak("App and session service use different installations.");
                    if ui.small_button("Review installation…").clicked() {
                        self.open_installation_settings();
                    }
                } else if let Some(info) = self.info.clone() {
                    ui.horizontal(|ui| {
                        ui.label(info);
                        if ui.small_button("×").clicked() {
                            self.info = None;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.weak(format!(
                            "{} sessions running",
                            self.state
                                .sessions
                                .iter()
                                .filter(|s| s.lifecycle.live())
                                .count()
                        ));
                    });
                }
                if let Some(metadata) = self.metadata.clone() {
                    if let Some(branch) = metadata.branch {
                        ui.weak(if metadata.worktree {
                            format!("Worktree · {branch}")
                        } else {
                            branch
                        });
                    }
                    if let Some(pr) = metadata.pull_request
                        && ui
                            .link(format!("PR #{}", pr.number))
                            .on_hover_text(pr.title)
                            .clicked()
                    {
                        let _ = self.jobs.send(Job::Browser(pr.url));
                    }
                    for port in metadata.ports.iter().take(3) {
                        if ui
                            .link(format!(":{}", port.port))
                            .on_hover_text(&port.address)
                            .clicked()
                        {
                            let _ = self.jobs.send(Job::Browser(port.url()));
                        }
                    }
                }
            });
        });
        let projects_response = egui::Panel::left("projects")
            .resizable(true)
            .default_size(225.0)
            .size_range(170.0..=420.0)
            .show(ui, |ui| {
                self.agent_bar(ui);
                ui.push_id("left-sidebar-content", |ui| {
                    if self.preferences.left_agents {
                        self.agents_view(ui);
                    } else {
                        appearance::sidebar_scroll("left-projects")
                            .show(ui, |ui| self.projects(ui));
                    }
                });
            });
        self.project_width = projects_response.response.rect.width();
        let response = egui::Panel::right("context")
            .resizable(true)
            .default_size(self.preferences.width)
            .size_range(220.0..=480.0)
            .show(ui, |ui| {
                if self.state.settings.notifications_side {
                    self.notifications(ui);
                    ui.separator();
                }
                if self.preferences.visible {
                    self.sidebar(ui);
                }
            });
        self.preferences.width = response.response.rect.width().clamp(220.0, 480.0);
        if self.preferences_writable
            && !self.preferences_pending
            && self.preferences != self.preferences_saved
        {
            let _ = self.jobs.send(Job::Preferences(self.preferences.clone()));
            self.preferences_pending = true;
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(appearance::color(&self.theme.window))
                    .inner_margin(2),
            )
            .show(ui, |ui| {
                if let Some(project) = self.selected.clone() {
                    let mut dock = self
                        .layouts
                        .remove(&project)
                        .unwrap_or_else(Workspace::empty);
                    match dock
                        .main_surface_mut()
                        .find_active_focused()
                        .map(|(_, tab)| tab.clone())
                    {
                        Some(Tab::Terminal(sid)) => self.active_session = Some(sid),
                        Some(Tab::Diff { .. } | Tab::Image { .. }) => self.active_session = None,
                        None => {}
                    }
                    if dock.iter_all_tabs().next().is_none() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() * 0.3);
                            ui.heading("Your workspace, ready.");
                            ui.label("Open a terminal. Run the tools you already use.");
                            ui.add_space(12.0);
                            if ui.button("Open terminal").clicked() {
                                self.create(None);
                            }
                        });
                    } else {
                        let mut style = egui_dock::Style::from_egui(ui.style());
                        style.tab_bar.height = 32.0;
                        style.buttons.add_tab_align = egui_dock::style::TabAddAlign::Left;
                        style.separator.width = self.theme.pane_divider_width;
                        style.separator.color_idle = appearance::color(&self.theme.window);
                        style.main_surface_border_rounding = egui::CornerRadius::same(2);
                        style.tab.tab_body.corner_radius = egui::CornerRadius::same(2);
                        self.pane_by_tab = dock
                            .iter_all_tabs()
                            .map(|(path, tab)| (tab.key(), path.node_path()))
                            .collect();
                        self.pane_tabs = self
                            .pane_by_tab
                            .values()
                            .map(|path| {
                                (
                                    *path,
                                    dock.leaf(*path)
                                        .map(|leaf| leaf.tabs.clone())
                                        .unwrap_or_default(),
                                )
                            })
                            .collect();
                        DockArea::new(&mut dock)
                            .style(style)
                            .show_add_buttons(true)
                            .show_leaf_close_all_buttons(false)
                            .show_leaf_collapse_buttons(false)
                            .show_inside(ui, &mut Viewer { app: self });
                    }
                    if let Some(tab) = self.focus_tab.take()
                        && let Some(path) = dock.find_tab(&tab)
                    {
                        let _ = dock.set_active_tab(path);
                        dock.set_focused_node_and_surface(path.node_path());
                    }
                    if let Some((path, split)) = self.add_tab.take() {
                        let cwd = dock
                            .leaf(path)
                            .ok()
                            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
                            .and_then(|tab| match tab {
                                Tab::Terminal(id) => self
                                    .state
                                    .sessions
                                    .iter()
                                    .find(|s| &s.id == id)
                                    .map(|s| s.cwd.clone()),
                                Tab::Diff { cwd, .. } => Some(cwd.clone()),
                                Tab::Image { path } => path.parent().map(PathBuf::from),
                            })
                            .or_else(|| self.selected_project().map(|p| p.path.clone()));
                        let _ = self.jobs.send(Job::rpc(
                            Request::Create {
                                project: project.clone(),
                                cwd,
                                file: None,
                                line: None,
                                column: None,
                                editor: false,
                            },
                            if split.is_none() {
                                After::Workspace(id(), vec![])
                            } else {
                                After::CreateAt(
                                    dock.leaf(path)
                                        .map(|leaf| leaf.tabs.clone())
                                        .unwrap_or_default(),
                                    split,
                                )
                            },
                        ));
                    }
                    if self.highlight_session != self.active_session {
                        self.highlight_session = self.active_session.clone();
                        self.highlight_since = Instant::now();
                    }
                    if let Some(sid) = &self.active_session
                        && let Some(path) = dock.find_tab(&Tab::Terminal(sid.clone()))
                        && let Ok(leaf) = dock.leaf(path.node_path())
                    {
                        ui.painter().rect_stroke(
                            leaf.rect.shrink(1.0),
                            2,
                            appearance::focus_stroke(
                                appearance::color(&self.theme.accent),
                                self.highlight_since.elapsed(),
                            ),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if self.highlight_since.elapsed() < Duration::from_millis(1200) {
                        ctx.request_repaint_after(Duration::from_millis(16));
                    }
                    self.layouts.insert(project, dock);
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.3);
                        let empty = self.state.projects.is_empty();
                        ui.heading(if empty {
                            "A home for your terminals."
                        } else {
                            "No project selected."
                        });
                        ui.label(if empty {
                            "Persistent sessions. Project layouts. Agents within reach."
                        } else {
                            "Restore a project from Removed, or add a folder."
                        });
                        ui.add_space(15.0);
                        if ui
                            .button(if empty {
                                "Add your first project"
                            } else {
                                "Add project"
                            })
                            .clicked()
                        {
                            self.add_project = true;
                        }
                    });
                }
            });
        self.images
            .retain(|path, _| self.visible_images.contains(path));
        self.markdown.end_frame(&ctx);
        self.backends
            .retain(|sid, _| self.visible_sessions.contains(sid));
        if let Some(session) =
            self.state.sessions.iter().find(|s| {
                Some(&s.id) == self.active_session.as_ref() && s.kind == SessionKind::Shell
            })
        {
            self.terminal_context
                .insert(session.project_id.clone(), session.id.clone());
        }
        if self.active_session != self.last_focus {
            if let Some(session) = &self.active_session {
                self.send(Request::Focus {
                    session: session.clone(),
                });
            }
            self.last_focus = self.active_session.clone();
        }
        let next_metadata = self.cwd().map(|cwd| metadata_refresh::Request {
            cwd,
            identity: self
                .context_session()
                .and_then(|s| s.pid.map(|pid| (pid, s.created))),
            include_pr: self.state.settings.pr_metadata,
            generation: self.metadata_generation,
        });
        if next_metadata != self.metadata_request {
            self.metadata_generation = self.metadata_generation.wrapping_add(1);
            self.metadata = None;
            self.metadata_request = next_metadata.map(|mut r| {
                r.generation = self.metadata_generation;
                r
            });
            let _ = self.metadata_jobs.send(self.metadata_request.clone());
        }
        self.visible_dirs.sort();
        self.visible_dirs.dedup();
        let wants_files = self.preferences.visible
            && matches!(
                self.preferences.tool,
                SidebarTool::Explorer | SidebarTool::Git
            );
        let next = self
            .cwd()
            .filter(|_| wants_files)
            .map(|cwd| refresh::Request {
                cwd,
                generation: self.refresh_generation,
                directories: self.visible_dirs.clone(),
            });
        if next != self.refresh_request {
            self.refresh_generation += 1;
            let next = next.map(|mut r| {
                r.generation = self.refresh_generation;
                r
            });
            let cwd = next.as_ref().map(|r| r.cwd.clone());
            if self.context_path != cwd {
                self.context = None;
                self.dirs.clear();
            }
            self.context_path = cwd;
            self.refresh_request = next.clone();
            let _ = self.refresh.send(next);
        }
        if self.last_save.elapsed() > Duration::from_secs(1) {
            self.save_layouts();
            self.last_save = Instant::now();
        }
        if cfg!(target_os = "linux") {
            window_resize_edges(ui);
        }
        self.modals(&ctx, frame);
        self.popups.end_frame();
        appearance::click_cursor(&ctx);
        #[cfg(feature = "test-support")]
        self.diagnostics.capture(&ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}
#[cfg(test)]
mod daemon_compatibility_tests {
    use super::*;
    #[test]
    fn unknown_healthy_current_and_newer_daemons_are_preserved() {
        let mut state = State {
            attachment_helper_available: Some(true),
            capabilities: vec![STABLE_HELPER_CAPABILITY.into()],
            ..State::default()
        };
        assert!(!can_retire_daemon(&state));
        state.daemon_version = Some("0.0.1".into());
        assert!(!can_retire_daemon(&state));
        state.capabilities.push(SHUTDOWN_IF_IDLE_CAPABILITY.into());
        assert!(can_retire_daemon(&state));
        for version in ["unknown", env!("CARGO_PKG_VERSION"), "999.0.0"] {
            state.daemon_version = Some(version.into());
            assert!(!can_retire_daemon(&state));
        }
    }
}
fn main() -> Result<()> {
    if !installation::preflight()? {
        return Ok(());
    }
    let args = std::env::args().collect::<Vec<_>>();
    let paths = if let Some(i) = args.iter().position(|a| a == "--data-dir") {
        Paths::at(args.get(i + 1).context("Missing data directory")?.into())
    } else {
        Paths::discover()?
    };
    paths.init()?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.runtime.join("ui.lock"))?;
    if lock.try_lock_exclusive().is_err() {
        eprintln!("Terminator is already open.");
        return Ok(());
    }
    daemon_connection::ensure_running(&paths, &std::env::current_exe()?)?;
    let window_size = [1440.0, 900.0];
    #[cfg(feature = "test-support")]
    let window_size = {
        let scale = std::env::var("TERMINATOR_TEST_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|s| (1.0..=2.0).contains(s))
            .unwrap_or(1.0);
        let size = if std::env::var_os("TERMINATOR_TEST_NARROW").is_some() {
            [900.0, 650.0]
        } else {
            window_size
        };
        [size[0] * scale, size[1] * scale]
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../assets/branding/terminator.png"
                ))
                .expect("bundled Terminator icon must be a valid PNG"),
            )
            .with_active(
                !(cfg!(feature = "test-support")
                    && std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some()
                    && std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some()),
            )
            .with_inner_size(window_size)
            .with_min_inner_size([900.0, 550.0])
            .with_fullsize_content_view(cfg!(target_os = "macos"))
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(true)
            .with_movable_by_background(false)
            .with_decorations(cfg!(target_os = "macos")),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Terminator",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, paths)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    #[test]
    fn unrendered_layout_roundtrips_all_tabs() {
        let mut dock = DockState::new(vec![Tab::Terminal("first".into())]);
        dock.main_surface_mut().split_right(
            NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("second".into())],
        );
        let json = sanitize_layout(serde_json::to_value(&dock).unwrap());
        let restored: DockState<Tab> = serde_json::from_value(json).unwrap();
        assert_eq!(restored.iter_all_tabs().count(), 2);
        assert!(restored.find_tab(&Tab::Terminal("second".into())).is_some());
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    fn fixture() -> (App, egui::Context, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        app.preferences_writable = false;
        let state = State {
            projects: ["a", "b"]
                .into_iter()
                .map(|id| Project {
                    id: id.into(),
                    name: id.into(),
                    path: PathBuf::from(format!("/{id}")),
                    layout: serde_json::Value::Null,
                })
                .collect(),
            selected_project: Some("a".into()),
            ..Default::default()
        };
        app.apply_state(state);
        (app, ctx, dir)
    }
    fn session_fixture(sid: &str, kind: SessionKind) -> Session {
        Session {
            review: false,
            id: sid.into(),
            project_id: "a".into(),
            label: sid.into(),
            cwd: "/a".into(),
            kind,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "fixture".into(),
            pid: Some(42),
            truncated: false,
            cwd_confirmed: true,
        }
    }

    #[test]
    fn idle_legacy_or_broken_daemon_is_retired_but_live_sessions_are_preserved() {
        let mut state = State {
            daemon_version: Some(env!("CARGO_PKG_VERSION").into()),
            capabilities: vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()],
            ..State::default()
        };
        for health in [None, Some(false)] {
            state.attachment_helper_available = health;
            assert!(can_retire_daemon(&state));
            state
                .sessions
                .push(session_fixture("live", SessionKind::Shell));
            assert!(!can_retire_daemon(&state));
            state.sessions.clear();
        }
        state.attachment_helper_available = Some(true);
        // Even a healthy sibling helper must migrate to a pinned copy when idle.
        assert!(can_retire_daemon(&state));
        state.capabilities.push(STABLE_HELPER_CAPABILITY.into());
        assert!(!can_retire_daemon(&state));
        state.attachment_helper_available = Some(false);
        state.capabilities.clear();
        assert!(!can_retire_daemon(&state));
        state.capabilities.push(SHUTDOWN_IF_IDLE_CAPABILITY.into());
        state.daemon_version = Some("999.0.0".into());
        assert!(!can_retire_daemon(&state));
    }

    #[test]
    fn legacy_helper_error_opens_recovery_and_survives_unreported_health() {
        let (mut app, ctx, _dir) = fixture();
        app.state.generation = "old-daemon".into();
        app.update_tx
            .send(Update::Error(
                "Attachment helper unavailable: /AppTranslocation/old/terminator-hook".into(),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.installation_problem());
        app.error = None; // Dismissing a general error must not hide recovery.
        app.apply_state(app.state.clone());
        assert!(app.installation_problem());
        app.open_installation_settings();
        assert!(app.settings_open);
        assert_eq!(app.settings_section, 6);
        let mut repaired = app.state.clone();
        repaired.generation = "new-daemon".into();
        repaired.attachment_helper_available = Some(true);
        app.apply_state(repaired);
        assert!(!app.installation_problem());
    }

    #[test]
    fn reconnect_clears_transport_errors_but_preserves_failed_operations() {
        let (mut app, ctx, _dir) = fixture();
        for message in [
            "Reconnecting: Session daemon unavailable",
            "Session daemon unavailable: No such file or directory (os error 2)",
        ] {
            app.update_tx.send(Update::Error(message.into())).unwrap();
            app.process_updates(&ctx);
            assert!(!app.connected);
            app.update_tx
                .send(Update::State(Box::new(app.state.clone())))
                .unwrap();
            app.process_updates(&ctx);
            assert!(app.connected);
            assert!(app.error.is_none());
        }
        for message in [
            "Settings rejected",
            "Could not save workspace before repair",
        ] {
            app.update_tx.send(Update::Error(message.into())).unwrap();
            app.process_updates(&ctx);
            app.apply_state(app.state.clone());
            assert_eq!(app.error.as_deref(), Some(message));
        }
    }

    #[test]
    fn expired_gui_request_cannot_change_the_workspace_after_timeout() {
        let (mut app, ctx, _dir) = fixture();
        let (reply, result) = mpsc::sync_channel(1);
        app.update_tx
            .send(Update::UiRequest(
                terminator_core::ui_control::Request::OpenFile {
                    project: "b".into(),
                    path: "/b/late.txt".into(),
                    as_text: true,
                },
                reply,
                Instant::now() - Duration::from_secs(1),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert!(result.recv().unwrap().unwrap_err().contains("expired"));
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert!(!app.open_path);
    }

    #[test]
    fn repair_never_queues_shutdown_with_live_or_unsupported_sessions() {
        let (mut app, _, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.state.daemon_version = Some(env!("CARGO_PKG_VERSION").into());
        app.state.capabilities = vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()];
        app.state
            .sessions
            .push(session_fixture("unsaved-editor", SessionKind::Editor));
        app.begin_installation_repair();
        assert!(requests.try_recv().is_err());
        app.state.sessions.clear();
        app.state.capabilities.clear();
        app.begin_installation_repair();
        assert!(requests.try_recv().is_err());
        app.state
            .capabilities
            .push(SHUTDOWN_IF_IDLE_CAPABILITY.into());
        app.begin_installation_repair();
        app.begin_installation_repair();
        assert!(app.repair_pending);
        assert!(matches!(
            requests.try_recv().unwrap(),
            Job::RepairInstallation(_, _)
        ));
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn removing_projects_only_hides_sidebar_entries_and_survives_snapshots() {
        let (mut app, ctx, dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.insert("a", Tab::Terminal("shell".into()), None);
        let layouts = serde_json::to_value(&app.layouts).unwrap();
        let sessions = serde_json::to_value(&app.state.sessions).unwrap();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.hide_project("a");
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(app.preferences.hidden_projects.contains("a"));
        assert!(requests.try_iter().all(|job| matches!(job, Job::Control(request, _) if matches!(*request, Request::SelectProject { .. }))));
        app.hide_project("b");
        assert!(app.selected.is_none());
        app.apply_state(app.state.clone());
        assert!(app.selected.is_none());
        assert_eq!(serde_json::to_value(&app.layouts).unwrap(), layouts);
        assert_eq!(serde_json::to_value(&app.state.sessions).unwrap(), sessions);
        assert_eq!(app.state.projects.len(), 2);
        app.preferences.save(dir.path()).unwrap();
        assert_eq!(
            UiPreferences::load(dir.path())
                .unwrap()
                .hidden_projects
                .len(),
            2
        );
        // Adding the same folder again restores its existing ID and layout.
        app.update_tx
            .send(Update::OpenedProject(
                Box::new(app.state.clone()),
                "a".into(),
                app.selection_generation,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert!(!app.preferences.hidden_projects.contains("a"));
        assert_eq!(serde_json::to_value(&app.layouts).unwrap(), layouts);
        assert_eq!(serde_json::to_value(&app.state.sessions).unwrap(), sessions);
    }

    #[test]
    fn a_delayed_folder_open_does_not_restore_a_project_removed_afterward() {
        let (mut app, ctx, _dir) = fixture();
        let generation = app.selection_generation;
        app.hide_project("a");
        app.update_tx
            .send(Update::OpenedProject(
                Box::new(app.state.clone()),
                "a".into(),
                generation,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.preferences.hidden_projects.contains("a"));
        assert_eq!(app.selected.as_deref(), Some("b"));
    }
    #[test]
    #[cfg(feature = "test-support")]
    fn expanded_project_terminals_stay_indented_at_all_sidebar_sizes() {
        for width in [180.0, 280.0, 420.0] {
            for scale in [1.0, 2.0] {
                let (mut app, ctx, _dir) = fixture();
                ctx.set_pixels_per_point(scale);
                let first = session_fixture("first-shell", SessionKind::Shell);
                let mut second = session_fixture("second-shell", SessionKind::Shell);
                second.project_id = "b".into();
                app.state.sessions = vec![first, second];
                for project in ["a", "b"] {
                    app.preferences.expanded.insert(project.into(), true);
                }
                for selected in ["a", "b"] {
                    app.selected = Some(selected.into());
                    // Include the first frame and settled layout frames.
                    for _ in 0..3 {
                        let mut expected_indent = 0.0;
                        let mut output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 800.0),
                                )),
                                ..Default::default()
                            },
                            |ui| {
                                expected_indent = ui.spacing().indent;
                                app.projects(ui);
                            },
                        );
                        output.textures_delta.clear();
                        let target = |name: &str| {
                            ctx.data(|data| {
                                data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name)))
                            })
                            .unwrap()
                        };
                        for (project, session) in [("a", "first-shell"), ("b", "second-shell")] {
                            let parent = target(&format!("project-row:{project}"));
                            let child = target(&format!("session-row:{session}"));
                            let indent = child.left() - parent.left();
                            assert!(
                                indent >= expected_indent - 0.1,
                                "width={width}, scale={scale}, project={project}: indent={indent}"
                            );
                            assert!(child.top() >= parent.bottom());
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn project_list_excludes_editors_and_global_history_includes_other_projects() {
        let (mut app, ctx, _dir) = fixture();
        let mut ended = session_fixture("ended-other", SessionKind::Shell);
        ended.project_id = "b".into();
        ended.lifecycle = Lifecycle::Ended;
        app.state.sessions = vec![
            session_fixture("live-shell", SessionKind::Shell),
            session_fixture("open-file", SessionKind::Editor),
            ended,
        ];
        app.preferences.expanded.insert("a".into(), true);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.projects(ui));
        output.textures_delta.clear();
        let target = |name: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
        };
        assert!(target("session-row:live-shell").is_some());
        assert!(target("session-row:open-file").is_none());
        assert!(target("session-row:ended-other").is_none());
        app.preferences.tool = SidebarTool::History;
        app.preferences.all_projects = false;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.sidebar(ui));
        output.textures_delta.clear();
        assert!(target("session-row:ended-other").is_some());
        assert_eq!(app.state.sessions.len(), 3);
    }

    #[test]
    fn file_only_views_are_distinguished_from_shell_and_mixed_tabs() {
        let (mut app, _, _dir) = fixture();
        app.state.sessions.extend([
            session_fixture("file", SessionKind::Editor),
            session_fixture("shell", SessionKind::Shell),
        ]);
        assert!(app.editors_only(&["file".into()]));
        assert!(!app.editors_only(&["shell".into()]));
        assert!(!app.editors_only(&["file".into(), "shell".into()]));
        assert!(!app.editors_only(&[]));
    }

    #[test]
    fn extra_close_while_prompting_does_not_queue_another_check() {
        let (mut app, _, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.upsert_unsaved_close(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            "Unsaved changes".into(),
        );
        assert!(app.skip_editor_close_request(&["file".into()]));
        assert!(!app.skip_editor_close_request(&["other".into()]));
        app.close_editors(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            editor_close::Mode::Discard,
        );
        assert!(matches!(
            requests.try_recv().unwrap(),
            Job::CloseEditors(_, _, editor_close::Mode::Discard)
        ));
        app.close_editors(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            editor_close::Mode::Discard,
        );
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn failed_close_updates_the_same_prompt() {
        let (mut app, _, _dir) = fixture();
        app.upsert_unsaved_close(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            "Unsaved changes".into(),
        );
        app.editors_closed(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            Err("Editor did not close. Check for unsaved buffers or running editor jobs.".into()),
        );
        assert_eq!(app.editor_close_prompts.len(), 1);
        assert!(
            app.editor_close_prompts[0]
                .2
                .contains("Editor did not close")
        );
        app.editors_closed(
            editor_close::Target::Pane("other".into()),
            vec!["other".into()],
            Err("Unsaved changes".into()),
        );
        assert_eq!(app.editor_close_prompts.len(), 2);
        assert!(app.unsaved_close_prompt("file").is_some());
        assert!(app.unsaved_close_prompt("other").is_some());
    }
    #[test]
    fn inline_rename_saves_with_enter_and_cancels_with_escape() {
        for surface in [
            RenameSurface::Workspace,
            RenameSurface::Pane,
            RenameSurface::Sidebar,
        ] {
            for save in [false, true] {
                let (mut app, ctx, _dir) = fixture();
                app.state
                    .sessions
                    .push(session_fixture("named", SessionKind::Shell));
                let (jobs, requests) = mpsc::channel();
                app.jobs = jobs.into();
                app.begin_rename("named", surface);
                let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(220.0, 24.0));
                let mut frame = |events| {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(260.0, 80.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| app.inline_rename(ui, "named", surface, rect),
                    );
                    output.textures_delta.clear();
                };
                frame(vec![]);
                frame(vec![
                    egui::Event::Text("New title".into()),
                    egui::Event::Key {
                        key: if save {
                            egui::Key::Enter
                        } else {
                            egui::Key::Escape
                        },
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                ]);
                assert!(app.rename_session.is_none());
                assert!(ctx.input(|input| {
                    !input.events.iter().any(|event| {
                        matches!(event, egui::Event::Text(_) | egui::Event::Key { .. })
                    })
                }));
                if save {
                    let Ok(Job::Control(request, _)) = requests.try_recv() else {
                        panic!("Inline rename should save")
                    };
                    assert!(
                        matches!(*request,Request::Rename {ref session,ref label} if session=="named"&&label=="New title")
                    );
                } else {
                    assert!(requests.try_recv().is_err());
                }
            }
        }
    }
    #[test]
    fn closing_editor_tab_restores_the_other_tabs_latest_pane_focus() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions.extend([
            session_fixture("shell", SessionKind::Shell),
            session_fixture("other", SessionKind::Shell),
        ]);
        app.insert("a", Tab::Terminal("shell".into()), None);
        let original = app.layouts["a"].active.clone();
        app.update_tx
            .send(Update::WorkspaceCreated(
                session_fixture("editor", SessionKind::Editor),
                "file".into(),
                vec![Tab::Terminal("shell".into())],
            ))
            .unwrap();
        app.process_updates(&ctx);
        app.layouts.get_mut("a").unwrap().active = original.clone();
        app.insert("a", Tab::Terminal("other".into()), Some("right"));
        app.layouts.get_mut("a").unwrap().active = "file".into();
        app.active_session = Some("editor".into());
        let mut state = app.state.clone();
        state
            .sessions
            .iter_mut()
            .find(|s| s.id == "editor")
            .unwrap()
            .lifecycle = Lifecycle::Ended;
        app.apply_state(state);
        assert_eq!(app.layouts["a"].active, original);
        assert_eq!(app.active_session.as_deref(), Some("other"));
    }
    #[test]
    fn legacy_daemon_diff_never_sends_an_unsupported_creation_request() {
        let (mut app, _, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.selected = Some("a".into());
        app.error = Some("failed to fill whole buffer".into());
        for staged in [false, true] {
            app.add_diff("/a".into(), "/a/file.rs".into(), staged);
            let Job::Diff(Tab::Diff { staged: actual, .. }) = requests.recv().unwrap() else {
                panic!("Legacy daemon must use the local diff renderer");
            };
            assert_eq!(actual, staged);
        }
        assert!(requests.try_recv().is_err());
        assert!(app.error.is_none());
        assert!(app.info.as_ref().unwrap().contains("updated daemon"));
        assert!(app.state.sessions.is_empty());
        assert_eq!(app.layouts["a"].tabs.len(), 2);
    }
    #[test]
    fn git_reviews_open_distinct_top_level_tabs_in_the_origin_project() {
        let (mut app, ctx, _dir) = fixture();
        app.state.capabilities.push(NVIM_REVIEW_CAPABILITY.into());
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.selected = Some("a".into());
        app.add_diff("/a".into(), "/a/file.rs".into(), false);
        app.add_diff("/a".into(), "/a/file.rs".into(), true);
        let mut ids = Vec::new();
        for staged in [false, true] {
            let Job::Control(request, After::Workspace(id, anchors)) = requests.recv().unwrap()
            else {
                panic!("Expected review workspace")
            };
            assert!(
                matches!(*request, Request::CreateReview { ref project, staged: actual, .. } if project == "a" && actual == staged)
            );
            ids.push(id.clone());
            app.selected = Some("b".into());
            let mut session = session_fixture(
                if staged { "staged" } else { "working" },
                SessionKind::Editor,
            );
            session.review = true;
            app.update_tx
                .send(Update::WorkspaceCreated(session, id, anchors))
                .unwrap();
            app.process_updates(&ctx);
            assert_eq!(app.selected.as_deref(), Some("b"));
        }
        assert_ne!(ids[0], ids[1]);
        assert!(app.layouts["a"].contains(&Tab::Terminal("working".into())));
        assert!(app.layouts["a"].contains(&Tab::Terminal("staged".into())));
    }
    #[test]
    fn invalid_saved_focus_reports_error_and_preserves_layout() {
        let (mut app, _, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        let workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        let mut saved = sanitize_layout(serde_json::to_value(workspace).unwrap());
        saved["tabs"][0]["layout"]["surfaces"][0]["Main"]["focused_node"] = serde_json::json!(999);
        let mut state = app.state.clone();
        state.projects[0].layout = saved.clone();
        app.layouts.clear();
        app.apply_state(state);
        // Loading persisted input must not allow an invalid index to reach the GUI.
        app.layouts["a"].active_pane();
        assert!(app.error.as_ref().is_some_and(|e| e.contains("focus")));
        assert!(app.layout_readonly.contains("a"));
        app.save_layouts();
        assert_eq!(app.state.projects[0].layout, saved);
        assert!(
            !requests
                .try_iter()
                .any(|job| matches!(job, Job::Control(request, _)
            if matches!(*request, Request::SaveLayout { ref project, .. } if project == "a")))
        );
    }
    #[test]
    fn unknown_workspace_format_is_not_overwritten() {
        let (mut app, _, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.layouts.clear();
        let mut state = app.state.clone();
        state.projects[0].layout = serde_json::json!({"version":99});
        app.apply_state(state);
        app.save_layouts();
        assert!(app.layout_readonly.contains("a"));
        assert!(!requests.try_iter().any(|job|matches!(job,Job::Control(request,_) if matches!(*request,Request::SaveLayout {ref project,..} if project=="a"))));
    }
    #[test]
    fn sidebar_navigation_selects_owning_top_level_tab() {
        let (mut app, _, _dir) = fixture();
        app.state.sessions.extend([
            session_fixture("one", SessionKind::Shell),
            session_fixture("two", SessionKind::Shell),
        ]);
        app.insert("a", Tab::Terminal("one".into()), None);
        let first = app.layouts["a"].active.clone();
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("second".into(), Tab::Terminal("two".into()));
        app.go_session("one");
        assert_eq!(app.layouts["a"].active, first);
        assert_eq!(app.active_session.as_deref(), Some("one"));
        app.go_session("two");
        assert_eq!(app.layouts["a"].active, "second");
        assert_eq!(app.layouts["a"].tabs.len(), 2);
    }
    #[test]
    fn delayed_split_stays_in_origin_tab_without_stealing_tab_selection() {
        let (mut app, ctx, _dir) = fixture();
        app.insert("a", Tab::Terminal("one".into()), None);
        let first = app.layouts["a"].active.clone();
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("second".into(), Tab::Terminal("two".into()));
        app.active_session = Some("two".into());
        app.update_tx
            .send(Update::Created(
                session_fixture("split", SessionKind::Shell),
                Some("right".into()),
                Some(vec![Tab::Terminal("one".into())]),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.layouts["a"].active, "second");
        assert_eq!(app.active_session.as_deref(), Some("two"));
        assert_eq!(
            app.layouts["a"]
                .tabs
                .iter()
                .find(|tab| tab.id == first)
                .unwrap()
                .layout
                .iter_all_tabs()
                .count(),
            2
        );
    }
    #[test]
    fn closing_either_split_direction_expands_the_remaining_pane() {
        for direction in ["left", "right", "up", "down"] {
            let (mut app, _, _dir) = fixture();
            app.state.sessions.extend([
                session_fixture("remaining", SessionKind::Shell),
                session_fixture("closed", SessionKind::Shell),
            ]);
            app.insert("a", Tab::Terminal("remaining".into()), None);
            app.insert("a", Tab::Terminal("closed".into()), Some(direction));
            app.active_session = Some("closed".into());
            let mut next = app.state.clone();
            next.sessions
                .iter_mut()
                .find(|s| s.id == "closed")
                .unwrap()
                .lifecycle = Lifecycle::Ended;
            app.apply_state(next);
            let leaf = app.layouts["a"].main_surface()[NodeIndex::root()]
                .get_leaf()
                .expect("Remaining pane should replace the split root");
            assert_eq!(leaf.tabs, vec![Tab::Terminal("remaining".into())]);
            assert_eq!(app.active_session.as_deref(), Some("remaining"));
            assert!(
                app.state
                    .sessions
                    .iter()
                    .any(|s| s.id == "closed" && s.lifecycle == Lifecycle::Ended)
            );
        }
    }
    #[test]
    fn restart_cleans_ended_panes_but_history_can_be_reopened() {
        let (mut app, _, _dir) = fixture();
        let mut dock = DockState::new(vec![Tab::Terminal("remaining".into())]);
        dock.main_surface_mut().split_below(
            NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("ended".into())],
        );
        let mut state = app.state.clone();
        state.projects[0].layout = sanitize_layout(serde_json::to_value(&dock).unwrap());
        let mut ended = session_fixture("ended", SessionKind::Shell);
        ended.lifecycle = Lifecycle::Ended;
        state.sessions = vec![session_fixture("remaining", SessionKind::Shell), ended];
        app.layouts.clear();
        app.state_loaded = false;
        app.apply_state(state.clone());
        assert!(app.layouts["a"].main_surface()[NodeIndex::root()].is_leaf());
        app.go_session("ended");
        app.apply_state(state);
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("ended".into()))
                .is_some()
        );
    }
    #[test]
    fn editor_open_preserves_shell_tabs_and_quit_restores_original_focus() {
        for lower in [false, true] {
            let (mut app, ctx, _dir) = fixture();
            let shell = session_fixture("shell", SessionKind::Shell);
            app.state.sessions.push(shell.clone());
            app.insert("a", Tab::Terminal(shell.id.clone()), None);
            if lower {
                let lower = session_fixture("lower", SessionKind::Shell);
                app.state.sessions.push(lower);
                app.insert("a", Tab::Terminal("lower".into()), Some("down"));
            }
            let original = if lower { "lower" } else { "shell" };
            app.active_session = Some(original.into());
            let After::Workspace(workspace_id, anchors) = app.editor_target("a", None, None) else {
                panic!("Expected anchored editor creation")
            };

            let editor = session_fixture("editor", SessionKind::Editor);
            app.update_tx
                .send(Update::WorkspaceCreated(
                    editor.clone(),
                    workspace_id,
                    anchors,
                ))
                .unwrap();
            app.process_updates(&ctx);
            assert!(app.layouts["a"].contains(&Tab::Terminal(original.into())));
            assert_eq!(app.layouts["a"].tabs.len(), 2);
            assert_eq!(app.layouts["a"].iter_all_tabs().count(), 1);
            let mut state = app.state.clone();
            state
                .sessions
                .iter_mut()
                .find(|s| s.id == editor.id)
                .unwrap()
                .lifecycle = Lifecycle::Ended;
            app.apply_state(state);
            assert!(
                app.layouts["a"]
                    .find_tab(&Tab::Terminal(editor.id))
                    .is_none()
            );
            assert!(
                app.layouts["a"]
                    .find_tab(&Tab::Terminal(original.into()))
                    .is_some()
            );
            assert_eq!(app.active_session.as_deref(), Some(original));
            assert!(
                app.state
                    .sessions
                    .iter()
                    .find(|s| s.id == original)
                    .unwrap()
                    .lifecycle
                    .live()
            );
        }
    }
    #[test]
    fn editor_exit_does_not_change_another_projects_focus() {
        let (mut app, ctx, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.insert("a", Tab::Terminal("shell".into()), None);
        app.update_tx
            .send(Update::Created(
                session_fixture("editor", SessionKind::Editor),
                Some("right".into()),
                Some(vec![Tab::Terminal("shell".into())]),
            ))
            .unwrap();
        app.process_updates(&ctx);
        app.select_project("b".into());
        app.insert("b", Tab::Terminal("other".into()), None);
        app.active_session = Some("other".into());
        let mut state = app.state.clone();
        state
            .sessions
            .iter_mut()
            .find(|s| s.id == "editor")
            .unwrap()
            .lifecycle = Lifecycle::Ended;
        app.apply_state(state);
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert_eq!(app.active_session.as_deref(), Some("other"));
    }
    #[test]
    fn rename_targets_the_requested_session() {
        let (mut app, _, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("named", SessionKind::Shell));
        app.active_session = Some("different".into());
        app.begin_rename("named", RenameSurface::Sidebar);
        assert_eq!(app.rename_session, Some(("named".into(), "named".into())));
        assert!(app.rename_focus);
    }
    #[test]
    fn explorer_single_click_opens_an_editor_tab_across_the_entire_row() {
        for x in [12.0, 55.0, 230.0] {
            let (mut app, ctx, dir) = fixture();
            let path = dir.path().join(".gitkeep");
            fs::write(&path, "").unwrap();
            app.dirs.insert(
                dir.path().into(),
                vec![services::Entry {
                    path: path.clone(),
                    directory: false,
                    ignored: false,
                }],
            );
            let (jobs, received) = mpsc::channel();
            app.jobs = jobs.into();
            let mut draw = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(260.0, 80.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| app.tree(ui, dir.path(), 0),
                );
                output.textures_delta.clear();
            };
            draw(vec![]);
            let pos = egui::pos2(x, 12.0);
            draw(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                draw(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
            let Ok(Job::Control(request, After::Workspace(_, _))) = received.try_recv() else {
                panic!("A single click at x={x} should open an editor tab");
            };
            assert!(
                matches!(*request, Request::Create { file: Some(ref file), editor: true, ref project, .. } if file == &path && project == "a")
            );
            assert!(
                received.try_recv().is_err(),
                "One click should create only one tab"
            );
        }
    }
    #[test]
    fn appearance_preview_cancel_and_external_conflict() {
        let (mut app, ctx, _dir) = fixture();
        app.settings_open = true;
        app.theme_draft.text = "#123456".into();
        app.preview_appearance(&ctx);
        assert_eq!(app.theme.text, "#123456");
        app.settings_open = false;
        app.preview_appearance(&ctx);
        assert_eq!(app.theme, app.theme_committed);
        app.settings_open = true;
        app.theme_draft.text = "#123456".into();
        let external = AppearanceConfig {
            text: "#654321".into(),
            ..Default::default()
        };
        app.update_tx
            .send(Update::Appearance(Box::new(AppearanceFile {
                config: external.clone(),
                source: "external".into(),
            })))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.theme_conflict);
        assert_eq!(app.theme_draft.text, "#123456");
        assert_eq!(app.theme_committed, external);
    }
    #[test]
    fn stale_refresh_is_ignored_even_for_same_directory() {
        let (mut app, ctx, _dir) = fixture();
        app.refresh_generation = 3;
        app.refresh_request = Some(refresh::Request {
            cwd: "/a".into(),
            generation: 3,
            directories: vec![],
        });
        app.update_tx
            .send(Update::Refresh(
                2,
                services::ContextData {
                    cwd: "/a".into(),
                    root: None,
                    git_dirs: vec![],
                    branch: "stale".into(),
                    changes: vec![],
                    decorations: Default::default(),
                    error: None,
                },
                vec![],
                false,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.context.is_none());
    }
    #[test]
    fn delayed_creation_uses_original_project_after_navigation_and_pane_removal() {
        let (mut app, ctx, _dir) = fixture();
        app.insert("a", Tab::Terminal("anchor".into()), None);
        app.select_project("b".into());
        app.remove_tab("anchor");
        let session = Session {
            review: false,
            id: "created".into(),
            project_id: "a".into(),
            label: "new".into(),
            cwd: "/a/subdir".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "fixture".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        };
        app.update_tx
            .send(Update::Created(
                session,
                None,
                Some(vec![Tab::Terminal("anchor".into())]),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("created".into()))
                .is_some()
        );
    }
    #[test]
    fn cancelled_picker_and_error_keep_workspace_and_layout() {
        let (mut app, ctx, _dir) = fixture();
        app.insert("a", Tab::Terminal("original".into()), None);
        app.update_tx.send(Update::PickedProject(None, 0)).unwrap();
        app.update_tx
            .send(Update::Error("folder unavailable".into()))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("original".into()))
                .is_some()
        );
    }
    #[test]
    fn delayed_open_refreshes_inventory_without_overriding_new_selection() {
        let (mut app, ctx, _dir) = fixture();
        app.select_project("b".into());
        app.select_project("a".into());
        app.update_tx
            .send(Update::OpenedProject(
                Box::new(app.state.clone()),
                "b".into(),
                0,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("a"));
        app.update_tx
            .send(Update::OpenedProject(
                Box::new(app.state.clone()),
                "b".into(),
                app.selection_generation,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
    }
    #[test]
    fn project_round_trip_restores_split_tabs_and_focus() {
        let (mut app, _, _dir) = fixture();
        app.insert("a", Tab::Terminal("first".into()), None);
        app.insert("a", Tab::Terminal("focused".into()), Some("right"));
        app.select_project("b".into());
        app.insert("b", Tab::Terminal("other".into()), None);
        app.select_project("a".into());
        assert_eq!(app.active_session.as_deref(), Some("focused"));
        assert_eq!(app.layouts["a"].iter_all_tabs().count(), 2);
        assert_eq!(app.layouts["b"].iter_all_tabs().count(), 1);
    }
    #[test]
    fn repeated_gui_show_focuses_the_existing_session_without_duplicate_tabs() {
        let (mut app, ctx, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.insert("a", Tab::Terminal("shell".into()), None);
        for _ in 0..2 {
            app.ui_request(
                &ctx,
                terminator_core::ui_control::Request::ShowSession {
                    session: "shell".into(),
                    anchor: None,
                    split: None,
                },
            )
            .unwrap();
        }
        assert_eq!(app.layouts["a"].tabs.len(), 1);
        assert_eq!(app.layouts["a"].iter_all_tabs().count(), 1);
    }
    #[test]
    fn image_open_creates_no_editor_and_keeps_original_project() {
        let (mut app, ctx, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.open_file("/a/image.PNG".into(), None, None, false);
        app.select_project("b".into());
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(app.layouts["a"].contains(&Tab::Image {
            path: "/a/image.PNG".into()
        }));
        assert_eq!(app.layouts["a"].version, 3);
        assert!(app.state.sessions.is_empty());
        assert!(!requests.try_iter().any(|j|matches!(j,Job::Control(request,_) if matches!(*request,Request::Create { editor:true,.. }))));
    }
    #[test]
    fn image_split_survives_layout_temporarily_owned_by_renderer() {
        let (mut app, ctx, _dir) = fixture();
        app.insert("a", Tab::Terminal("shell".into()), None);
        app.active_session = Some("shell".into());
        let mut dock = app.layouts.remove("a").unwrap();
        let path = dock
            .find_tab(&Tab::Terminal("shell".into()))
            .unwrap()
            .node_path();
        app.pane_by_tab
            .insert(Tab::Terminal("shell".into()).key(), path);
        app.pane_tabs
            .insert(path, vec![Tab::Terminal("shell".into())]);
        app.open_image("a", "/a/picture.png".into(), Some("right"));
        let original = dock.active.clone();
        dock.add("other".into(), Tab::Terminal("other".into()));
        app.layouts.insert("a".into(), dock);
        app.process_updates(&ctx);
        assert_eq!(app.layouts["a"].active, "other");
        let source = app.layouts["a"]
            .tabs
            .iter()
            .find(|t| t.id == original)
            .unwrap();
        assert!(
            source
                .layout
                .find_tab(&Tab::Image {
                    path: "/a/picture.png".into()
                })
                .is_some()
        );
    }
    #[test]
    fn attention_migration_retries_without_ack_and_preserves_later_choices() {
        let (mut app, ctx, dir) = fixture();
        app.preferences_writable = true;
        app.migrate_attention();
        assert!(app.attention_requested.is_some());
        assert!(!app.preferences.attention_migrated);
        app.attention_requested = Some(Instant::now() - Duration::from_secs(6));
        app.migrate_attention();
        assert!(app.attention_requested.unwrap().elapsed() >= Duration::from_secs(6));
        app.update_tx
            .send(Update::AttentionMigrated(Err("rejected".into())))
            .unwrap();
        app.process_updates(&ctx);
        assert!(!app.preferences.attention_migrated);
        app.attention_requested = Some(Instant::now() - Duration::from_secs(6));
        app.migrate_attention();
        assert!(app.attention_requested.unwrap().elapsed() < Duration::from_secs(1));
        app.update_tx
            .send(Update::AttentionMigrated(Ok(())))
            .unwrap();
        app.process_updates(&ctx);
        app.preferences.save(dir.path()).unwrap();
        assert!(UiPreferences::load(dir.path()).unwrap().attention_migrated);
        app.state.settings.notifications_side = false;
        app.attention_requested = None;
        app.migrate_attention();
        assert!(app.attention_requested.is_none());
        assert!(!app.state.settings.notifications_side);
    }
    #[test]
    fn opening_settings_keeps_selected_sidebar_and_custom_editor() {
        let (mut app, _, _dir) = fixture();
        app.preferences.tool = SidebarTool::Git;
        app.preferences.visible = false;
        app.state.settings.external_editor = "/custom/editor".into();
        app.state.settings.external_args = vec!["a b".into()];
        app.open_settings();
        assert_eq!(app.preferences.tool, SidebarTool::Git);
        assert!(!app.preferences.visible);
        assert_eq!(app.editor_preset, external_editor::CUSTOM);
        assert_eq!(app.settings_draft.external_args, vec!["a b"]);
    }
    #[test]
    fn failed_migration_does_not_set_marker() {
        let (mut app, ctx, _dir) = fixture();
        app.update_tx
            .send(Update::Error("Settings rejected".into()))
            .unwrap();
        app.process_updates(&ctx);
        assert!(!app.preferences.typography_migrated);
        app.update_tx.send(Update::TypographyMigrated).unwrap();
        app.process_updates(&ctx);
        assert!(app.preferences.typography_migrated);
    }
    #[test]
    fn hidden_agent_terminal_keeps_owner_and_directory_context() {
        let (mut app, _, _dir) = fixture();
        app.state.sessions.push(Session {
            review: false,
            id: "hidden".into(),
            project_id: "a".into(),
            label: "Shell".into(),
            cwd: "/b".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "same".into(),
            pid: Some(123),
            truncated: false,
            cwd_confirmed: true,
        });
        app.select_project("b".into());
        assert!(
            !app.preferences
                .includes_project("a", app.selected.as_deref())
        );
        app.preferences.all_projects = true;
        assert!(
            app.preferences
                .includes_project("a", app.selected.as_deref())
        );
        app.go_session("hidden");
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert_eq!(app.cwd(), Some(PathBuf::from("/b")));
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("hidden".into()))
                .is_some()
        );
        assert_eq!(app.state.sessions[0].pid, Some(123));
        app.terminal_context.insert("a".into(), "hidden".into());
        app.active_session = None; // diff/editor focus retains the preceding shell.
        assert_eq!(app.cwd(), Some(PathBuf::from("/b")));
        app.preferences.all_projects = false;
        assert!(
            app.preferences
                .includes_project("a", app.selected.as_deref())
        );
    }

    #[cfg(feature = "test-support")]
    fn notice_fixture(id: &str, session: &str, state: AgentState, created: u64) -> Notification {
        Notification {
            id: id.into(),
            session_id: session.into(),
            invocation_id: id.into(),
            request_id: None,
            state,
            summary: state.label().into(),
            details: "Agent: codex\nEvent: PermissionRequest\nSession: test".into(),
            created,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        }
    }

    #[cfg(feature = "test-support")]
    fn agent_target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
    }

    #[cfg(feature = "test-support")]
    fn render_agents(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.agents_view(ui),
        );
        output.textures_delta.clear();
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn inline_attention_preserves_input_but_out_of_scope_details_remain_modal() {
        let (mut app, _, _dir) = fixture();
        app.preferences.left_agents = true;
        app.preferences.all_projects = true;
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        app.detail = Some("wait".into());
        assert!(!app.notice_detail_modal_open());
        app.preferences.left_agents = false;
        app.preferences.visible = false;
        assert!(app.notice_detail_modal_open());
        app.preferences.left_agents = true;
        app.preferences.all_projects = false;
        app.selected = Some("different-project".into());
        assert!(app.notice_detail_modal_open());
        app.preferences.all_projects = true;
        app.state.notifications[0].snoozed_until = now() + 600;
        assert!(app.notice_detail_modal_open());
        app.state.notifications[0].snoozed_until = 0;
        app.state.notifications[0].dismissed = true;
        assert!(app.notice_detail_modal_open());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn attention_actions_fit_minimum_sidebar_widths() {
        for width in [170.0, 220.0, 320.0] {
            let (mut app, ctx, _dir) = fixture();
            appearance::install(&ctx);
            app.preferences.all_projects = true;
            app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
            app.state.notifications = vec![notice_fixture(
                "wait",
                "live-shell",
                AgentState::WaitingPermission,
                now(),
            )];
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let bounds = ui.max_rect();
                    app.agents_view(ui);
                    for action in ["go", "snooze", "dismiss"] {
                        let rect =
                            agent_target(&ctx, &format!("agent-{action}:live-shell")).unwrap();
                        assert!(
                            bounds.contains_rect(rect),
                            "width {width}: {action} {rect:?} outside {bounds:?}"
                        );
                    }
                },
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn resolved_waiting_notice_sorts_below_unresolved_completion() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.all_projects = true;
        app.state.sessions = vec![
            session_fixture("done", SessionKind::Shell),
            session_fixture("resolved", SessionKind::Shell),
        ];
        let mut resolved =
            notice_fixture("old-wait", "resolved", AgentState::WaitingPermission, now());
        resolved.resolved = true;
        app.state.notifications = vec![
            resolved,
            notice_fixture("done", "done", AgentState::Completed, 1),
        ];
        render_agents(&mut app, &ctx, vec![]);
        assert!(
            agent_target(&ctx, "agent-row:done").unwrap().top()
                < agent_target(&ctx, "agent-row:resolved").unwrap().top()
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agents_inbox_lists_pending_notices_not_stopped_agents() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.all_projects = true;
        app.state.sessions = vec![
            session_fixture("stopped-shell", SessionKind::Shell),
            session_fixture("live-shell", SessionKind::Shell),
        ];
        app.state.agents = vec![Agent {
            invocation_id: "stopped".into(),
            session_id: "stopped-shell".into(),
            kind: "codex".into(),
            provider_session_id: None,
            state: AgentState::Stopped,
            sequence: None,
            updated: 1,
            resume: None,
        }];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        render_agents(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "agent-row:stopped-shell").is_none());
        assert!(agent_target(&ctx, "agent-row:live-shell").is_some());
        assert!(agent_target(&ctx, "agent-go:live-shell").is_some());
        assert!(agent_target(&ctx, "agent-snooze:live-shell").is_some());
        assert!(agent_target(&ctx, "agent-dismiss:live-shell").is_some());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agents_inbox_puts_waiting_above_completed() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.all_projects = true;
        app.state.sessions = vec![
            session_fixture("done-shell", SessionKind::Shell),
            session_fixture("live-shell", SessionKind::Shell),
        ];
        app.state.notifications = vec![
            notice_fixture("done", "done-shell", AgentState::Completed, now()),
            notice_fixture("wait", "live-shell", AgentState::WaitingPermission, 1),
        ];
        render_agents(&mut app, &ctx, vec![]);
        let waiting = agent_target(&ctx, "agent-row:live-shell").unwrap();
        let completed = agent_target(&ctx, "agent-row:done-shell").unwrap();
        assert!(waiting.top() < completed.top());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn agents_inbox_go_focuses_session_and_dismiss_hides_card() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.all_projects = true;
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.apply_notice_action("wait".into(), AttentionAction::Go);
        assert_eq!(app.active_session.as_deref(), Some("live-shell"));
        app.apply_notice_action("wait".into(), AttentionAction::Dismiss);
        assert!(
            app.state
                .notifications
                .iter()
                .all(|notice| notice.dismissed)
        );
        render_agents(&mut app, &ctx, vec![]);
        assert!(agent_target(&ctx, "agent-row:live-shell").is_none());
        let mut saw_dismiss = false;
        while let Ok(job) = received.try_recv() {
            if let Job::Control(request, _) = job
                && matches!(
                    *request,
                    Request::Notice {
                        ref action,
                        ..
                    } if action == "dismiss"
                )
            {
                saw_dismiss = true;
            }
        }
        assert!(saw_dismiss);
    }
}
