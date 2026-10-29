//! The buffer borrow for one native view: status rows, the source view,
//! and what its outcome asks for.
use super::*;
use terminator_native_edit::highlight::HighlightCache;
use terminator_native_edit::inlay::InlayCache;
use terminator_native_edit::view::{ChangeMark, SourceOutcome};

/// Find matches as `(line, start, end)` for the source view.
type FindMatches = Arc<Vec<(usize, usize, usize)>>;

/// Source view results the popups drawn after it need.
struct SourceShown {
    outcome: Option<SourceOutcome>,
    revision_before: u64,
    gutter_hunks: Arc<Vec<gutter::GutterHunk>>,
}

/// Per-frame decorations computed from the buffer for the source view.
struct SourceMarks {
    mode: markdown::Mode,
    read_only: bool,
    soft_wrap: bool,
    squiggles: Vec<terminator_native_edit::view::Squiggle>,
    inlay_hints: Arc<Vec<terminator_native_edit::inlay::InlayHint>>,
    change_marks: Arc<Vec<ChangeMark>>,
    popup_keys: bool,
    find_matches: FindMatches,
    find_current: Option<usize>,
}

struct GutterMarksInput<'a> {
    cache: &'a mut Option<GutterCache>,
    crlf: bool,
    buffer: &'a Doc,
    base: Option<&'a Arc<str>>,
}

struct FindPassInput<'a> {
    buffer: &'a mut Doc,
    engine: &'a mut VimEngine,
    read_only: bool,
    source_id: &'a str,
}

struct RenderSourceInput<'a> {
    buffer: &'a mut Doc,
    engine: &'a mut VimEngine,
    highlight: &'a mut HighlightCache,
    path: &'a Path,
    language: Language,
    source_id: &'a str,
    options: SourceOptions,
}

