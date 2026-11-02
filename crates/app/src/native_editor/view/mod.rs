//! The native editor pane, rendered once per docked or floating view.
//!
//! One frame runs in three phases: per-frame values are read and the
//! file's popups are checked out of the app ([`frame`]), the buffer is
//! borrowed for the header, source view, and popups ([`ViewPass`]), and
//! the requests that pass queued run once the borrow ends ([`after`]).
mod after;
mod assist;
mod body;
mod effects;
mod format;
mod frame;
mod header;
mod panels;
mod support;

use super::*;
use std::collections::HashSet;
use std::sync::Arc;
use support::{NativeClosePromptBarInput, NativeMarkdownPreviewInput};
use terminator_native_edit::view::Annotation;

/// The docked or floating view under render.
#[derive(Clone, Copy)]
struct ViewSite<'a> {
    path: &'a Path,
    project: Option<&'a str>,
    tab: Option<&'a str>,
    node: Option<egui_dock::NodePath>,
}

/// Breadcrumb and the pinned/recent file menus.
struct ViewFiles {
    file_location: String,
    breadcrumb: String,
    pinned_files: HashSet<PathBuf>,
    recent_files: Vec<PathBuf>,
    pinned: bool,
}

/// Markdown preview identity and scroll sync for this view.
struct ViewPreview {
    key: String,
    hovered: bool,
    scroll: Option<f32>,
}

/// Per-frame values read before the buffer is borrowed.
struct ViewFrame {
    vim: bool,
    failed_color: Color32,
    services: gui_services::Services,
    files: ViewFiles,
    outline: Arc<[native_lsp::Symbol]>,
    language: Language,
    is_markdown: bool,
    preview: ViewPreview,
    /// Split views share one buffer. Their widget and focus ids must
    /// still differ per pane, or the second split inherits the first
    /// pane's scroll offset and keyboard focus.
    source_id: String,
    rename_applying: bool,
    problems: Vec<native_lsp::Diagnostic>,
    blame_offer: Option<bool>,
    inlay_answer: Option<(u64, Arc<[native_lsp::InlayHint]>)>,
    remote_snapshot: bool,
    /// Offer Local History: a local file and a capable daemon.
    local_history: bool,
    git_base: Option<Arc<str>>,
    change_colors: [Color32; 3],
}

/// Values moved into the source view's options.
struct SourceDecor {
    marks: Vec<DiagnosticMark>,
    /// "Copy reference" names the file relative to its project.
    reference: String,
    annotations: Option<Arc<Vec<Annotation>>>,
}

/// Popups for this file, checked out of the app while the view borrows
/// the buffer.
struct ViewPopups {
    find: Option<FindBar>,
    completion: Option<CompletionPopup>,
    signature: Option<SignaturePopup>,
    actions: Option<ActionsPopup>,
    gutter: Option<GutterPopup>,
}

/// What the frame asks for once the buffer borrow ends. The workspace is
/// checked out while this renders, so layout changes, closes, and
/// language-server requests wait until the drain below.
#[derive(Default)]
struct ViewRequests {
    defer_close_after_save: bool,
    /// The view issuing `:wq`: the save settles frames later, when a
    /// different view of the file may render first.
    wq_issuer: Option<CloseIssuer>,
    quit_all: Option<bool>,
    splits: Vec<NativeSplit>,
    hover_at: Option<(native_lsp::Position, (usize, usize))>,
    goto_at: Option<native_lsp::Position>,
    /// Go to Implementation / Type Declaration at a position.
    locate_at: Option<(native_lsp::Position, native_lsp::Locate)>,
    /// Find in Files asked for, with its seed query.
    find_in_files: Option<String>,
    references_at: Option<native_lsp::Position>,
    hierarchy_at: Option<(native_lsp::Position, native_lsp::HierarchyKind)>,
    rename_at: Option<native_lsp::Position>,
    /// Add to agent context: 1-based first and last line.
    agent_context: Option<(usize, usize)>,
    open_file_picker: bool,
    toggle_pin: bool,
    open_local_history: bool,
    recent_open: Option<PathBuf>,
    activated_editor: bool,
    blame_toggle: bool,
    /// Clicked blame line, and whether Cmd/Ctrl was held (open on web).
    blame_click: Option<(usize, bool)>,
    /// Gutter popup: open the working-tree diff of this file.
    gutter_diff: bool,
    /// Gutter popup: stage this block of the saved buffer text.
    gutter_stage: Option<(terminator_native_edit::view::ChangeMark, String)>,
    complete_now: bool,
    signature_request: Option<(native_lsp::Position, usize, usize)>,
    format_now: bool,
    /// The selection when formatting was asked for (range formatting).
    format_range: Option<native_lsp::Range>,
    actions_at: Option<native_lsp::Range>,
    /// The picked action and the buffer revision it was listed for.
    picked_action: Option<(native_lsp::CodeActionItem, u64)>,
    completion_request: Option<(native_lsp::Position, usize, usize)>,
    caret_rect: Option<egui::Rect>,
    source_scroll: Option<f32>,
}

/// One view's frame while it borrows the shared buffer.
struct ViewPass<'a> {
    doc: &'a mut NativeDoc,
    frame: &'a ViewFrame,
    site: ViewSite<'a>,
    popups: &'a mut ViewPopups,
    requests: &'a mut ViewRequests,
}

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
        let site = ViewSite {
            path,
            project,
            tab,
            node,
        };
        let services = self.services.clone();
        self.native_view_load(ui, path, &services);
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
        let (frame, decor) = self.native_view_frame(ui, site, services);
        let mut popups = self.native_view_checkout(path);
        let mut requests = ViewRequests::default();
        let revision_before = self.native_revision(path);
        let pass = ViewPass {
            doc: self.native_doc(path, frame.vim),
            frame: &frame,
            site,
            popups: &mut popups,
            requests: &mut requests,
        };
        let Some(post) = pass.run(ui, decor) else {
            return;
        };
        self.native_view_check_in(path, popups, &requests);
        self.native_view_edit_requests(path, frame.language, &mut requests);
        self.native_view_on_type(path, frame.language, revision_before);
        self.drain_edit_answers();
        self.native_view_signature(path, frame.language, &requests);
        self.native_view_completion(path, frame.language, &requests);
        if frame.is_markdown {
            self.native_markdown_preview(NativeMarkdownPreviewInput {
                ui,
                path,
                preview_key: &frame.preview.key,
                preview_hovered: frame.preview.hovered,
                source_scroll: requests.source_scroll,
                remote_snapshot: frame.remote_snapshot,
            });
        }
        if let PostEdit::Close { force } = post {
            self.pending_native_close.push((
                path.to_owned(),
                force,
                Self::close_issuer(CloseIssuerInput {
                    ui,
                    project,
                    tab,
                    node,
                }),
            ));
            return;
        }
        self.native_view_follow_up(path, &frame, &mut requests);
        self.native_lsp_sync(path, frame.language);
        self.request_editor_blame(ui.ctx(), path);
        self.native_view_lsp_requests(site, frame.language, &mut requests);
        self.native_view_highlights(ui.ctx(), path, &frame.source_id);
        self.show_hover_popup(ui, path);
        self.native_view_deferred(ui, path, requests);
    }
}
