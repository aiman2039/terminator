//! Per-frame helpers around the native editor view: load, close prompts,
//! Markdown preview, language-server sync, and the hover window.
use super::*;

pub(super) struct NativeClosePromptBarInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) path: &'a Path,
    pub(super) project: Option<&'a str>,
    pub(super) tab: Option<&'a str>,
    pub(super) node: Option<egui_dock::NodePath>,
}

pub(super) struct NativeMarkdownPreviewInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) path: &'a Path,
    pub(super) preview_key: &'a str,
    pub(super) preview_hovered: bool,
    pub(super) source_scroll: Option<f32>,
    pub(super) remote_snapshot: bool,
}

impl App {
    /// Per-frame load/poll for one native view, plus the language-server
    /// pump and any pending goto cursor placement.
    pub(super) fn native_view_load(
        &mut self,
        path: &Path,
        vim: bool,
        services: &gui_services::Services,
    ) {
        {
            let doc = self.native_doc(path, vim);
            doc.ensure_loading(services);
            doc.poll();
        }
        // Language servers pump even while the file loads: slow serving
        // must never stall typing, and diagnostics arrive when they do.
        self.lsp.poll();
        if let Some(line) = self.native_pending_line.remove(path) {
            let col = self.native_pending_col.remove(path).unwrap_or(0);
            let mut keep = false;
            if let Some(doc) = self.native_docs.get_mut(path) {
                if let Some(buffer) = doc.doc.as_mut() {
                    let chars = native_lsp::utf16_to_char_col(&buffer.line_text(line), col);
                    doc.engine.place_cursor(buffer, line, chars);
                } else if doc.load_error.is_none() {
                    keep = true;
                }
            }
            if keep {
                self.native_pending_line.insert(path.to_owned(), line);
                self.native_pending_col.insert(path.to_owned(), col);
            }
        }
    }

    /// Queue the deferred close for a settled save-then-close (`:wq` or
    /// Save and close). Returns true when the view should stop rendering.
    pub(super) fn native_close_after_save_settled(&mut self, path: &Path) -> bool {
        if self.native_close_after_save.as_deref() == Some(path) {
            let settled = !self.native_dirty(path)
                && !self.native_docs.get(path).is_some_and(|doc| doc.saving);
            if settled {
                // Deferred: the workspace is checked out of `layouts` while
                // this view renders; closing runs after it is checked back in.
                // Consumed here: every other copy rendering later must not
                // queue its own close for the same request.
                let issuer = self.native_close_after_save_issuer.take();
                self.native_close_after_save = None;
                self.pending_native_close
                    .push((path.to_owned(), false, issuer));
                return true;
            }
        }
        false
    }

    /// In-pane unsaved-changes bar for this view's pending tab close.
    /// Returns true when the view should stop rendering this frame.
    pub(super) fn native_close_prompt_bar(&mut self, input: NativeClosePromptBarInput<'_>) -> bool {
        let NativeClosePromptBarInput {
            ui,
            path,
            project,
            tab,
            node,
        } = input;
        // Tab-close prompt for this pane: an in-pane bar, not a modal, so
        // the rest of the GUI stays usable while the choice is pending.
        // Quit-flow prompts carry no issuer and stay modal (`native_close_modal`).
        if let Some((prompt, issuer)) = self.native_close_prompt.clone()
            && prompt == path
            && issuer.is_some()
        {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let enabled = !self
                .native_docs
                .get(path)
                .is_some_and(|doc| doc.saving || doc.loading);
            let bar = appearance::unsaved_close_bar(
                ui,
                appearance::UnsavedCloseBar {
                    theme: &self.theme,
                    message: &format!("{name} has unsaved changes."),
                    enabled,
                },
            );
            if let Some(choice) = bar.choice() {
                self.native_close_prompt = None;
                match choice {
                    appearance::UnsavedCloseChoice::Save => {
                        self.apply_native_close_choice(
                            path,
                            NativeCloseChoice::SaveClose,
                            issuer.clone(),
                        );
                    }
                    // Deferred: the workspace is checked out while this view
                    // renders, so the close runs after it is checked back in.
                    appearance::UnsavedCloseChoice::Discard => {
                        self.pending_native_close
                            .push((path.to_owned(), false, issuer.clone()))
                    }
                    appearance::UnsavedCloseChoice::Cancel => {
                        if let Some(doc) = self.native_docs.get(path) {
                            let focus = source_focus_id(&format!(
                                "{:?}",
                                ui.make_persistent_id((
                                    path,
                                    project,
                                    tab,
                                    node,
                                    ui.ctx().viewport_id()
                                ))
                            ));
                            let _ = doc;
                            ui.memory_mut(|memory| memory.request_focus(focus));
                        }
                    }
                }
                if matches!(
                    choice,
                    appearance::UnsavedCloseChoice::Discard | appearance::UnsavedCloseChoice::Save
                ) {
                    return true;
                }
            }
            ui.separator();
        }
        false
    }

