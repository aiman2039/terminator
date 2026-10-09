use super::super::appearance;
use eframe::egui::{self};
use egui_term::PtyEvent;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn process_updates(&mut self, ctx: &egui::Context) {
        let processing_started = Instant::now();
        let mut budget = native_jobs::ResultBudget::new();
        loop {
            if !budget.next() {
                ctx.request_repaint();
                break;
            }
            if self.service_ready.is_empty() {
                self.service_completion.take();
                if let Some(mut completion) = self.service_owner.supervisor.try_recv() {
                    let Some(result) = completion.result.take() else {
                        continue;
                    };
                    match result {
                        Ok(updates) => self.service_ready.extend(updates),
                        Err(async_service::Failure::Cancelled) => {
                            let key = &completion.context.resource;
                            if matches!(completion.context.subsystem, "diff" | "files") {
                                self.loading.remove(key);
                            }
                            if completion.context.subsystem == "images"
                                && let Some(preview) = self.images.values_mut().find(|preview| {
                                    preview.generation == completion.context.generation
                                })
                            {
                                preview.loading = false;
                                preview.cancellation = None;
                            }
                        }
                        Err(error) => {
                            if let Some(update) =
                                service_failure_update(&completion.context, &error)
                            {
                                if let Update::ResolvedTarget(key, _) = &update {
                                    self.loading.remove(key);
                                }
                                self.service_ready.push_back(update);
                            }
                        }
                    }
                    self.service_completion = Some(completion);
                    ctx.request_repaint();
                }
            }
            let update = if let Some(state) = self
                .service_owner
                .snapshot
                .try_lock()
                .ok()
                .and_then(|mut state| state.take())
            {
                Update::State(state)
            } else if let Some(update) = self.service_ready.pop_front() {
                update
            } else if let Ok(update) = self.service_owner.events.try_recv() {
                update
            } else if let Ok(update) = self.updates.try_recv() {
                update
            } else {
                break;
            };
            match update {
                Update::LayoutsPrepared(generation, layouts) => {
                    if generation == self.layout_generation && !self.exit.active() {
                        for (project, value, text) in layouts {
                            if self.can_persist_layout(&project)
                                && self.layout_saved.get(&project) != Some(&text)
                                && self.layout_pending.get(&project) != Some(&text)
                                && self
                                    .jobs
                                    .send(Job::SaveLayout(project.clone(), value, text.clone()))
                                    .is_ok()
                            {
                                self.layout_pending.insert(project, text);
                            }
                        }
                    }
                }
                Update::LayoutSaved(project, text, result) => {
                    if self.layout_pending.get(&project) == Some(&text) {
                        self.layout_pending.remove(&project);
                    }
                    match result {
                        Ok(()) => {
                            self.layout_saved.insert(project, text);
                        }
                        Err(error) => {
                            // Force the next pass to resend so a transient
                            // failure is retried instead of being treated as
                            // "already persisted" by the unchanged signature.
                            self.layout_signature = None;
                            self.layout_save_failed(error);
                        }
                    }
                }

                Update::RadioCatalog(catalog) => self.player.radio_base = catalog,
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
                            self.error = Some(format!("Could not repair installation: {error}"));
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
                    match *result {
                        Ok(prefs) => self.preferences_saved = prefs,
                        Err(error) => {
                            if self.exit.active() {
                                self.cancel_exit(error);
                            } else {
                                self.report_status_error(error);
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
                #[cfg(test)]
                Update::UiRequest(request, reply, deadline) => {
                    let result = if Instant::now() >= deadline {
                        Err("GUI request expired before processing; no action was performed".into())
                    } else {
                        self.ui_request(ctx, request).map_err(|e| format!("{e:#}"))
                    };
                    let _ = reply.send(result);
                }
                Update::AsyncUiRequest(request, reply, deadline) => {
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
                Update::Resources(sample) => self.resources = Some(sample),
                Update::OpenImage(project, path, after) => {
                    self.place_gui_tab(project, Tab::Image { path }, after);
                }
                Update::OpenNativeEditor(project, path, after) => {
                    let existing = self.layouts.get(&project).and_then(|workspace| {
                        workspace
                            .tabs
                            .iter()
                            .flat_map(|group| group.layout.iter_all_tabs())
                            .map(|(_, tab)| tab)
                            .find(|tab| matches!(tab, Tab::NativeEditor { path: current } if *current == path))
                            .cloned()
                    });
                    self.place_gui_tab(
                        project,
                        existing.unwrap_or(Tab::NativeEditor { path }),
                        after,
                    );
                }
                Update::OpenBrowser(project, target, after) => {
                    let existing = self.layouts.get(&project).and_then(|workspace| {
                        workspace.tabs.iter().flat_map(|group| group.layout.iter_all_tabs())
                            .map(|(_, tab)| tab)
                            .find(|tab| matches!(tab, Tab::Browser { target: current, .. } if *current == target))
                            .cloned()
                    });
                    self.place_gui_tab(
                        project,
                        existing.unwrap_or_else(|| Tab::Browser { id: id(), target }),
                        after,
                    );
                }
                Update::Image(path, generation, result) => {
                    let used: usize = self
                        .images
                        .values()
                        .filter_map(|p| p.texture.as_ref())
                        .map(|t| t.size()[0].saturating_mul(t.size()[1]).saturating_mul(4))
                        .sum();
                    if let Some(preview) = self
                        .images
                        .get_mut(&path)
                        .filter(|p| p.generation == generation)
                    {
                        preview.loading = false;
                        let previous_bytes = preview.texture.as_ref().map_or(0, |t| {
                            t.size()[0].saturating_mul(t.size()[1]).saturating_mul(4)
                        });
                        match result {
                            Ok(image)
                                if used
                                    .saturating_sub(previous_bytes)
                                    .saturating_add(image.pixels.len().saturating_mul(4))
                                    <= 128 * 1024 * 1024 =>
                            {
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
                                );
                            }
                            Err(error) => preview.error = Some(error),
                        }
                    }
                }
                Update::TestPickerClosed => self.picker_active = false,
                Update::HookStatus(status) => self.hook_status = status,
                Update::Appearance(file) => {
                    let dirty = self.settings_session && self.theme_draft != self.theme_committed;
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
                            self.error = Some(format!("Attention settings migration: {error}"));
                        }
                    }
                }
                Update::TypographyMigrated => {
                    self.preferences.typography_migrated = true;
                }
                Update::IdleClosed(target, ids, result) => self.idle_closed(target, ids, result),
                Update::OpenedProject(state, project, generation) => {
                    self.missing_projects.remove(&project);
                    self.preferences.setup_completed = true;
                    self.refresh_request = None;
                    self.apply_state(*state);
                    if generation == self.selection_generation {
                        self.reveal_project(project);
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
                Update::PickedPath { path, target } => {
                    self.picker_active = false;
                    if let Some(path) = path {
                        let text = path.display().to_string();
                        match target {
                            BrowseTarget::Shell => self.settings_draft.shell = text,
                            BrowseTarget::Editor => self.settings_draft.editor_program = text,
                            BrowseTarget::External => {
                                self.settings_draft.external_editor = text;
                                self.editor_preset = external_editor::CUSTOM;
                            }
                            BrowseTarget::WorktreeDest => {
                                if let Some(draft) = &mut self.worktree_draft {
                                    let leaf = draft
                                        .dest
                                        .file_name()
                                        .map(PathBuf::from)
                                        .unwrap_or_else(|| PathBuf::from("terminator-task"));
                                    draft.dest = path.join(leaf);
                                }
                            }
                        }
                    }
                }
                Update::PickedAudio(paths) => {
                    self.picker_active = false;
                    self.add_audio_files(paths);
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
                        } else if crate::browser::supported_file(&path)
                            && let Some(project) = &project
                        {
                            self.open_html(project, path, None);
                        } else if player::supported(&path)
                            && let Some(project) = &project
                        {
                            self.open_audio(project, path, None);
                        } else if self.state.settings.editor_mode == EditorMode::External {
                            let _ = self.jobs.send(Job::External(path));
                        } else if self.state.settings.editor_mode == EditorMode::Native
                            && let Some(project) = &project
                        {
                            self.open_native(project, path, None, None);
                        } else if let Some(project) = project {
                            self.hide_center_overlay();
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
                Update::ProjectDirectories(directories) => {
                    self.apply_project_directories(directories)
                }
                Update::WorkspaceCreated(session, id, anchors) => {
                    let project = session.project_id.clone();
                    if self.selected.as_ref() == Some(&project) {
                        self.finish_rename(true);
                    }
                    if session.kind == SessionKind::Editor {
                        self.editor_origins
                            .insert(session.id.clone(), anchors.clone());
                    }
                    let index = self.workspace_insert.remove(&id).unwrap_or(usize::MAX);
                    self.layouts
                        .entry(project.clone())
                        .or_insert_with(Workspace::empty)
                        .add_at_from(index, id, Tab::Terminal(session.id.clone()), &anchors);
                    if self.selected.as_ref() == Some(&project) {
                        self.active_session = Some(session.id.clone());
                    }
                    if !self.state.sessions.iter().any(|s| s.id == session.id) {
                        self.state.sessions.push(session);
                        self.reconcile_presentations();
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
                        self.reconcile_presentations();
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
                Update::StripCreated(session, split, anchors) => {
                    if !self.state.sessions.iter().any(|s| s.id == session.id) {
                        self.state.sessions.push(session.clone());
                        self.reconcile_presentations();
                    }
                    if let Some(dock) = self
                        .preferences
                        .ide_strip_docks
                        .0
                        .get_mut(&session.project_id)
                        && let Some(path) = anchors.iter().find_map(|tab| dock.find_tab(tab))
                    {
                        dock.set_focused_node_and_surface(path.node_path());
                    }
                    self.insert_strip(
                        &session.project_id,
                        Tab::Terminal(session.id.clone()),
                        split.as_deref(),
                    );
                    if self.selected.as_ref() == Some(&session.project_id)
                        && self.preferences.ide_mode
                        && !self.preferences.ide_terminal_collapsed
                    {
                        self.active_session = Some(session.id.clone());
                    }
                }
                Update::FloatCreated(session, viewport, split, anchors) => {
                    if session.kind == SessionKind::Editor {
                        self.editor_origins
                            .insert(session.id.clone(), anchors.clone());
                    }
                    if !self.state.sessions.iter().any(|s| s.id == session.id) {
                        self.state.sessions.push(session.clone());
                        self.reconcile_presentations();
                    }
                    // The originating window wins even after focus or
                    // project changes. A window closed mid-flight falls
                    // back to the project dock so the tab is not lost.
                    if self.insert_float(
                        viewport,
                        Tab::Terminal(session.id.clone()),
                        split.as_deref(),
                        &anchors,
                    ) {
                        self.active_session = Some(session.id.clone());
                    } else {
                        self.insert(
                            &session.project_id,
                            Tab::Terminal(session.id.clone()),
                            split.as_deref(),
                        );
                    }
                }
                Update::Text(key, text) => {
                    self.loading.remove(&key);
                    self.texts.insert(key, text);
                }
                Update::Diff(key, result) => {
                    self.loading.remove(&key);
                    match result {
                        Ok(document) => {
                            self.diffs.insert(key, Ok(std::sync::Arc::new(document)));
                        }
                        Err(error) if self.diffs.get(&key).is_some_and(Result::is_ok) => {
                            self.error = Some(format!(
                                "Diff refresh failed; showing the previous view: {error}"
                            ));
                        }
                        Err(error) => {
                            self.diffs.insert(key, Err(error));
                        }
                    }
                }
                Update::Refresh(generation, context, directories, fallback) => {
                    if generation == self.refresh_generation
                        && self
                            .refresh_request
                            .as_ref()
                            .is_some_and(|r| r.cwd == context.cwd)
                    {
                        let context = match self.context.take() {
                            Some(mut previous)
                                if previous.cwd == context.cwd
                                    && previous.root.is_some()
                                    && (context.root.is_none() || context.error.is_some()) =>
                            {
                                previous.error = context.error.or_else(|| Some("Repository unavailable; showing the last successful status".into()));
                                previous
                            }
                            _ => context,
                        };
                        self.sidebar_cache.get_mut().clear_files();
                        self.context = Some(context);
                        self.watch_fallback = fallback;
                        for (path, entries) in directories {
                            match entries {
                                Ok(entries) => {
                                    #[cfg(feature = "test-support")]
                                    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                                        eprintln!(
                                            "Directory refresh succeeded: entries={}",
                                            entries.len()
                                        );
                                    }
                                    self.directory_errors.remove(&path);
                                    self.dirs.insert(path, entries);
                                }
                                Err(error) => {
                                    #[cfg(feature = "test-support")]
                                    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                                        eprintln!(
                                            "Directory refresh failed: kind={:?} cached_entries={}",
                                            error.kind,
                                            self.dirs.get(&path).map_or(0, Vec::len)
                                        );
                                    }
                                    self.directory_errors.insert(path, error);
                                }
                            }
                        }
                    }
                }
                Update::ClipboardPaste(session, text) => {
                    // Resolve by captured identity, never by current focus.
                    if let Some(backend) = self.backends.get_mut(&session) {
                        let mode = backend.last_content().terminal_mode;
                        let bytes = egui_term::paste_input(&text, mode);
                        backend.process_command(egui_term::BackendCommand::Write(bytes));
                    }
                }
                Update::WorktreeCreated(state, project, open_terminal) => {
                    self.missing_projects.remove(&project);
                    self.apply_state(*state);
                    self.reveal_project(project);
                    if open_terminal {
                        self.create(None);
                    }
                }
                Update::RestartFinished(message) => {
                    self.restart_pending = false;
                    self.error = Some(if message.is_empty() {
                        "Session service restart did not complete. See restart.log in the data directory for details, then retry.".into()
                    } else {
                        format!("Session service restart failed: {message}")
                    });
                }
                Update::ServiceStarted(result) => {
                    self.service_start_pending = false;
                    match result {
                        Ok(()) => {
                            self.info = Some("Session service started. Reconnecting…".into());
                        }
                        Err(error) => {
                            self.error = Some(format!("Could not start session service: {error}"));
                        }
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
                    self.report_status_error(e);
                }
                Update::Info(i) => self.info = Some(i),
                Update::Workspace(op, result) => match result {
                    Ok(workspace_ops::Report::Message(text)) => {
                        self.info = Some(text);
                        self.refresh_request = None;
                        if matches!(
                            op,
                            workspace_ops::Op::Commit(_) | workspace_ops::Op::Switch(_)
                        ) {
                            self.git_list_root = None;
                        }
                    }
                    Ok(workspace_ops::Report::Branches(names)) => self.git_branches = names,
                    Ok(workspace_ops::Report::Log(rows)) => self.git_log = rows,
                    Ok(workspace_ops::Report::Compare(data)) => {
                        self.git_compare_root.clone_from(&data.root);
                        self.git_compare = Some(data);
                    }
                    Err(error) => self.report_status_error(error),
                },
                Update::Search(id, result) => {
                    if id == self.explorer_search_generation {
                        self.explorer_search_pending = false;
                        match result {
                            Ok(hits) => {
                                self.explorer_search = hits;
                                self.explorer_search_error = None;
                            }
                            Err(error) => {
                                self.explorer_search.clear();
                                self.explorer_search_error = Some(error);
                            }
                        }
                    }
                }
            }
        }
        let mut budget = native_jobs::ResultBudget::new();
        loop {
            if !budget.next() {
                ctx.request_repaint();
                break;
            }
            let Ok((id, event)) = self.pty_rx.try_recv() else {
                break;
            };
            if let PtyEvent::ClipboardStore(_, ref text) = event {
                ctx.copy_text(text.clone());
            }
            if let PtyEvent::ChildExit(status) = &event
                && !status.success()
            {
                self.attach_failed.insert(id);
            }
            let failed = matches!(event, PtyEvent::Exit) && self.attach_failed.remove(&id);
            if let PtyEvent::Exit = event
                && let Some(session) = self.backend_ids.remove(&id)
                && self.backends.get(&session).is_some_and(|b| b.id() == id)
            {
                self.backends.remove(&session);
                let started = self.attach_started.remove(&session);
                if retry_budget::attach_exit_is_failure(started.map(|at| at.elapsed()), failed) {
                    let cwd = self
                        .state
                        .sessions
                        .iter()
                        .find(|candidate| candidate.id == session)
                        .map(|candidate| candidate.cwd.clone())
                        .unwrap_or_default();
                    self.attach_budget
                        .entry(session.clone())
                        .or_default()
                        .record(&cwd, 0, true);
                    self.attach_error.entry(session).or_insert_with(|| {
                        "Stopped retrying this terminal after repeated attachment failures. Check the session service, then retry.".into()
                    });
                } else {
                    self.attach_budget.remove(&session);
                    self.attach_error.remove(&session);
                }
            }
        }
        self.ui_service_peak_ms = self
            .ui_service_peak_ms
            .max(processing_started.elapsed().as_secs_f64() * 1000.0);
    }

    pub(crate) fn migrate_attention(&mut self) {
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
}
