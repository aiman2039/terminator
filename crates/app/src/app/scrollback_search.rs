//! Scrollback search page. Matches are computed once per (history text,
//! query) and the rows are virtualized, so a repaint does no work that
//! grows with the scrollback (#173).

use std::ops::Range;

use super::super::appearance;
use eframe::egui::{self};
use terminator_core::*;

use super::super::*;

/// Matching lines for one history text and one query.
#[derive(Default)]
pub(crate) struct ScrollbackHits {
    key: String,
    text_len: usize,
    query: String,
    /// (0-based line number, byte range of the line in the text).
    rows: Vec<(usize, Range<usize>)>,
}

impl ScrollbackHits {
    /// Recomputes the matches only when the text or the query changed.
    pub(crate) fn refresh(&mut self, key: &str, text: &str, query: &str) {
        if self.key == key && self.text_len == text.len() && self.query == query {
            return;
        }
        key.clone_into(&mut self.key);
        self.text_len = text.len();
        query.clone_into(&mut self.query);
        self.rows = matching_lines(text, query);
    }
}

/// Lines that contain `query`, ignoring case.
pub(crate) fn matching_lines(text: &str, query: &str) -> Vec<(usize, Range<usize>)> {
    let needle = query.to_lowercase();
    let mut rows = Vec::new();
    let mut start: usize = 0;
    // Same line split as `str::lines`, but keeping byte offsets.
    for (index, piece) in text.split_inclusive('\n').enumerate() {
        let line = piece.strip_suffix('\n').unwrap_or(piece);
        let body = line.strip_suffix('\r').unwrap_or(line);
        if needle.is_empty() || body.to_lowercase().contains(&needle) {
            rows.push((index, start..start.saturating_add(body.len())));
        }
        start = start.saturating_add(piece.len());
    }
    rows
}

impl App {
    pub(crate) fn search_history_center(&mut self, ui: &mut egui::Ui) {
        let Some(sid) = self.search_session().map(str::to_owned) else {
            self.close_scrollback_search();
            return;
        };
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Search session history");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.close_scrollback_search();
                }
            });
        });
        ui.add_space(6.0);
        ui.add(
            appearance::singleline(&mut self.search)
                .id(egui::Id::unique("scrollback-search"))
                .hint_text("Search scrollback…")
                .desired_width(400.0),
        );
        ui.add_space(8.0);
        let key = format!("history:{sid}");
        if !self.texts.contains_key(&key) && self.loading.insert(key.clone()) {
            let _ = self.jobs.send(Job::rpc(
                Request::History {
                    session: sid.clone(),
                },
                After::Text(key.clone()),
            ));
        }
        let Some(text) = self.texts.get(&key) else {
            ui.weak("Loading scrollback…");
            return;
        };
        self.scrollback_hits.refresh(&key, text, &self.search);
        let rows = &self.scrollback_hits.rows;
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both().auto_shrink(false).show_rows(
            ui,
            row_height,
            rows.len(),
            |ui, range| {
                for (line, bytes) in rows.get(range).unwrap_or_default() {
                    let body = text.get(bytes.clone()).unwrap_or_default();
                    ui.monospace(format!("{}  {}", line.saturating_add(1), body));
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, query: &str) -> Vec<(usize, String)> {
        matching_lines(text, query)
            .into_iter()
            .map(|(line, bytes)| (line, text.get(bytes).unwrap_or_default().to_owned()))
            .collect()
    }

    #[test]
    fn scrollback_search_matches_lines_ignoring_case() {
        let text = "alpha\r\nBeta one\ngamma\nbeta two\n";
        assert_eq!(
            lines(text, "BETA"),
            vec![(1, "Beta one".to_owned()), (3, "beta two".to_owned())]
        );
    }

    #[test]
    fn empty_scrollback_query_lists_every_line_like_str_lines() {
        let text = "a\nb\n";
        let all: Vec<_> = lines(text, "").into_iter().map(|(_, l)| l).collect();
        assert_eq!(all, text.lines().collect::<Vec<_>>());
    }

    #[test]
    fn scrollback_hits_recompute_only_when_text_or_query_changes() {
        let mut hits = ScrollbackHits::default();
        hits.refresh("history:a", "foo\nbar", "foo");
        assert_eq!(hits.rows.len(), 1);
        hits.rows.clear();
        hits.refresh("history:a", "foo\nbar", "foo");
        assert!(hits.rows.is_empty(), "same text and query reuse the cache");
        hits.refresh("history:a", "foo\nbar", "ba");
        assert_eq!(hits.rows.len(), 1);
        hits.refresh("history:a", "foo\nbar\nfoo", "ba");
        assert_eq!(hits.rows.len(), 1);
        hits.refresh("history:a", "foo\nbar\nfoo", "foo");
        assert_eq!(hits.rows.len(), 2);
    }
}
