//! Compare and highlight options in the diff gear menu. They persist in the
//! UI preferences and apply to every native diff.
use super::super::super::*;
use super::DiffPass;
use diff::{Highlight, Whitespace};

const WHITESPACE: [(Whitespace, &str, &str); 3] = [
    (
        Whitespace::None,
        "Do not ignore whitespace",
        "Every whitespace change is a change",
    ),
    (
        Whitespace::Trim,
        "Trim whitespace",
        "Ignore whitespace at the start and end of lines",
    ),
    (
        Whitespace::All,
        "Ignore all whitespace",
        "Ignore every whitespace change",
    ),
];

const HIGHLIGHT: [(Highlight, &str, &str); 4] = [
    (
        Highlight::Words,
        "Highlight words",
        "Tint changed words inside changed lines",
    ),
    (
        Highlight::Chars,
        "Highlight characters",
        "Tint changed characters inside changed lines",
    ),
    (
        Highlight::Lines,
        "Highlight lines",
        "Tint changed lines only",
    ),
    (
        Highlight::None,
        "Do not highlight",
        "No change tint; signs and connectors still show changes",
    ),
];

impl App {
    /// Whitespace mode, ignore case, and highlight granularity. A change
    /// rebuilds this diff; other open diffs follow when shown.
    pub(super) fn diff_options_menu(&mut self, ui: &mut egui::Ui, pass: &mut DiffPass) {
        let before = self.preferences.diff_options;
        let options = &mut self.preferences.diff_options;
        ui.weak("Compare");
        for (value, label, tip) in WHITESPACE {
            ui.radio_value(&mut options.whitespace, value, label)
                .on_hover_text(tip);
        }
        ui.checkbox(&mut options.ignore_case, "Ignore case")
            .on_hover_text("Compare lines without letter case");
        ui.separator();
        ui.weak("Highlight");
        for (value, label, tip) in HIGHLIGHT {
            ui.radio_value(&mut options.highlight, value, label)
                .on_hover_text(tip);
        }
        if *options != before {
            // Staging and discarding still apply the real lines.
            pass.refresh = true;
            ui.close();
        }
    }
}
