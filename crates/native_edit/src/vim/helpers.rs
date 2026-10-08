use super::super::doc::Buffer;
pub(crate) fn doc_is_empty<B: Buffer>(doc: &B) -> bool {
    doc.len_chars() == 0
}

pub(crate) fn range_text<B: Buffer>(doc: &B, from: usize, to: usize) -> String {
    let (from, to) = (from.min(to), from.max(to).min(doc.len_chars()));
    // Reconstruct from lines to avoid another full-text accessor on the trait.
    let mut out = String::new();
    let mut idx = 0;
    for line in 0..doc.line_count() {
        if line > 0 {
            if idx >= from && idx < to {
                out.push('\n');
            }
            idx = idx.checked_add(1).unwrap_or(idx);
        }
        for c in doc.line_text(line).chars() {
            if idx >= from && idx < to {
                out.push(c);
            }
            idx = idx.checked_add(1).unwrap_or(idx);
            if idx >= to {
                return out;
            }
        }
    }
    out
}
