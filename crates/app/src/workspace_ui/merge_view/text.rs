//! Syntax-highlighted text for the merge columns.
use crate::diff::DiffSpan;
use crate::merge::{Chunk, MergeDoc, Side};
use eframe::egui::{self, Color32};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    ops::Range,
    path::Path,
    sync::Arc,
};

/// Hand-edited chunk text above this size paints plain: highlighting it
/// would run on the GUI thread.
const EDIT_HIGHLIGHT_BYTES: usize = 16 * 1024;

/// Cached highlights kept per merge tab before the cache is reset.
const EDIT_CACHE_ENTRIES: usize = 256;

pub(super) type SpanCache = HashMap<u64, Arc<Vec<Vec<DiffSpan>>>>;

/// Lines `text` occupies (a trailing line break opens no extra row).
pub(super) fn line_count(text: &str) -> usize {
    text.split_inclusive('\n').count()
}

/// Layout job whose text is exactly `text`; lines whose spans no longer
/// match (mid-edit) paint plain.
pub(super) fn job(
    text: &str,
    spans: &[Vec<DiffSpan>],
    font: egui::FontId,
    fallback: Color32,
) -> egui::text::LayoutJob {
    let format = |color: Color32| egui::TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let content = line.trim_end_matches(['\n', '\r']);
        let parts = spans.get(index).filter(|parts| {
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>()
                == content
        });
        match parts {
            Some(parts) if !content.is_empty() => {
                for part in parts {
                    let [r, g, b] = part.rgb;
                    job.append(&part.text, 0.0, format(Color32::from_rgb(r, g, b)));
                }
            }
            _ => job.append(content, 0.0, format(fallback)),
        }
        job.append(
            line.get(content.len()..).unwrap_or_default(),
            0.0,
            format(fallback),
        );
    }
    if text.is_empty() {
        job.append("", 0.0, format(fallback));
    }
    job
}

/// Spans of a side's line range.
pub(super) fn side_spans<'a>(side: &'a Side, lines: &Range<usize>) -> &'a [Vec<DiffSpan>] {
    side.spans.get(lines.clone()).unwrap_or_default()
}

/// Spans for a chunk's result: borrowed from the side it equals, or a
/// cached highlight of hand-edited text.
pub(super) enum ResultSpans<'a> {
    Side(&'a [Vec<DiffSpan>]),
    Edited(Arc<Vec<Vec<DiffSpan>>>),
}

impl ResultSpans<'_> {
    pub(super) fn as_slice(&self) -> &[Vec<DiffSpan>] {
        match self {
            Self::Side(spans) => spans,
            Self::Edited(spans) => spans.as_slice(),
        }
    }
}

pub(super) struct ResultSpansInput<'a> {
    pub(super) doc: &'a MergeDoc,
    pub(super) chunk: &'a Chunk,
    pub(super) cache: &'a mut SpanCache,
    pub(super) path: &'a Path,
}

pub(super) fn result_spans(input: ResultSpansInput<'_>) -> ResultSpans<'_> {
    let ResultSpansInput {
        doc,
        chunk,
        cache,
        path,
    } = input;
    for (side, lines) in [
        (&doc.ours, &chunk.ours),
        (&doc.theirs, &chunk.theirs),
        (&doc.base, &chunk.base),
    ] {
        if side.slice(lines) == chunk.result {
            return ResultSpans::Side(side_spans(side, lines));
        }
    }
    if chunk.result.len() > EDIT_HIGHLIGHT_BYTES {
        return ResultSpans::Side(&[]);
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    chunk.result.hash(&mut hasher);
    let hash = hasher.finish();
    if cache.len() > EDIT_CACHE_ENTRIES {
        cache.clear();
    }
    let spans = cache
        .entry(hash)
        .or_insert_with(|| Arc::new(crate::diff::highlight_spans(path, &chunk.result)));
    ResultSpans::Edited(Arc::clone(spans))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_text_is_the_buffer_even_with_stale_spans() {
        let text = "fn a() {}\r\nlet b = 1;\n\nend";
        let spans = crate::diff::highlight_spans(Path::new("x.rs"), "fn a() {}\nchanged\n");
        let font = egui::FontId::monospace(12.0);
        for spans in [&spans[..], &[]] {
            let job = job(text, spans, font.clone(), Color32::WHITE);
            assert_eq!(job.text, text);
        }
        assert_eq!(job("", &[], font, Color32::WHITE).text, "");
        assert_eq!(line_count("a\nb"), 2);
        assert_eq!(line_count("a\n"), 1);
        assert_eq!(line_count(""), 0);
    }
}
