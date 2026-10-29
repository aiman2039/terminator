//! Compare options (whitespace, case) and in-line change granularity.
use serde::{Deserialize, Serialize};
use similar::{Algorithm, DiffTag};

/// How whitespace takes part in line comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Whitespace {
    /// Every whitespace character counts.
    #[default]
    None,
    /// Leading and trailing whitespace is ignored.
    Trim,
    /// All whitespace is ignored.
    All,
}

/// What a changed line highlights inside its row tint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Highlight {
    /// Changed words get a stronger tint.
    #[default]
    Words,
    /// Changed characters get a stronger tint.
    Chars,
    /// Only the changed lines are tinted.
    Lines,
    /// No change tint; the gutter signs and connectors still show changes.
    None,
}

/// Diff view options, persisted in UI preferences and applied to every
/// native diff. Display always keeps the real text; staging always applies
/// the real hunks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffOptions {
    pub whitespace: Whitespace,
    pub ignore_case: bool,
    pub highlight: Highlight,
}

impl DiffOptions {
    /// Lines compare as written: no text normalization is needed.
    pub(crate) fn exact(self) -> bool {
        self.whitespace == Whitespace::None && !self.ignore_case
    }

    /// One line (without its line break) as the comparison sees it.
    pub(crate) fn normalize_line(self, line: &str) -> String {
        let body = match self.whitespace {
            Whitespace::None => line.to_owned(),
            Whitespace::Trim => line.trim().to_owned(),
            Whitespace::All => line.chars().filter(|c| !c.is_whitespace()).collect(),
        };
        if self.ignore_case {
            body.to_lowercase()
        } else {
            body
        }
    }

    /// `text` normalized line by line; line breaks are kept so every line
    /// index matches the original.
    pub(crate) fn normalize(self, text: &str) -> String {
        text.split_inclusive('\n')
            .map(|line| {
                let body = line.trim_end_matches(['\n', '\r']);
                let mut out = self.normalize_line(body);
                if line.ends_with('\n') {
                    out.push('\n');
                }
                out
            })
            .collect()
    }

    /// Changed fragments of a paired old/new line, as `(changed, text)` runs
    /// that join back to each original line. `None` when the highlight
    /// mode marks whole lines (or nothing).
    pub(crate) fn intra(self, old: &str, new: &str) -> Option<Fragments> {
        let (old_tokens, new_tokens) = match self.highlight {
            Highlight::Words => (words(old), words(new)),
            Highlight::Chars => (chars(old), chars(new)),
            Highlight::Lines | Highlight::None => return None,
        };
        let key = |token: &&str| -> String {
            if self.whitespace != Whitespace::None && token.trim().is_empty() {
                // Whitespace runs compare equal to each other.
                return " ".to_owned();
            }
            if self.ignore_case {
                token.to_lowercase()
            } else {
                (*token).to_owned()
            }
        };
        let old_keys: Vec<String> = old_tokens.iter().map(key).collect();
        let new_keys: Vec<String> = new_tokens.iter().map(key).collect();
        let ops = similar::capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys);
        let mut out = Fragments::default();
        for op in ops {
            let changed = op.tag() != DiffTag::Equal;
            push_run(
                &mut out.old,
                old_tokens.get(op.old_range()).unwrap_or_default(),
                changed,
                self,
            );
            push_run(
                &mut out.new,
                new_tokens.get(op.new_range()).unwrap_or_default(),
                changed,
                self,
            );
        }
        Some(out)
    }
}

/// `(changed, text)` runs for each side of a paired line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fragments {
    pub old: Vec<(bool, String)>,
    pub new: Vec<(bool, String)>,
}

/// Append tokens as one run. Ignored whitespace never counts as changed.
fn push_run(out: &mut Vec<(bool, String)>, tokens: &[&str], changed: bool, options: DiffOptions) {
    let text: String = tokens.concat();
    if text.is_empty() {
        return;
    }
    let changed = changed && !(options.whitespace != Whitespace::None && text.trim().is_empty());
    match out.last_mut() {
        Some((last, run)) if *last == changed => run.push_str(&text),
        _ => out.push((changed, text)),
    }
}

/// Identifier runs, whitespace runs, and single other characters.
fn words(text: &str) -> Vec<&str> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut current: Option<Class> = None;
    for (index, c) in text.char_indices() {
        let next = class(c);
        let joins = current
            .as_ref()
            .is_some_and(|current| *current == next && next != Class::Other);
        if !joins && index > start {
            out.extend(text.get(start..index));
            start = index;
        }
        current = Some(next);
    }
    if start < text.len() {
        out.extend(text.get(start..));
    }
    out
}

fn chars(text: &str) -> Vec<&str> {
    text.char_indices()
        .filter_map(|(index, c)| text.get(index..index.saturating_add(c.len_utf8())))
        .collect()
}
