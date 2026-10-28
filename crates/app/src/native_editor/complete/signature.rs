//! Parameter info above the caret while typing call arguments.
use super::*;

/// Parameter info for the call around the caret.
pub(crate) struct SignaturePopup {
    pub path: PathBuf,
    pub generation: u64,
    pub help: Option<native_lsp::SignatureHelp>,
    pub line: usize,
    /// Column just after the `(` that opened the call.
    pub open: usize,
    pub anchor: Option<egui::Rect>,
}

/// Column after the innermost unclosed `(` before `col` on this line.
pub(crate) fn open_paren(line: &str, col: usize) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 0usize;
    for index in (0..col.min(chars.len())).rev() {
        match chars.get(index) {
            Some(')') => depth = depth.saturating_add(1),
            Some('(') if depth == 0 => return Some(index.saturating_add(1)),
            Some('(') => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Signature upkeep after the view drew: close when the caret leaves the
/// call, and say when to ask (typing `(` or `,` inside a call).
pub(crate) struct SignatureAfterFrameInput<'a> {
    pub(crate) popup: &'a mut Option<SignaturePopup>,
    pub(crate) buffer: &'a Doc,
    pub(crate) engine: &'a VimEngine,
    pub(crate) revision_before: u64,
    pub(crate) server: bool,
}

pub(crate) fn signature_after_frame(
    input: SignatureAfterFrameInput<'_>,
) -> Option<(native_lsp::Position, usize, usize)> {
    let SignatureAfterFrameInput {
        popup,
        buffer,
        engine,
        revision_before,
        server,
    } = input;
    use terminator_native_edit::vim::{ModalEngine as _, Mode};
    let caret = engine.cursor();
    let text = buffer.line_text(caret.line);
    let open = open_paren(&text, caret.col);
    if let Some(shown) = popup.as_ref()
        && (shown.line != caret.line || open != Some(shown.open) || engine.mode() != Mode::Insert)
    {
        *popup = None;
    }
    let typed = buffer.revision() != revision_before
        && engine.mode() == Mode::Insert
        && caret
            .col
            .checked_sub(1)
            .and_then(|index| text.chars().nth(index))
            .is_some_and(|c| c == '(' || c == ',');
    let open = open?;
    if !(server && typed) {
        return None;
    }
    let position = native_lsp::Position {
        line: u32::try_from(caret.line).ok()?,
        character: native_lsp::char_col_to_utf16(&text, caret.col),
    };
    Some((position, caret.line, open))
}

/// Paint parameter info above the caret, the active parameter in bold.
pub(crate) fn signature_ui(ui: &egui::Ui, popup: &SignaturePopup) {
    let (Some(help), Some(anchor)) = (&popup.help, popup.anchor) else {
        return;
    };
    let font = egui::FontId::monospace(12.0);
    let mut job = egui::text::LayoutJob::default();
    let chars: Vec<char> = help.label.chars().collect();
    let (start, end) = help.active.as_ref().map_or((0, 0), |range| {
        (range.start.min(chars.len()), range.end.min(chars.len()))
    });
    let plain = ui.visuals().text_color();
    let strong = ui.visuals().strong_text_color();
    for (range, color, underline) in [
        (0..start, plain, false),
        (start..end, strong, true),
        (end..chars.len(), plain, false),
    ] {
        let text: String = chars.get(range).unwrap_or_default().iter().collect();
        if text.is_empty() {
            continue;
        }
        job.append(
            &text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                underline: if underline {
                    egui::Stroke::new(1.0, strong)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            },
        );
    }
    egui::Area::new(egui::Id::unique(("native-signature", &popup.path)))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(anchor.left_top())
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(640.0);
                ui.label(job);
                if let Some(doc) = &help.documentation {
                    let first = doc
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or_default();
                    ui.weak(first);
                }
            });
        });
}