    /// Markdown preview for a native view in Preview or Split mode.
    pub(super) fn native_markdown_preview(&mut self, input: NativeMarkdownPreviewInput<'_>) {
        let NativeMarkdownPreviewInput {
            ui,
            path,
            preview_key,
            preview_hovered,
            source_scroll,
            remote_snapshot,
        } = input;
        let mode = self.native_docs.get(path).map(|doc| doc.markdown_mode);
        if mode != Some(markdown::Mode::Edit) {
            let text = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.doc.as_ref())
                .map(Doc::text);
            if let Some(text) = text {
                let state = if self.native_dirty(path) {
                    "Unsaved changes"
                } else {
                    "Saved"
                };
                let cursor = self.native_docs.get(path).map(|doc| doc.engine.cursor());
                let status = cursor.map_or_else(
                    || state.to_owned(),
                    |cursor| {
                        format!(
                            "Ln {}, Col {}  ·  {state}",
                            cursor.line.saturating_add(1),
                            cursor.col.saturating_add(1)
                        )
                    },
                );
                let remote_base = std::path::PathBuf::from("/__terminator_remote__")
                    .join(path.file_name().unwrap_or_default());
                let preview = self.markdown.snapshot(
                    preview_key,
                    if remote_snapshot { &remote_base } else { path },
                    &text,
                );
                let mut link = None;
                let mut show_preview = |ui: &mut egui::Ui| {
                    let sync = if mode == Some(markdown::Mode::Split) && !preview_hovered {
                        source_scroll
                    } else {
                        None
                    };
                    link = preview.show_synced(ShowSyncedInput {
                        ui,
                        sid: preview_key,
                        status_override: Some(&status),
                        scroll_fraction: sync,
                    });
                };
                if mode == Some(markdown::Mode::Split) {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ui, &mut show_preview);
                } else {
                    show_preview(ui);
                }
                if let Some(link) = link {
                    match link {
                        markdown::Link::File(target) => {
                            self.open_file(OpenFileInput {
                                path: target,
                                line: None,
                                split: None,
                                external: false,
                            });
                        }
                        markdown::Link::Web(url) => {
                            let _ = self.jobs.send(Job::Browser(url));
                        }
                    }
                }
            }
        }
    }

    pub(super) fn native_lsp_sync(&mut self, path: &Path, language: Language) {
        // Language-server sync for the visible buffer: clone the text
        // only when the manager actually wants it.
        if let Some(revision) = self
            .native_docs
            .get(path)
            .and_then(|doc| doc.doc.as_ref())
            .map(|buffer| buffer.revision())
            && self.lsp.sync_doc(path, language, revision)
            && let Some(text) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.doc.as_ref())
                .map(|buffer| buffer.text())
        {
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
        }
    }

    /// Hover answer window for one file: cursor moves and the X both
    /// dismiss; the manager owns the text.
    pub(super) fn show_hover_popup(&mut self, ui: &mut egui::Ui, path: &Path) {
        let cursor = self.native_docs.get(path).map(|doc| doc.engine.cursor());
        let Some(cursor) = cursor else {
            return;
        };
        let Some((text, markdown)) = self.lsp.hover_popup(path, (cursor.line, cursor.col)) else {
            return;
        };
        let mut open = true;
        self.popups
            .window(ui.ctx(), "Hover")
            .open(&mut open)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(500.0)
                    .show(ui, |ui| {
                        ui.set_max_width(700.0);
                        if markdown {
                            egui_commonmark::CommonMarkViewer::new()
                                .explicit_image_uri_scheme(true)
                                .show(ui, &mut self.native_hover_markdown_cache, &text);
                        } else {
                            ui.monospace(&text);
                        }
                    });
            });
        if !open {
            self.lsp.close_hover();
        }
    }
}
