//! Case operators: `gu`, `gU`, `g~`, `~` and Visual `u`/`U`/`~`.
use super::super::mode::PendingOp;

/// `text` lowered, uppered or with each letter's case toggled.
pub(crate) fn convert(text: &str, op: PendingOp) -> String {
    match op {
        PendingOp::Lower => text.to_lowercase(),
        PendingOp::Upper => text.to_uppercase(),
        _ => text
            .chars()
            .flat_map(|c| {
                let flipped: Vec<char> = if c.is_uppercase() {
                    c.to_lowercase().collect()
                } else {
                    c.to_uppercase().collect()
                };
                flipped
            })
            .collect(),
    }
}
