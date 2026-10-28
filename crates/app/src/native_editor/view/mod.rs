//! The native editor pane, rendered once per docked or floating view.
mod support;

use support::{NativeClosePromptBarInput, NativeMarkdownPreviewInput};

use super::*;

impl App {
    /// Render the native editor pane for an open file. `project`/`tab`/
    /// `node` locate the docked view under render, if any: close requests
    /// remember their issuing view so plain closes take only that copy.
    pub(crate) fn native_editor_view(&mut self, input: NativeEditorViewInput<'_>) {
        let NativeEditorViewInput {
            ui,
            path,
            project,
            tab,
            node,
        } = input;
        let vim = self.state.settings.native_vim;
        let failed_color = appearance::color(&self.theme.status_failed);
        let services = self.services.clone();
        self.native_view_load(path, vim, &services);

        if self.native_close_after_save_settled(path) {
            return;
        }

        if self.native_close_prompt_bar(NativeClosePromptBarInput {
            ui,
            path,
            project,
            tab,
            node,
        }) {
            return;
        }

        let remote_source = self.remote_file_source(path);
        let file_location = self
            .remote_file_label(path)
            .unwrap_or_else(|| path.display().to_string());
        if let Some((remote, relative)) = &remote_source {
            ui.horizontal(|ui| {
                ui.weak("Remote snapshot · read-only");
                if ui.button("Open in SSH editor").clicked() {
                    self.open_remote_file(OpenRemoteFileInput {
                        project: remote,
                        path: relative.clone(),
                        line: None,
                        split: None,
                        text: true,
                    });
                }
            });
        }
        let project_root = self.selected_project().map(|project| project.path.clone());
        let breadcrumb = if remote_source.is_some() {
            file_location.clone()
        } else {
            self.selected_project()
                .and_then(|project| path.strip_prefix(&project.path).ok())
                .unwrap_or(path)
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" › ")
        };
        let pinned_files = self.preferences.pinned_native_files.clone();
        let mut recent_files = self.preferences.recent_native_files.clone();
        if let Some(root) = project_root.as_ref() {
            recent_files.retain(|item| item.starts_with(root) && item != path);
        } else {
            recent_files.clear();
        }
        recent_files.sort_by_key(|item| !pinned_files.contains(item));
        let pinned = pinned_files.contains(path);
        // The breadcrumb needs the outline whether or not the Structure
        // panel is open; the request is revision-gated, so this is cheap.
        let outline = self.document_outline(path);
        // Deferred closes run after the borrow below ends.
        let mut defer_close_after_save = false;
        // The view issuing `:wq`: the save settles frames later, when a
        // different view of the file may render first.
        let mut wq_issuer = None;
        let mut quit_all: Option<bool> = None;
        // Same for splits/tabs: the workspace is checked out while this
        // renders, so layout changes wait for the drain below. Hover/goto
        // answers are async too: effects only queue requests, handled
        // after the drain.
        let mut splits: Vec<NativeSplit> = Vec::new();
        let mut hover_at: Option<(native_lsp::Position, (usize, usize))> = None;
        let mut goto_at: Option<native_lsp::Position> = None;
        let mut references_at: Option<native_lsp::Position> = None;
        let mut hierarchy_at: Option<(native_lsp::Position, native_lsp::HierarchyKind)> = None;
        let mut rename_at: Option<native_lsp::Position> = None;
        let mut open_file_picker = false;
        let mut toggle_pin = false;
        let mut recent_open: Option<PathBuf> = None;
        let activated_editor;
        let language: Language = language_for_path(&path.to_string_lossy());
        let is_markdown = markdown::supported(path);
        let preview_key = format!("native:{}", path.display());
        let preview_hovered = self
            .markdown
            .entries
            .get(&preview_key)
            .is_some_and(|preview| preview.contains_pointer(ui));
        let preview_scroll = if preview_hovered {
            self.markdown
                .entries
                .get(&preview_key)
                .map(|preview| preview.scroll_fraction)
        } else {
            None
        };
        let source_scroll;
        // Split views share one buffer. Their widget and focus ids must
        // still differ per pane, or the second split inherits the first
        // pane's scroll offset and keyboard focus.
        let source_id = format!(
            "{:?}",
            ui.make_persistent_id((path, project, tab, node, ui.ctx().viewport_id()))
        );
        if self.native_focus_pending.as_ref().is_some_and(|target| {
            project == Some(target.project.as_str())
                && tab == Some(target.group.as_str())
                && node == Some(target.path.node_path())
        }) && self
            .native_docs
            .get(path)
            .is_some_and(|doc| doc.doc.is_some())
        {
            self.native_focus_pending = None;
            ui.memory_mut(|memory| memory.request_focus(source_focus_id(&source_id)));
        }
        let rename_applying = self
            .native_rename
            .as_ref()
            .is_some_and(|dialog| dialog.applying_to(path));
        // Diagnostics snapshots for this frame: the manager borrow ends
        // before the buffer borrow below begins.
        let marks: Vec<DiagnosticMark> = self.lsp.row_marks(path);
        let problems = self.lsp.problems_for(path);
        // "Copy reference" names the file relative to its project.
        let reference = self
            .selected_project()
            .and_then(|project| path.strip_prefix(&project.path).ok())
            .unwrap_or(path)
            .display()
            .to_string();
        // Blame column for this frame (blank while a fresh blame runs).
        let blame_revision = self
            .native_docs
            .get(path)
            .and_then(|doc| doc.doc.as_ref())
            .map(Buffer::revision);
        let annotations =
            blame_revision.and_then(|revision| self.editor_blame_annotations(path, revision));
        let blame_offer = (!crate::remote_ui::is_snapshot(&self.paths.data, path))
            .then(|| self.editor_blame.contains_key(path));
        let mut blame_toggle = false;
        let mut blame_click = None;
        let inlay_answer = self
            .state
            .settings
            .inlay_hints
            .then(|| self.lsp.inlay_hints(path))
            .flatten();
        let remote_snapshot = crate::remote_ui::is_snapshot(&self.paths.data, path);
        // The find bar is checked out while this view borrows the buffer.
        let mut find = if self.find_bar.as_ref().is_some_and(|bar| bar.path == path) {
            self.find_bar.take()
        } else {
            None
        };
        let mut completion = if self
            .completion
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.completion.take()
        } else {
            None
        };
        let mut complete_now = false;
        let mut signature = if self
            .signature
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.signature.take()
        } else {
            None
        };
        let mut signature_request = None;
        let mut actions_popup = if self
            .code_actions
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.code_actions.take()
        } else {
            None
        };
        let mut format_now = false;
        let mut actions_at: Option<native_lsp::Range> = None;
        let mut picked_action: Option<native_lsp::CodeActionItem> = None;
        let git_base = if crate::remote_ui::is_snapshot(&self.paths.data, path) {
            None
        } else {
            self.git_base(path)
        };
        let mut gutter_popup = if self
            .gutter_popup
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.gutter_popup.take()
        } else {
            None
        };
        let change_colors = [
            appearance::color(&self.theme.git_added),
            appearance::color(&self.theme.git_modified),
            appearance::color(&self.theme.git_deleted),
        ];
        let mut completion_request = None;
        let caret_rect;
        let post = {
            let doc = self.native_doc(path, vim);
            // Split views of one buffer keep their own caret and layout.
            doc.enter_view(&source_id);
            let mut post = PostEdit::Keep;
            // Clicking a header button moves keyboard focus to it; hand focus
            // back to the file so arrows and typing keep editing text.
            let file_focus = source_focus_id(&source_id);
            let current_symbol = doc.doc.as_ref().and_then(|_| {
                let line = u32::try_from(doc.engine.cursor().line).ok()?;
                native_lsp::innermost_symbol(&outline, line)
                    .and_then(|index| outline.get(index))
                    .map(|symbol| symbol.name.as_str())
            });
            let mut heading_jump = None;
            // Wrapped: on a narrow pane the mode badge and buttons move to
            // a second row instead of being clipped off the edge.
            ui.horizontal_wrapped(|ui| {
                let title_width = (ui.available_width() - if is_markdown { 260.0 } else { 200.0 })
                    .clamp(48.0, 260.0);
                ui.add_sized(
                    [title_width, appearance::TOOLBAR_BUTTON],
                    egui::Label::new(egui::RichText::new(&breadcrumb).weak()).truncate(),
                )
                .on_hover_text(&file_location);
                if doc.crlf {
                    ui.weak("CRLF")
                        .on_hover_text("Windows line endings; saving keeps them");
                }
                if let Some(symbol) = current_symbol {
                    ui.weak("›");
                    ui.monospace(symbol);
                }
                if doc.dirty() {
                    ui.monospace("●");
                }
                if is_markdown {
                    for (option, icon) in [
                        (markdown::Mode::Edit, "Pencil"),
                        (markdown::Mode::Preview, "Eye"),
                        (markdown::Mode::Split, "Columns2"),
                    ] {
                        let response = appearance::selectable_icon(
                            ui,
                            icon,
                            option.label(),
                            doc.markdown_mode == option,
                        );
                        #[cfg(feature = "test-support")]
                        crate::diagnostics::record(
                            ui.ctx(),
                            &format!("native-markdown-mode:{}", option.label()),
                            response.rect,
                        );
                        if response.clicked() {
                            doc.markdown_mode = option;
                            if option == markdown::Mode::Edit {
                                ui.memory_mut(|memory| memory.request_focus(file_focus));
                            }
                        }
                    }
                    appearance::icon_menu_button(ui, "ListTree", |ui| {
                        let Some(buffer) = doc.doc.as_ref() else {
                            return;
                        };
                        let mut found = false;
                        for block in parse_blocks(buffer) {
                            let BlockKind::Heading(level) = block.kind else {
                                continue;
                            };
                            found = true;
                            let title = buffer.line_text(block.start_line);
                            let title = title.trim_start_matches('#').trim();
                            if ui
                                .button(format!("{} {title}", "·".repeat(level.into())))
                                .clicked()
                            {
                                heading_jump = Some(block.start_line);
                                ui.close();
                            }
                        }
                        if !found {
                            ui.weak("No headings");
                        }
                    })
                    .response
                    .on_hover_text("Go to heading");
                } else if vim {
                    // Modeless editing has no mode to report: the badge
                    // would always read NORMAL and only add noise.
                    let mode = doc.engine.mode();
                    ui.colored_label(mode_color(mode), format!("-- {} --", mode_name(mode)));
                }
                if doc.saving {
                    ui.weak("Saving…");
                }
                if rename_applying {
                    ui.weak("Renaming…");
                }
                if ui
                    .add_enabled_ui(doc.dirty() && !doc.saving && !rename_applying, |ui| {
                        appearance::sidebar_action(ui, "Save", "Save file")
                    })
                    .inner
                    .clicked()
                {
                    doc.force_save = false;
                    doc.start_save(&services);
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                // Reload while dirty would silently discard edits: disabled.
                let reload = ui
                    .add_enabled_ui(!doc.dirty() && !doc.loading && !rename_applying, |ui| {
                        appearance::sidebar_action(ui, "RefreshCw", "Reload file")
                    })
                    .inner
                    .on_disabled_hover_text("Unsaved changes are kept");
                if reload.clicked() {
                    doc.load_error = None;
                    doc.loading = false;
                    doc.load_rx = None;
                    doc.doc = None;
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                if native_lsp::server_language(language).is_some()
                    && appearance::sidebar_action(ui, "ListTree", "Find references").clicked()
                {
                    let cursor = doc.engine.cursor();
                    let row = doc
                        .doc
                        .as_ref()
                        .map(|live| live.line_text(cursor.line))
                        .unwrap_or_default();
                    references_at = Some(native_lsp::Position {
                        line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                        character: native_lsp::char_col_to_utf16(&row, cursor.col),
                    });
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                if appearance::selectable_icon(
                    ui,
                    "FileText",
                    if pinned {
                        "Remove bookmark"
                    } else {
                        "Bookmark file"
                    },
                    pinned,
                )
                .clicked()
                {
                    toggle_pin = true;
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                appearance::icon_menu_button(ui, "History", |ui| {
                    if recent_files.is_empty() {
                        ui.weak("No other recent files in this project");
                    }
                    for item in &recent_files {
                        let name = item.file_name().unwrap_or_default().to_string_lossy();
                        let marker = if pinned_files.contains(item) {
                            "◆ "
                        } else {
                            ""
                        };
                        if ui
                            .button(format!("{marker}{name}"))
                            .on_hover_text(item.display().to_string())
                            .clicked()
                        {
                            recent_open = Some(item.clone());
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Recent files");
            });
            if let Some(line) = heading_jump
                && let Some(buffer) = doc.doc.as_mut()
            {
                doc.markdown_mode = markdown::Mode::Edit;
                doc.engine.place_cursor(buffer, line, 0);
                ui.memory_mut(|memory| memory.request_focus(file_focus));
            }
            if let Some(error) = doc.save_error.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(failed_color, format!("Save failed: {error}"));
                    if error.contains("Changed on disk") && ui.small_button("Save anyway").clicked()
                    {
                        doc.force_save = true;
                        doc.start_save(&services);
                        ui.memory_mut(|memory| memory.request_focus(file_focus));
                    }
                });
            }
            let on_disk = doc.disk_mtime_cached(&services);
            if !doc.dirty() && doc.doc.is_some() && on_disk != doc.loaded_mtime && !doc.loading {
                // Clean buffer follows the disk without asking; nothing is lost.
                doc.doc = None;
                doc.load_rx = None;
            }
            if doc.dirty() && doc.doc.is_some() && on_disk != doc.loaded_mtime {
                ui.weak("Changed on disk. Reload is disabled while dirty; Save anyway overwrites.");
            }
            if let Some(error) = doc.load_error.clone() {
                ui.colored_label(failed_color, format!("Could not open: {error}"));
                return;
            }
            let Some(buffer) = doc.doc.as_mut() else {
                ui.weak("Loading…");
                ui.ctx().request_repaint();
                return;
            };
            let mode = if is_markdown {
                doc.markdown_mode
            } else {
                markdown::Mode::Edit
            };
            // `:set wrap` / `:set nowrap` (also from `~/.vimrc`) override
            // the per-type default: Markdown wraps, code does not.
            let soft_wrap = doc.engine.wrap().unwrap_or(is_markdown);
            let squiggles = terminator_native_edit::view::squiggles(&*buffer, &problems);
            let inlay_hints = if let Some((revision, hints)) = &inlay_answer {
                doc.inlay.adopt(&*buffer, *revision, hints);
                doc.inlay.visible(&*buffer)
            } else {
                doc.inlay.clear();
                std::sync::Arc::new(Vec::new())
            };
            let revision_before = buffer.revision();
            // Gutter changes against HEAD, once per edit (or new base).
            let (gutter_hunks, change_marks) = match &git_base {
                Some(base) => {
                    let key = (
                        buffer.revision(),
                        std::sync::Arc::as_ptr(base).cast::<u8>() as usize,
                    );
                    match &doc.gutter {
                        Some((revision, ptr, hunks, marks)) if (*revision, *ptr) == key => {
                            (std::sync::Arc::clone(hunks), std::sync::Arc::clone(marks))
                        }
                        _ => {
                            let base_text = if doc.crlf {
                                base.replace("\r\n", "\n")
                            } else {
                                base.to_string()
                            };
                            let hunks =
                                std::sync::Arc::new(gutter::hunks(&base_text, &buffer.text()));
                            let marks = std::sync::Arc::new(
                                hunks.iter().map(|hunk| hunk.mark).collect::<Vec<_>>(),
                            );
                            doc.gutter = Some((
                                key.0,
                                key.1,
                                std::sync::Arc::clone(&hunks),
                                std::sync::Arc::clone(&marks),
                            ));
                            (hunks, marks)
                        }
                    }
                }
                None => (std::sync::Arc::default(), std::sync::Arc::default()),
            };
            let popup_keys = completion
                .as_ref()
                .is_some_and(|popup| !popup.visible.is_empty())
                || actions_popup
                    .as_ref()
                    .is_some_and(|popup| !popup.items.is_empty());
            let mut find_matches = std::sync::Arc::new(Vec::new());
            let mut find_current = None;
            if let Some(bar) = find.as_mut() {
                // Split views of one file share the bar: the first view
                // this frame draws it, the others only highlight.
                let pass = ui.ctx().cumulative_pass_nr();
                let (outcome, matches) = if bar.drawn == Some(pass) {
                    (find::FindOutcome::default(), bar.matches(buffer))
                } else {
                    bar.drawn = Some(pass);
                    find::find_bar(find::FindBarInput {
                        ui,
                        bar,
                        buffer,
                        engine: &mut doc.engine,
                        read_only: rename_applying || remote_snapshot,
                    })
                };
                find_matches =
                    std::sync::Arc::new(matches.iter().map(|m| (m.line, m.start, m.end)).collect());
                find_current = bar.current;
                if outcome.close {
                    find = None;
                    ui.memory_mut(|memory| memory.request_focus(source_focus_id(&source_id)));
                }
            }
            let source = |ui: &mut egui::Ui| {
                show_source(
                    ui,
                    buffer,
                    &mut doc.engine,
                    ShowSourceInput {
                        options: &SourceOptions {
                            vim,
                            read_only: rename_applying || remote_snapshot,
                            menu_style: Some(appearance::menu_style),
                            menu_header: Some(appearance::target_header),
                            menu_item: Some(appearance::menu_item),
                            soft_wrap,
                            font_size: if is_markdown { 14.0 } else { 13.0 },
                            scroll_fraction: if mode == markdown::Mode::Split {
                                preview_scroll
                            } else {
                                None
                            },
                            diagnostics: marks,
                            squiggles,
                            find_matches,
                            find_current,
                            popup_keys,
                            change_marks,
                            change_colors,
                            inlay_hints,
                            reference: Some(reference),
                            annotations,
                            blame: blame_offer,
                            ..SourceOptions::default()
                        },
                        highlight: &mut doc.highlight,
                        language,
                        id: &source_id,
                    },
                )
            };
            let outcome = match mode {
                markdown::Mode::Preview => None,
                markdown::Mode::Edit => Some(source(ui)),
                markdown::Mode::Split => {
                    let width = ui.available_width();
                    Some(
                        egui::Panel::left(egui::Id::unique(("native-markdown-editor", path)))
                            .resizable(true)
                            .default_size(width * 0.5)
                            .size_range(80.0..=(width - 80.0).max(80.0))
                            .frame(egui::Frame::NONE)
                            .show(ui, source)
                            .inner,
                    )
                }
            };
            activated_editor = outcome.as_ref().is_some_and(|outcome| outcome.activated);
            caret_rect = outcome.as_ref().and_then(|outcome| outcome.caret_rect);
            if let Some(outcome) = outcome.as_ref() {
                blame_toggle = outcome.toggle_blame;
                blame_click = outcome.annotation_clicked;
            }
            if let Some((index, at)) = outcome.as_ref().and_then(|outcome| outcome.change_clicked) {
                gutter_popup = Some(GutterPopup {
                    path: path.to_owned(),
                    index,
                    at,
                });
            }
            source_scroll = outcome.as_ref().map(|outcome| outcome.scroll_fraction);
            if let Some(outcome) = outcome {
                #[cfg(feature = "test-support")]
                crate::diagnostics::record(ui.ctx(), "native-file", outcome.content_rect);
                #[cfg(feature = "test-support")]
                if let Some(rect) = outcome.rename_action_rect {
                    crate::diagnostics::record(ui.ctx(), "native-rename-action", rect);
                }
                #[cfg(feature = "test-support")]
                if let Some(rect) = outcome.hover_action_rect {
                    crate::diagnostics::record(ui.ctx(), "native-hover-action", rect);
                }
                for effect in outcome.effects {
                    match effect {
                        Effect::Save => {
                            doc.force_save = false;
                            doc.start_save(&services);
                        }
                        Effect::SaveForce => {
                            doc.force_save = true;
                            doc.start_save(&services);
                        }
                        Effect::Quit => {
                            if doc.dirty() {
                                doc.save_error =
                                    Some("No write since last change (add ! to override)".into());
                            } else {
                                post = PostEdit::Close { force: false };
                            }
                        }
                        Effect::QuitForce => {
                            post = PostEdit::Close { force: true };
                        }
                        Effect::WriteQuit { force } => {
                            // The write is async: closing here would always see
                            // a dirty buffer and refuse, so defer to the
                            // save-settled check instead.
                            doc.force_save = force;
                            doc.start_save(&services);
                            defer_close_after_save = true;
                            wq_issuer = Self::close_issuer(ui, project, tab, node);
                        }
                        Effect::QuitAll { force } => {
                            quit_all = Some(force);
                        }
                        Effect::Split => {
                            splits.push(NativeSplit::Below);
                        }
                        Effect::Vsplit => {
                            splits.push(NativeSplit::Beside);
                        }
                        Effect::TabNew => {
                            splits.push(NativeSplit::NewTab);
                        }
                        Effect::OpenFilePicker => {
                            open_file_picker = true;
                        }
                        Effect::Hover => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            hover_at = Some((
                                native_lsp::Position {
                                    line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                    character: native_lsp::char_col_to_utf16(&row, cursor.col),
                                },
                                (cursor.line, cursor.col),
                            ));
                        }
                        Effect::GotoDefinition => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            goto_at = Some(native_lsp::Position {
                                line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                character: native_lsp::char_col_to_utf16(&row, cursor.col),
                            });
                        }
                        Effect::FindReferences => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            references_at = Some(native_lsp::Position {
                                line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                character: native_lsp::char_col_to_utf16(&row, cursor.col),
                            });
                        }
                        Effect::Hierarchy { types } => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            hierarchy_at = Some((
                                native_lsp::Position {
                                    line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                    character: native_lsp::char_col_to_utf16(&row, cursor.col),
                                },
                                if types {
                                    native_lsp::HierarchyKind::Type
                                } else {
                                    native_lsp::HierarchyKind::Call
                                },
                            ));
                        }
                        Effect::RenameSymbol => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            rename_at = Some(native_lsp::Position {
                                line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                character: native_lsp::char_col_to_utf16(&row, cursor.col),
                            });
                        }
                        Effect::Complete => complete_now = true,
                        Effect::Format => format_now = true,
                        Effect::CodeActions => {
                            // The selection, else the caret, as an LSP range.
                            if let Some(live) = doc.doc.as_ref() {
                                let caret = doc.engine.cursor();
                                let (from, to) = doc.engine.selection().unwrap_or((caret, caret));
                                let position = |cursor: terminator_native_edit::vim::Cursor| {
                                    native_lsp::Position {
                                        line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                        character: native_lsp::char_col_to_utf16(
                                            &live.line_text(cursor.line),
                                            cursor.col,
                                        ),
                                    }
                                };
                                actions_at = Some(native_lsp::Range {
                                    start: position(from),
                                    end: position(to),
                                });
                            }
                        }
                        Effect::FindBar { replace } => {
                            // Seed with a one-line selection, IntelliJ-style.
                            let seed = doc
                                .doc
                                .as_ref()
                                .filter(|_| doc.engine.selection().is_some())
                                .map(|live| doc.engine.copy_text(live))
                                .filter(|text| !text.contains('\n'))
                                .unwrap_or_default();
                            match find.as_mut() {
                                Some(bar) => {
                                    if !seed.is_empty() {
                                        bar.query = seed;
                                    }
                                    bar.replacing |= replace;
                                    bar.focus = true;
                                }
                                None => {
                                    find = Some(find::FindBar::new(path.to_owned(), seed, replace));
                                }
                            }
                        }
                        Effect::Bell => {}
                    }
                }
            }
            // Problems panel: click targets under the header. Clicks
            // place the cursor directly; the buffer is borrowed here.
            if !problems.is_empty() {
                let mut jumps: Vec<(usize, usize)> = Vec::new();
                let errors = problems
                    .iter()
                    .filter(|problem| problem.severity == Some(native_lsp::Severity::Error))
                    .count();
                let warnings = problems
                    .iter()
                    .filter(|problem| problem.severity == Some(native_lsp::Severity::Warning))
                    .count();
                let mut summary = format!(
                    "{} problem{}",
                    problems.len(),
                    if problems.len() == 1 { "" } else { "s" }
                );
                if errors > 0 {
                    summary.push_str(&format!(
                        " · {errors} error{}",
                        if errors == 1 { "" } else { "s" }
                    ));
                }
                if warnings > 0 {
                    summary.push_str(&format!(
                        " · {warnings} warning{}",
                        if warnings == 1 { "" } else { "s" }
                    ));
                }
                ui.horizontal(|ui| {
                    ui.weak(summary);
                });
                // The panel must not push the source view off screen; the
                // first five stay clickable and the rest are counted.
                const LISTED: usize = 5;
                for problem in problems.iter().take(LISTED) {
                    let line = usize::try_from(problem.range.start.line).unwrap_or(usize::MAX);
                    let row = doc
                        .doc
                        .as_ref()
                        .map(|live| live.line_text(line))
                        .unwrap_or_default();
                    let col = native_lsp::utf16_to_char_col(&row, problem.range.start.character);
                    let tag = match problem.severity {
                        Some(native_lsp::Severity::Error) => "E",
                        Some(native_lsp::Severity::Warning) => "W",
                        _ => "·",
                    };
                    let label = format!(
                        "{tag} {}: {}",
                        line.saturating_add(1),
                        problem.message.lines().next().unwrap_or("")
                    );
                    if ui.small_button(label).clicked() {
                        jumps.push((line, col));
                    }
                }
                if problems.len() > LISTED {
                    ui.weak(format!("+{} more", problems.len().saturating_sub(LISTED)));
                }
                for (line, col) in jumps {
                    if let Some(live) = doc.doc.as_mut() {
                        doc.engine.place_cursor(live, line, col);
                    }
                }
            }
            if let Some(popup) = actions_popup.as_mut() {
                if caret_rect.is_some() {
                    popup.anchor = caret_rect;
                }
                match actions::actions_ui(ui, popup) {
                    actions::ActionPick::Pick(index) => {
                        picked_action = popup.items.get(index).cloned();
                        actions_popup = None;
                    }
                    actions::ActionPick::Close => actions_popup = None,
                    actions::ActionPick::None => {}
                }
            }
            let rollback_index = gutter_popup.as_ref().map(|popup| popup.index);
            if gutter::popup_ui(ui, &mut gutter_popup, &gutter_hunks, change_colors[2])
                && let Some(hunk) = rollback_index.and_then(|index| gutter_hunks.get(index))
                && let Some(live) = doc.doc.as_mut()
            {
                gutter::rollback(live, &mut doc.engine, hunk);
            }
            if let Some(live) = doc.doc.as_ref() {
                signature_request =
                    complete::signature_after_frame(complete::SignatureAfterFrameInput {
                        popup: &mut signature,
                        buffer: live,
                        engine: &doc.engine,
                        revision_before,
                        server: native_lsp::server_language(language).is_some(),
                    });
                if let Some(shown) = signature.as_mut() {
                    if caret_rect.is_some() {
                        shown.anchor = caret_rect;
                    }
                    complete::signature_ui(ui, shown);
                }
            }
            if let Some(live) = doc.doc.as_mut() {
                completion_request = complete::after_frame(complete::AfterFrameInput {
                    ui,
                    popup: &mut completion,
                    buffer: live,
                    engine: &mut doc.engine,
                    revision_before,
                    manual: complete_now,
                    caret_rect,
                    server: native_lsp::server_language(language).is_some(),
                });
            }
            // The view the user clicked or types in owns the live caret
            // between frames; another view only borrowed it to draw.
            if activated_editor || ui.memory(|memory| memory.has_focus(source_focus_id(&source_id)))
            {
                doc.focused_view = Some(source_id.clone());
            }
            doc.restore_focused_view();
            post
        };
        if find.is_some() {
            self.find_bar = find;
        }
        if completion.is_some() {
            self.completion = completion;
        }
        if gutter_popup.is_some() {
            self.gutter_popup = gutter_popup;
            if blame_toggle {
                self.toggle_editor_blame(path);
            }
            if let Some(line) = blame_click {
                self.open_blame_commit(path, line);
            }
        }
        if signature.is_some() {
            self.signature = signature;
        }
        if actions_popup.is_some() {
            self.code_actions = actions_popup;
        }
        if let Some(item) = picked_action {
            self.run_code_action(path, &item);
        }
        if (format_now || actions_at.is_some())
            && let Some((revision, text)) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.buffer())
                .map(|buffer| (buffer.revision(), buffer.text()))
        {
            // Both act on the text as typed: send it past the debounce.
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
            if format_now && !self.lsp.request_format(path, revision) {
                self.info = Some("The language server is not ready to format".into());
            }
            if let Some(range) = actions_at {
                match self.lsp.request_code_actions(path, range) {
                    Some(generation) => {
                        self.code_actions = Some(ActionsPopup {
                            path: path.to_owned(),
                            generation: Some(generation),
                            items: Vec::new(),
                            selected: 0,
                            anchor: caret_rect,
                        });
                    }
                    None => self.info = Some("The language server is not ready".into()),
                }
            }
        }
        self.drain_edit_answers();
        if let Some((position, line, open)) = signature_request
            && let Some((revision, text)) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.buffer())
                .map(|buffer| (buffer.revision(), buffer.text()))
        {
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
            if let Some(generation) = self.lsp.request_signature_help(path, position) {
                // Keep showing the previous signature until the update lands.
                let help = self
                    .signature
                    .take()
                    .filter(|shown| shown.path == path && shown.open == open)
                    .and_then(|shown| shown.help);
                self.signature = Some(SignaturePopup {
                    path: path.to_owned(),
                    generation,
                    help,
                    line,
                    open,
                    anchor: caret_rect,
                });
            }
        }
        if let Some((answer_path, generation, help)) = self.lsp.take_signature()
            && let Some(shown) = self.signature.as_mut()
            && shown.generation == generation
            && shown.path == answer_path
        {
            shown.help = help;
        }
        // Completion asks with the live text: push it past the sync
        // debounce first so the server sees what was just typed.
        if let Some((position, line, start)) = completion_request
            && let Some((revision, text)) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.buffer())
                .map(|buffer| (buffer.revision(), buffer.text()))
        {
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
            if let Some(generation) = self.lsp.request_completion(path, position) {
                self.completion = Some(CompletionPopup {
                    path: path.to_owned(),
                    generation: Some(generation),
                    items: Vec::new(),
                    line,
                    start,
                    visible: Vec::new(),
                    selected: 0,
                    anchor: caret_rect,
                });
            }
        }
        if let Some(results) = self.lsp.take_completion()
            && let Some(popup) = self.completion.as_mut()
            && popup.generation == Some(results.generation)
            && popup.path == results.path
        {
            popup.items = results.items;
            popup.generation = None;
        }
        if is_markdown {
            self.native_markdown_preview(NativeMarkdownPreviewInput {
                ui,
                path,
                preview_key: &preview_key,
                preview_hovered,
                source_scroll,
                remote_snapshot,
            });
        }
        if let PostEdit::Close { force } = post {
            self.pending_native_close.push((
                path.to_owned(),
                force,
                Self::close_issuer(ui, project, tab, node),
            ));
            return;
        }
        if activated_editor {
            self.follow_focus_tab(Some(Tab::NativeEditor {
                path: path.to_owned(),
            }));
        }
        if open_file_picker {
            self.open_path = true;
        }
        if toggle_pin {
            if pinned {
                self.preferences.pinned_native_files.remove(path);
            } else {
                self.preferences.pinned_native_files.insert(path.to_owned());
            }
        }
        if let Some(path_to_open) = recent_open
            && let Some(uri) = native_lsp::file_uri(&path_to_open)
        {
            let at = native_lsp::Position {
                line: 0,
                character: 0,
            };
            self.pending_goto.push(lsp_manager::GotoJump {
                from: path.to_owned(),
                targets: vec![native_lsp::GotoTarget {
                    uri: uri.to_string(),
                    range: native_lsp::Range { start: at, end: at },
                }],
            });
        }
        self.native_lsp_sync(path, language);
        self.request_editor_blame(ui.ctx(), path);
        if self.state.settings.inlay_hints
            && let Some(lines) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.doc.as_ref())
                .map(|buffer| buffer.line_count())
        {
            self.lsp.request_inlay_hints(path, lines);
        }
        if let Some((position, cursor)) = hover_at {
            self.lsp.request_hover(path, position, cursor);
        }
        if let Some(position) = goto_at {
            self.lsp.request_definition(path, position);
        }
        if let Some(position) = references_at {
            self.lsp.request_references(path, position);
        }
        if let Some((position, kind)) = hierarchy_at {
            self.start_hierarchy(path, position, kind);
        }
        if let Some(position) = rename_at {
            self.begin_native_rename(path, project, position);
        }
        if let Some(results) = self.lsp.take_references() {
            self.show_usages("Find Usages", results.targets);
        }
        // Finished jumps queue until the workspace is checked back in:
        // this view renders while it is checked out, so opening the
        // target now would miss its project and drop the jump.
        if let Some(jump) = self.lsp.take_goto() {
            self.pending_goto.push(jump);
        }
        self.show_hover_popup(ui, path);
        if defer_close_after_save {
            self.native_close_after_save = Some(path.to_owned());
            self.native_close_after_save_issuer = wq_issuer.map(CloseIssuer::deferred);
        }
        // `:qa` resolves at drain time: the current workspace is checked
        // out of `layouts` here, so its tabs are invisible to the dirty
        // check until check-in.
        if quit_all.is_some() {
            self.pending_quit_all = quit_all;
        }
        // The issuing viewport travels with the request so splits land
        // in the floating window they were typed in.
        let viewport = ui.ctx().viewport_id();
        for split in splits {
            self.pending_native_splits
                .push((viewport, path.to_owned(), split));
        }
    }
}