impl ViewPass<'_> {
    /// Draw this view. `None` means the buffer is not ready (load failed
    /// or still loading) and the rest of the frame is skipped.
    pub(super) fn run(mut self, ui: &mut egui::Ui, decor: SourceDecor) -> Option<PostEdit> {
        // Split views of one buffer keep their own caret and layout.
        self.doc.enter_view(&self.frame.source_id);
        let mut post = PostEdit::Keep;
        let heading_jump = self.header(ui);
        self.jump_to_heading(ui, heading_jump);
        self.save_error_row(ui);
        self.follow_disk(ui);
        if self.load_failed(ui) {
            return None;
        }
        let shown = self.show_source(ui, decor)?;
        self.adopt_outcome(ui, shown.outcome, &mut post);
        self.problems_panel(ui);
        self.actions_popup(ui);
        self.gutter_popup(ui, &shown.gutter_hunks);
        self.signature_popup(ui, shown.revision_before);
        self.completion_after_frame(ui, shown.revision_before);
        self.claim_focus(ui);
        Some(post)
    }

    pub(super) fn save(&mut self, force: bool) {
        self.doc.force_save = force;
        self.doc.start_save(&self.frame.services);
    }

    /// The caret as a language-server position.
    pub(super) fn caret_at(&self) -> native_lsp::Position {
        let cursor = self.doc.engine.cursor();
        let row = self
            .doc
            .doc
            .as_ref()
            .map(|live| live.line_text(cursor.line))
            .unwrap_or_default();
        native_lsp::Position {
            line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
            character: native_lsp::char_col_to_utf16(&row, cursor.col),
        }
    }

    pub(super) fn read_only(&self) -> bool {
        self.frame.rename_applying || self.frame.remote_snapshot
    }

    fn jump_to_heading(&mut self, ui: &egui::Ui, heading_jump: Option<usize>) {
        if let Some(line) = heading_jump
            && let Some(buffer) = self.doc.doc.as_mut()
        {
            self.doc.markdown_mode = markdown::Mode::Edit;
            self.doc.engine.place_cursor(buffer, line, 0);
            ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
        }
    }

    fn save_error_row(&mut self, ui: &mut egui::Ui) {
        let Some(error) = self.doc.save_error.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.colored_label(self.frame.failed_color, format!("Save failed: {error}"));
            if error.contains("Changed on disk") && ui.small_button("Save anyway").clicked() {
                self.save(true);
                ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
            }
        });
    }

    /// A clean buffer follows the disk without asking; nothing is lost.
    /// A dirty one only warns.
    fn follow_disk(&mut self, ui: &mut egui::Ui) {
        let doc = &mut *self.doc;
        let on_disk = doc.disk_mtime_cached(&self.frame.services);
        if !doc.dirty() && doc.doc.is_some() && on_disk != doc.loaded_mtime && !doc.loading {
            doc.doc = None;
            doc.load_rx = None;
        }
        if doc.dirty() && doc.doc.is_some() && on_disk != doc.loaded_mtime {
            ui.weak("Changed on disk. Reload is disabled while dirty; Save anyway overwrites.");
        }
    }

    fn load_failed(&self, ui: &mut egui::Ui) -> bool {
        let Some(error) = self.doc.load_error.clone() else {
            return false;
        };
        ui.colored_label(self.frame.failed_color, format!("Could not open: {error}"));
        true
    }

    /// Draw the find bar and the source view (unless Markdown preview
    /// replaces it). `None` while the buffer is still loading.
    fn show_source(&mut self, ui: &mut egui::Ui, decor: SourceDecor) -> Option<SourceShown> {
        let read_only = self.read_only();
        let frame = self.frame;
        let doc = &mut *self.doc;
        let Some(buffer) = doc.doc.as_mut() else {
            ui.weak("Loading…");
            ui.ctx().request_repaint();
            return None;
        };
        let mode = if frame.is_markdown {
            doc.markdown_mode
        } else {
            markdown::Mode::Edit
        };
        // `:set wrap` / `:set nowrap` (also from `~/.vimrc`) override
        // the per-type default: Markdown wraps, code does not.
        let soft_wrap = doc.engine.wrap().unwrap_or(frame.is_markdown);
        let squiggles = terminator_native_edit::view::squiggles(&*buffer, &frame.problems);
        let inlay_hints = Self::inlay_hints(&mut doc.inlay, buffer, frame.inlay_answer.as_ref());
        let revision_before = buffer.revision();
        // Gutter changes against HEAD, once per edit (or new base).
        let (gutter_hunks, change_marks) = Self::gutter_marks(GutterMarksInput {
            cache: &mut doc.gutter,
            crlf: doc.crlf,
            buffer,
            base: frame.git_base.as_ref(),
        });
        let popup_keys = self.popups.popup_keys();
        let (find_matches, find_current) = Self::find_pass(
            ui,
            &mut self.popups.find,
            FindPassInput {
                buffer,
                engine: &mut doc.engine,
                read_only,
                source_id: &frame.source_id,
            },
        );
        let options = Self::source_options(
            frame,
            decor,
            SourceMarks {
                mode,
                read_only,
                soft_wrap,
                squiggles,
                inlay_hints,
                change_marks,
                popup_keys,
                find_matches,
                find_current,
            },
        );
        let outcome = Self::render_source(
            ui,
            mode,
            RenderSourceInput {
                buffer,
                engine: &mut doc.engine,
                highlight: &mut doc.highlight,
                path: self.site.path,
                language: frame.language,
                source_id: &frame.source_id,
                options,
            },
        );
        Some(SourceShown {
            outcome,
            revision_before,
            gutter_hunks,
        })
    }

    fn inlay_hints(
        inlay: &mut InlayCache,
        buffer: &Doc,
        answer: Option<&(u64, Arc<[native_lsp::InlayHint]>)>,
    ) -> Arc<Vec<terminator_native_edit::inlay::InlayHint>> {
        if let Some((revision, hints)) = answer {
            inlay.adopt(buffer, *revision, hints);
            inlay.visible(buffer)
        } else {
            inlay.clear();
            Arc::new(Vec::new())
        }
    }

    fn gutter_marks(
        input: GutterMarksInput<'_>,
    ) -> (Arc<Vec<gutter::GutterHunk>>, Arc<Vec<ChangeMark>>) {
        let GutterMarksInput {
            cache,
            crlf,
            buffer,
            base,
        } = input;
        let Some(base) = base else {
            return (Arc::default(), Arc::default());
        };
        let key = (buffer.revision(), Arc::as_ptr(base).cast::<u8>() as usize);
        if let Some((revision, ptr, hunks, marks)) = cache.as_ref()
            && (*revision, *ptr) == key
        {
            return (Arc::clone(hunks), Arc::clone(marks));
        }
        let base_text = if crlf {
            base.replace("\r\n", "\n")
        } else {
            base.to_string()
        };
        let hunks = Arc::new(gutter::hunks(&base_text, &buffer.text()));
        let marks = Arc::new(hunks.iter().map(|hunk| hunk.mark).collect::<Vec<_>>());
        *cache = Some((key.0, key.1, Arc::clone(&hunks), Arc::clone(&marks)));
        (hunks, marks)
    }

    /// The find bar and its matches. Split views of one file share the
    /// bar: the first view this frame draws it, the others only highlight.
    fn find_pass(
        ui: &mut egui::Ui,
        find: &mut Option<FindBar>,
        input: FindPassInput<'_>,
    ) -> (FindMatches, Option<usize>) {
        let FindPassInput {
            buffer,
            engine,
            read_only,
            source_id,
        } = input;
        let Some(bar) = find.as_mut() else {
            return (Arc::new(Vec::new()), None);
        };
        let pass = ui.ctx().cumulative_pass_nr();
        let (outcome, matches) = if bar.drawn == Some(pass) {
            (find::FindOutcome::default(), bar.matches(buffer))
        } else {
            bar.drawn = Some(pass);
            find::find_bar(find::FindBarInput {
                ui,
                bar,
                buffer,
                engine,
                read_only,
            })
        };
        let find_matches = Arc::new(matches.iter().map(|m| (m.line, m.start, m.end)).collect());
        let find_current = bar.current;
        if outcome.close {
            *find = None;
            ui.memory_mut(|memory| memory.request_focus(source_focus_id(source_id)));
        }
        (find_matches, find_current)
    }

    fn source_options(frame: &ViewFrame, decor: SourceDecor, marks: SourceMarks) -> SourceOptions {
        let SourceDecor {
            marks: diagnostics,
            reference,
            annotations,
        } = decor;
        let SourceMarks {
            mode,
            read_only,
            soft_wrap,
            squiggles,
            inlay_hints,
            change_marks,
            popup_keys,
            find_matches,
            find_current,
        } = marks;
        SourceOptions {
            vim: frame.vim,
            read_only,
            menu_style: Some(appearance::menu_style),
            menu_header: Some(appearance::target_header),
            menu_item: Some(|ui, label, icon, shortcut| {
                appearance::menu_item(appearance::MenuItemInput {
                    ui,
                    label,
                    icon,
                    shortcut,
                })
            }),
            soft_wrap,
            font_size: if frame.is_markdown { 14.0 } else { 13.0 },
            line_spacing: if frame.is_markdown { 1.3 } else { 1.0 },
            scroll_fraction: if mode == markdown::Mode::Split {
                frame.preview.scroll
            } else {
                None
            },
            diagnostics,
            squiggles,
            find_matches,
            find_current,
            popup_keys,
            change_marks,
            change_colors: frame.change_colors,
            inlay_hints,
            reference: Some(reference),
            annotations,
            blame: frame.blame_offer,
            ..SourceOptions::default()
        }
    }

    /// Show the source view as the mode wants it: alone, in the left half
    /// of a Markdown split, or not at all (preview only).
    fn render_source(
        ui: &mut egui::Ui,
        mode: markdown::Mode,
        input: RenderSourceInput<'_>,
    ) -> Option<SourceOutcome> {
        let RenderSourceInput {
            buffer,
            engine,
            highlight,
            path,
            language,
            source_id,
            options,
        } = input;
        let mut source = |ui: &mut egui::Ui| {
            show_source(ShowSourceInput {
                ui,
                doc: buffer,
                engine,
                options: &options,
                highlight,
                language,
                id: source_id,
            })
        };
        match mode {
            markdown::Mode::Preview => None,
            markdown::Mode::Edit => Some(source(ui)),
            markdown::Mode::Split => {
                let width = ui.available_width();
                let shown = egui::Panel::left(egui::Id::unique(("native-markdown-editor", path)))
                    .resizable(true)
                    .default_size(width * 0.5)
                    .size_range(80.0..=(width - 80.0).max(80.0))
                    .frame(egui::Frame::NONE)
                    .show(ui, source);
                // A clear divider between source and preview (MarkEdit-like).
                let color = ui.visuals().weak_text_color().gamma_multiply(0.5);
                ui.painter().vline(
                    shown.response.rect.right(),
                    shown.response.rect.y_range(),
                    egui::Stroke::new(1.0, color),
                );
                Some(shown.inner)
            }
        }
    }

    /// Record what the source view reported and run its engine effects.
    fn adopt_outcome(
        &mut self,
        ui: &egui::Ui,
        outcome: Option<SourceOutcome>,
        post: &mut PostEdit,
    ) {
        let requests = &mut *self.requests;
        requests.activated_editor = outcome.as_ref().is_some_and(|outcome| outcome.activated);
        requests.caret_rect = outcome.as_ref().and_then(|outcome| outcome.caret_rect);
        if let Some(outcome) = outcome.as_ref() {
            requests.blame_toggle = outcome.toggle_blame;
            requests.blame_click = outcome.annotation_clicked;
        }
        if let Some((index, at)) = outcome.as_ref().and_then(|outcome| outcome.change_clicked) {
            self.popups.gutter = Some(GutterPopup {
                path: self.site.path.to_owned(),
                index,
                at,
            });
        }
        requests.source_scroll = outcome.as_ref().map(|outcome| outcome.scroll_fraction);
        let Some(outcome) = outcome else {
            return;
        };
        #[cfg(feature = "test-support")]
        Self::record_fixture_rects(ui, &outcome);
        for effect in outcome.effects {
            self.apply_effect(ui, effect, post);
        }
    }

    #[cfg(feature = "test-support")]
    fn record_fixture_rects(ui: &egui::Ui, outcome: &SourceOutcome) {
        crate::diagnostics::record(ui.ctx(), "native-file", outcome.content_rect);
        if let Some(rect) = outcome.rename_action_rect {
            crate::diagnostics::record(ui.ctx(), "native-rename-action", rect);
        }
        if let Some(rect) = outcome.hover_action_rect {
            crate::diagnostics::record(ui.ctx(), "native-hover-action", rect);
        }
    }

    /// The view the user clicked or types in owns the live caret between
    /// frames; another view only borrowed it to draw.
    fn claim_focus(&mut self, ui: &egui::Ui) {
        if self.requests.activated_editor || ui.memory(|memory| memory.has_focus(self.file_focus()))
        {
            self.doc.focused_view = Some(self.frame.source_id.clone());
        }
        self.doc.restore_focused_view();
    }
}

impl ViewPopups {
    /// A visible completion or code-action list owns the arrow keys.
    fn popup_keys(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|popup| !popup.visible.is_empty())
            || self
                .actions
                .as_ref()
                .is_some_and(|popup| !popup.items.is_empty())
    }
}
