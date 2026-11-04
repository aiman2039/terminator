//! The header's file-format menu: line breaks and text encoding, shown
//! as the current values and converted on pick (saved by the next save).
use super::*;
use crate::native_editor::disk::{apply_edits, text_edits};
use crate::native_editor::file_io::{LineEnding, TextEncoding};
use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE};
use terminator_native_edit::doc::normalize_breaks;

/// Encodings offered for conversion, with their menu labels.
fn encodings() -> [(&'static str, TextEncoding); 4] {
    let form = |encoding, bom| TextEncoding { encoding, bom };
    [
        ("UTF-8", form(UTF_8, false)),
        ("UTF-8 with BOM", form(UTF_8, true)),
        ("UTF-16 LE", form(UTF_16LE, true)),
        ("UTF-16 BE", form(UTF_16BE, true)),
    ]
}

impl ViewPass<'_> {
    pub(super) fn header_format(&mut self, ui: &mut egui::Ui) {
        if self.doc.doc.is_none() || self.read_only() {
            return;
        }
        let title = format!(
            "{} · {}",
            self.doc.encoding.label(),
            self.doc.line_ending.label()
        );
        let mut picked: Option<Result<LineEnding, TextEncoding>> = None;
        appearance::text_menu_button(ui, &title, |ui| {
            for (label, ending) in [
                ("LF line breaks", LineEnding::Lf),
                ("CRLF line breaks", LineEnding::Crlf),
            ] {
                let current = self.doc.line_ending == ending;
                if ui.selectable_label(current, label).clicked() {
                    picked = Some(Ok(ending));
                    ui.close();
                }
            }
            ui.separator();
            for (label, encoding) in encodings() {
                if ui
                    .selectable_label(self.doc.encoding == encoding, label)
                    .clicked()
                {
                    picked = Some(Err(encoding));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Line breaks and encoding (saved with the next save)");
        match picked {
            Some(Ok(ending)) => self.convert_line_breaks(ending),
            Some(Err(encoding)) => self.doc.encoding = encoding,
            None => {}
        }
    }

    /// Make every break `\n` in the buffer (one undo step) and save them
    /// as `ending` (LF or CRLF).
    fn convert_line_breaks(&mut self, ending: LineEnding) {
        if let Some(buffer) = self.doc.doc.as_mut() {
            let text = buffer.text();
            if let std::borrow::Cow::Owned(normalized) = normalize_breaks(&text) {
                apply_edits(buffer, &text_edits(&text, &normalized));
            }
        }
        self.doc.line_ending = ending;
    }
}
