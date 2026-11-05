//! The buffer borrow for one native view: in-file bars, the source view,
//! the Markdown toolbar, and what the source view's outcome asks for.
use super::source_options::{FindMatches, SourceMarks};
use super::*;
use crate::native_editor::file_io::LineEnding;
use terminator_native_edit::highlight::HighlightCache;
use terminator_native_edit::inlay::InlayCache;
use terminator_native_edit::view::{ChangeMark, SourceOutcome};

/// Source view results the popups drawn after it need.
struct SourceShown {
    outcome: Option<SourceOutcome>,
    revision_before: u64,
    gutter_hunks: Arc<Vec<gutter::GutterHunk>>,
    /// The source and preview area, under the in-file bars and find bar.
    area: egui::Rect,
}

struct GutterMarksInput<'a> {
    ctx: &'a egui::Context,
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
        let pass = ui.ctx().cumulative_pass_nr();
        self.doc.drawn.insert(self.frame.source_id.clone(), pass);
        let mut post = PostEdit::Keep;
        self.save_error_row(ui);
        self.follow_disk(ui);
        self.large_file_bar(ui);
        if self.load_failed(ui) {
            return None;
        }
        let shown = self.show_source(ui, decor)?;
        self.adopt_outcome(ui, shown.outcome, &mut post);
        if self.frame.is_markdown {
            self.markdown_toolbar(ui, shown.area);
        }
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

    /// Keyboard focus id of this view's source widget. Clicking a toolbar
    /// button moves focus to it; buttons hand focus back to the file so
    /// arrows and typing keep editing text.
    pub(super) fn file_focus(&self) -> egui::Id {
        source_focus_id(&self.frame.source_id)
    }

    /// Draw the find bar and the source view (unless Markdown preview
    /// replaces it). `None` while the buffer is still loading.
    fn show_source(&mut self, ui: &mut egui::Ui, decor: SourceDecor) -> Option<SourceShown> {
        let read_only = self.read_only();
        let frame = self.frame;
        let full_features = !self.doc.large_file();
        let status_items = self.status_items();
        let doc = &mut *self.doc;
        let Some(buffer) = doc.doc.as_mut() else {
            ui.weak("Loading…");
            ui.ctx().request_repaint();
            return None;
        };
        let (mode, soft_wrap) = Self::layout_mode(frame, doc.markdown_mode, &doc.engine);
        // A remembered scroll waits for a frame that draws the source.
        let top_line = (mode != markdown::Mode::Preview)
            .then(|| doc.top_line.take())
            .flatten();
        let squiggles = Self::squiggle_sets(&mut doc.extras, buffer, frame, full_features);
        self.requests.squiggles = (Arc::clone(&squiggles.0), Arc::clone(&squiggles.1));
        let yield_copy = crate::native_editor::quick_doc::popup_holds_copy(ui.ctx());
        let inlay_hints = Self::inlay_hints(&mut doc.inlay, buffer, frame.inlay_answer.as_ref());
        let revision_before = buffer.revision();
        // Gutter changes against HEAD, once per edit (or new base).
        let (gutter_hunks, change_marks) = Self::gutter_marks(GutterMarksInput {
            ctx: ui.ctx(),
            cache: &mut doc.gutter,
            crlf: doc.line_ending == LineEnding::Crlf,
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
        let area = ui.available_rect_before_wrap();
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
                full_features,
                status_items,
                yield_copy,
                top_line,
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
            area,
        })
    }

    /// The Markdown mode this view shows, and whether source lines wrap:
    /// `:set wrap` / `:set nowrap` (also from `~/.vimrc`) override the
    /// per-type default (Markdown wraps, code does not).
    fn layout_mode(
        frame: &ViewFrame,
        markdown_mode: markdown::Mode,
        engine: &VimEngine,
    ) -> (markdown::Mode, bool) {
        let mode = if frame.is_markdown {
            markdown_mode
        } else {
            markdown::Mode::Edit
        };
        (mode, engine.wrap().unwrap_or(frame.is_markdown))
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
            ctx,
            cache,
            crlf,
            buffer,
            base,
        } = input;
        let Some(base) = base else {
            return (Arc::default(), Arc::default());
        };
        // The diff runs on a worker keyed by revision and base; this
        // frame only snapshots the text when a new diff starts.
        cache
            .get_or_insert_default()
            .current(&gutter::GutterRequest {
                ctx,
                revision: buffer.revision(),
                base,
                crlf,
                text: &|| buffer.text(),
            })
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
            requests.git_compare = outcome.git_compare;
            let web = ui.input(|input| input.modifiers.command);
            requests.blame_click = outcome.annotation_clicked.map(|line| (line, web));
        }
        if let Some((index, at)) = outcome.as_ref().and_then(|outcome| outcome.change_clicked) {
            self.popups.gutter = Some(GutterPopup {
                path: self.site.path.to_owned(),
                index,
                at,
            });
        }
        requests.source_scroll = outcome.as_ref().map(|outcome| outcome.scroll_fraction);
        requests.hover_spot = outcome.as_ref().and_then(|outcome| outcome.hover);
        let Some(outcome) = outcome else {
            return;
        };
        self.status_menus(ui, &outcome.status_rects);
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
            self.doc.focused_at = Some(Instant::now());
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
