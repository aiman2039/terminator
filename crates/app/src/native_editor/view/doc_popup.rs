//! Quick Documentation for one native view once it drew
//! ([`crate::native_editor::quick_doc`]): time the pointer's rest on a
//! word, open the popup there or at the caret, ask the language server
//! for the docs, and draw the popup with the problems at its spot.
use super::*;
use crate::lsp_manager::HoverKey;
use crate::native_editor::extras::SquiggleSets;
use crate::native_editor::quick_doc::{self, Input, Origin, Shown, Spot, Step, popup};
use terminator_native_edit::view::HoverSpot;

/// While docs are on their way, the frame looks for them this often, for
/// up to [`DOCS_WAIT`]: the language-server pump does not wake the GUI.
const DOCS_POLL: Duration = Duration::from_millis(50);
const DOCS_WAIT: Duration = Duration::from_secs(5);

/// The view under render: its file, the file's language, and its id.
type QuickDocSite<'a> = (&'a Path, Language, &'a str);

impl App {
    /// The popup of one view: hide it as the input says, open it after a
    /// pointer rest or a caret request (F1, Ctrl+J, Cmd/Ctrl+K, `K`), and
    /// draw it.
    pub(super) fn native_quick_doc(
        &mut self,
        ui: &egui::Ui,
        site: QuickDocSite<'_>,
        requests: &mut ViewRequests,
    ) {
        let (path, _, view) = site;
        let now = Instant::now();
        let pass = ui.ctx().cumulative_pass_nr();
        let Some(doc) = self.native_docs.get_mut(path) else {
            return;
        };
        let quick = &mut doc.extras.quick_doc;
        quick.drop_stale(view, pass);
        let pointer = match quick.step(frame_input(ui, view, now, requests.hover_spot.take())) {
            Step::Idle => None,
            Step::Wait(wait) => {
                ui.ctx().request_repaint_after(wait);
                None
            }
            Step::Open(spot) => Some((spot, Origin::Pointer)),
        };
        // A caret request comes after the key that asked for it hid the
        // popup, and wins over a pointer rest in the same frame.
        let opened = requests
            .quick_doc_at
            .take()
            .map(|spot| (spot, Origin::Caret))
            .or(pointer);
        if let Some(opened) = opened {
            self.open_quick_doc(site, opened, (&requests.squiggles, now, pass));
        }
        self.draw_quick_doc(ui, (path, view), (now, pass));
    }

    /// Open the popup on `spot`: the problems there, and its docs. A
    /// pointer rest asks for docs only when Settings shows them on hover,
    /// and says nothing about a server that cannot answer.
    fn open_quick_doc(
        &mut self,
        (path, language, _): QuickDocSite<'_>,
        (spot, origin): (Spot, Origin),
        ((heads, tails), now, pass): (&SquiggleSets, Instant, u64),
    ) {
        let problems = quick_doc::problems_at(heads, tails, &spot);
        let caret = origin == Origin::Caret;
        let (docs, note) = if caret || appearance::editor_style().prefs.editor_hover_docs {
            self.ask_quick_doc((path, language), &spot, now)
        } else {
            (None, None)
        };
        if let Some(doc) = self.native_docs.get_mut(path) {
            doc.extras.quick_doc.shown = Some(Shown {
                spot,
                origin,
                docs,
                problems,
                note: note.filter(|_| caret),
                rect: None,
                pass,
            });
        }
    }

    /// Ask the server for the docs at `spot` of the live text. Returns the
    /// key the answer comes under, or why the server cannot answer now.
    fn ask_quick_doc(
        &mut self,
        (path, language): (&Path, Language),
        spot: &Spot,
        now: Instant,
    ) -> (Option<(HoverKey, Instant)>, Option<String>) {
        let Some((revision, text)) = self
            .native_docs
            .get(path)
            .and_then(|doc| doc.buffer())
            .map(|buffer| (buffer.revision(), buffer.line_text(spot.line)))
        else {
            return (None, None);
        };
        let key = HoverKey {
            path: path.to_owned(),
            revision,
            line: spot.line,
            start: spot.start,
            end: spot.end,
        };
        let position = native_lsp::Position {
            line: u32::try_from(spot.line).unwrap_or(u32::MAX),
            character: native_lsp::char_col_to_utf16(&text, spot.ask_col()),
        };
        // A new question goes with the text as typed, past the debounce.
        if !self.lsp.hover_asked(&key) {
            let _ = self.push_live_text(path, language);
        }
        match self.lsp.request_hover(&key, position) {
            Ok(true) => (Some((key, now)), None),
            Ok(false) => (None, None),
            Err(note) => (None, Some(note)),
        }
    }

    /// Draw the popup of view `view`: the problems, then the docs once
    /// they arrived, or the note. A popup with nothing to say is not
    /// drawn.
    fn draw_quick_doc(
        &mut self,
        ui: &egui::Ui,
        (path, view): (&Path, &str),
        (now, pass): (Instant, u64),
    ) {
        let Some(shown) = self
            .native_docs
            .get_mut(path)
            .and_then(|doc| doc.extras.quick_doc.shown.as_mut())
            .filter(|shown| shown.spot.view == view)
        else {
            return;
        };
        shown.pass = pass;
        let docs = match &shown.docs {
            Some((key, since)) => self.lsp.hover_docs(key).unwrap_or_else(|| {
                if now.saturating_duration_since(*since) < DOCS_WAIT {
                    ui.ctx().request_repaint_after(DOCS_POLL);
                }
                None
            }),
            None => None,
        };
        let visuals = ui.visuals();
        let body = popup::Body {
            problems: &shown.problems,
            colors: (visuals.error_fg_color, visuals.warn_fg_color),
            docs: docs.as_deref(),
            note: shown.note.as_deref(),
        };
        if body.is_empty() {
            shown.rect = None;
            return;
        }
        let cache = &mut self.native_hover_markdown_cache;
        shown.rect = Some(popup::show(ui, &shown.spot, body, |ui, docs| {
            support::lsp_markup(ui, cache, (&docs.text, docs.markdown));
        }));
        quick_doc::note_on_screen(ui.ctx());
    }
}

/// What this frame's input means for the popup of `view`. Keys, text,
/// pastes and cuts count even when a widget used them up (raw events);
/// Copy does not, so copying popup text keeps the popup.
fn frame_input<'a>(
    ui: &egui::Ui,
    view: &'a str,
    now: Instant,
    hover: Option<HoverSpot>,
) -> Input<'a> {
    ui.input(|input| {
        let events = &input.raw.events;
        Input {
            now,
            view,
            spot: hover.map(|hover| Spot::pointer(view, hover)),
            pointer: input.pointer.hover_pos(),
            typed: events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Cut
                )
            }),
            pressed: input.pointer.press_origin(),
            scrolled: events
                .iter()
                .any(|event| matches!(event, egui::Event::MouseWheel { .. })),
            focused: input.focused,
        }
    })
}
