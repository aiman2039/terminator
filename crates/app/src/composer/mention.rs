//! `@file` completion in the composer: the `@word` that ends at the caret,
//! and its replacement by a project-relative `@path` (the form "Add to
//! agent context" pastes, without a line range).
use std::path::Path;

/// The byte offset of the `@` and the typed query when the caret (a byte
/// offset) ends an `@word` that starts the text or follows white space.
#[must_use]
pub(crate) fn mention_at(text: &str, caret: usize) -> Option<(usize, String)> {
    let before = text.get(..caret)?;
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(at, c)| at.saturating_add(c.len_utf8()));
    let word = before.get(start..)?;
    let query = word.strip_prefix('@')?;
    Some((start, query.to_owned()))
}

/// `text` with the `@word` from `start` to `caret` replaced by `@path `,
/// the path relative to `root`; and the new caret.
#[must_use]
pub(crate) fn insert_mention(
    text: &str,
    (start, caret): (usize, usize),
    path: &Path,
    root: Option<&Path>,
) -> (String, usize) {
    let shown = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let mention = format!("@{} ", shown.display());
    let head = text.get(..start).unwrap_or(text);
    let tail = text.get(caret..).unwrap_or("");
    let caret = head.len().saturating_add(mention.len());
    (format!("{head}{mention}{tail}"), caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_at_word_before_the_caret_is_a_mention() {
        assert_eq!(mention_at("fix @src/ma", 11), Some((4, "src/ma".into())));
        assert_eq!(mention_at("@", 1), Some((0, String::new())));
        assert_eq!(mention_at("mail a@b", 8), None, "inside a word");
        assert_eq!(mention_at("fix @a b", 8), None, "caret after the word");
        assert_eq!(mention_at("é @x", 5), Some((3, "x".into())));
    }

    #[test]
    fn a_picked_file_replaces_the_query_with_a_project_relative_path() {
        let (text, caret) = insert_mention(
            "fix @ma now",
            (4, 7),
            Path::new("/w/src/main.rs"),
            Some(Path::new("/w")),
        );
        assert_eq!(text, "fix @src/main.rs  now");
        assert_eq!(text.get(..caret), Some("fix @src/main.rs "));
    }
}
