//! Find / replace bar for native editors (`IntelliJ` Cmd+F / Cmd+R): case,
//! whole-word, and regex toggles, a live match count, all matches
//! highlighted in the view, next/previous, and replace one or all (one
//! undo step).

use super::*;
use std::sync::Arc;

/// Matches counted and highlighted per search; more are reported as "+".
const MAX_MATCHES: usize = 10_000;

/// One match on one line, in char columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FindMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

pub(crate) struct FindBar {
    pub path: PathBuf,
    pub query: String,
    pub replacement: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
    pub replacing: bool,
    /// Give the query field focus on the next paint.
    pub focus: bool,
    /// Index of the selected match, when one is selected.
    pub current: Option<usize>,
    /// `(buffer revision, query key)` the cached matches belong to.
    cache: Option<(u64, String, Arc<Vec<FindMatch>>)>,
    pub error: Option<String>,
    /// Render pass that last drew the bar (one view draws it per pass).
    pub drawn: Option<u64>,
}

/// What the bar asks of the editor after painting.
#[derive(Default)]
pub(crate) struct FindOutcome {
    pub close: bool,
}

impl FindBar {
    pub fn new(path: PathBuf, query: String, replacing: bool) -> Self {
        Self {
            path,
            query,
            replacement: String::new(),
            case: false,
            word: false,
            regex: false,
            replacing,
            focus: true,
            current: None,
            cache: None,
            error: None,
            drawn: None,
        }
    }

    fn key(&self) -> String {
        format!("{}|{}|{}|{}", self.case, self.word, self.regex, self.query)
    }

    fn pattern(&self) -> Result<regex::Regex, String> {
        let body = if self.regex {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        let body = if self.word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        regex::RegexBuilder::new(&body)
            .case_insensitive(!self.case)
            .size_limit(1 << 20)
            .build()
            .map_err(|error| error.to_string())
    }

    /// Matches for the current buffer revision (cached).
    pub fn matches(&mut self, buffer: &Doc) -> Arc<Vec<FindMatch>> {
        let key = self.key();
        if let Some((revision, cached, matches)) = &self.cache
            && *revision == buffer.revision()
            && *cached == key
        {
            return Arc::clone(matches);
        }
        let matches = Arc::new(if self.query.is_empty() {
            self.error = None;
            Vec::new()
        } else {
            match self.pattern() {
                Ok(pattern) => {
                    self.error = None;
                    find_all(buffer, &pattern)
                }
                Err(error) => {
                    self.error = Some(error);
                    Vec::new()
                }
            }
        });
        self.cache = Some((buffer.revision(), key, Arc::clone(&matches)));
        matches
    }
}

/// Every match of `pattern`, line by line, in char columns. Empty matches
/// are skipped (they would select nothing).
pub(crate) fn find_all(buffer: &Doc, pattern: &regex::Regex) -> Vec<FindMatch> {
    let mut out = Vec::new();
    for line in 0..buffer.line_count() {
        let text = buffer.line_text(line);
        for found in pattern.find_iter(&text) {
            if found.start() == found.end() {
                continue;
            }
            let start = text
                .get(..found.start())
                .map_or(0, |head| head.chars().count());
            let len = found.as_str().chars().count();
            out.push(FindMatch {
                line,
                start,
                end: start.saturating_add(len),
            });
            if out.len() >= MAX_MATCHES {
                return out;
            }
        }
    }
    out
}

/// Select match `index` in the editor (caret at its end, IDE-style).
fn select(engine: &mut VimEngine, buffer: &mut Doc, found: FindMatch) {
    let _ = engine.place_cursor(buffer, found.line, found.start);
    engine.extend_selection_to(buffer, found.line, found.end);
}

/// The match after (or before) the caret, wrapping around.
fn step(matches: &[FindMatch], caret: (usize, usize), forward: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let found = if forward {
        matches
            .iter()
            .position(|m| (m.line, m.start) >= caret)
            .unwrap_or(0)
    } else {
        matches
            .iter()
            .rposition(|m| (m.line, m.end) < caret)
            .unwrap_or(matches.len().saturating_sub(1))
    };
    Some(found)
}

/// Paint the bar and act on it. Returns the matches to highlight.
pub(crate) fn find_bar(
    ui: &mut egui::Ui,
    bar: &mut FindBar,
    buffer: &mut Doc,
    engine: &mut VimEngine,
    read_only: bool,
) -> (FindOutcome, Arc<Vec<FindMatch>>) {
    let mut outcome = FindOutcome::default();
    let mut go: Option<bool> = None;
    let mut replace_one = false;
    let mut replace_all = false;
    let query_id = egui::Id::unique(("native-find-query", &bar.path));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let field = ui.add(
            appearance::singleline(&mut bar.query)
                .id(query_id)
                .hint_text("Find")
                .desired_width((ui.available_width() - 260.0).max(120.0)),
        );
        if bar.focus {
            field.request_focus();
            bar.focus = false;
        }
        if field.changed() {
            bar.current = None;
        }
        if field.has_focus() || field.lost_focus() {
            ui.input(|input| {
                if input.key_pressed(egui::Key::Enter) {
                    go = Some(!input.modifiers.shift);
                }
                if input.key_pressed(egui::Key::Escape) {
                    outcome.close = true;
                }
            });
            if go.is_some() {
                field.request_focus();
            }
        }
        for (flag, icon, tip) in [
            (&mut bar.case, "CaseSensitive", "Match case"),
            (&mut bar.word, "WholeWord", "Words"),
            (&mut bar.regex, "Regex", "Regex"),
        ] {
            if appearance::selectable_icon(ui, icon, tip, *flag).clicked() {
                *flag = !*flag;
                bar.current = None;
            }
        }
        let matches = bar.matches(buffer);
        let count = if matches.len() >= MAX_MATCHES {
            format!("{}+", matches.len())
        } else {
            matches.len().to_string()
        };
        match (&bar.error, bar.current) {
            (Some(_), _) => {
                ui.colored_label(ui.visuals().error_fg_color, "Bad regex")
                    .on_hover_text(bar.error.clone().unwrap_or_default());
            }
            (None, Some(index)) => {
                ui.weak(format!("{}/{count}", index.saturating_add(1)));
            }
            (None, None) if !bar.query.is_empty() => {
                ui.weak(format!("{count} results"));
            }
            _ => {}
        }
        if appearance::sidebar_action(ui, "ArrowUp", "Previous occurrence (Shift+Enter)").clicked()
        {
            go = Some(false);
        }
        if appearance::sidebar_action(ui, "ArrowDown", "Next occurrence (Enter)").clicked() {
            go = Some(true);
        }
        if appearance::selectable_icon(ui, "Pencil", "Replace", bar.replacing).clicked() {
            bar.replacing = !bar.replacing;
        }
        if appearance::sidebar_action(ui, "X", "Close (Esc)").clicked() {
            outcome.close = true;
        }
    });
    if bar.replacing && !read_only {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(
                appearance::singleline(&mut bar.replacement)
                    .hint_text(if bar.regex {
                        "Replace ($1 for groups)"
                    } else {
                        "Replace"
                    })
                    .desired_width((ui.available_width() - 200.0).max(120.0)),
            );
            replace_one = ui.button("Replace").clicked();
            replace_all = ui.button("Replace All").clicked();
        });
    }
    let matches = bar.matches(buffer);
    if replace_all && !matches.is_empty() {
        if let Ok(pattern) = bar.pattern() {
            replace_matches(
                buffer,
                engine,
                &pattern,
                &bar.replacement,
                &matches,
                bar.regex,
            );
        }
        bar.current = None;
    } else if replace_one {
        // Replace the selected match, then move on to the next one.
        if let (Some(index), Ok(pattern)) = (bar.current, bar.pattern())
            && let Some(found) = matches.get(index).copied()
        {
            replace_matches(
                buffer,
                engine,
                &pattern,
                &bar.replacement,
                &[found],
                bar.regex,
            );
            go = Some(true);
        } else {
            go = Some(true);
        }
    }
    let matches = bar.matches(buffer);
    if let Some(forward) = go {
        let cursor = engine.cursor();
        let caret = match (bar.current.and_then(|i| matches.get(i)), forward) {
            // From the selected match, step past it.
            (Some(current), true) => (current.line, current.end),
            (Some(current), false) => (current.line, current.start.saturating_add(1)),
            (None, _) => (cursor.line, cursor.col),
        };
        bar.current = step(&matches, caret, forward);
        if let Some(found) = bar.current.and_then(|i| matches.get(i)).copied() {
            select(engine, buffer, found);
        }
    }
    (outcome, matches)
}

/// Replace `matches` (sorted) with `replacement`, last first so earlier
/// positions stay valid, as one undo step. Regex mode expands `$1`-style
/// groups against each matched text.
fn replace_matches(
    buffer: &mut Doc,
    engine: &mut VimEngine,
    pattern: &regex::Regex,
    replacement: &str,
    matches: &[FindMatch],
    regex: bool,
) {
    buffer.begin_undo_group();
    for found in matches.iter().rev() {
        let from = buffer.char_at_line_col(found.line, found.start);
        let to = buffer.char_at_line_col(found.line, found.end);
        let line = buffer.line_text(found.line);
        let matched: String = line
            .chars()
            .skip(found.start)
            .take(found.end.saturating_sub(found.start))
            .collect();
        let text = if regex {
            pattern.replace(&matched, replacement).into_owned()
        } else {
            replacement.to_owned()
        };
        buffer.delete(from..to);
        buffer.insert(from, &text);
    }
    buffer.end_undo_group();
    if let Some(first) = matches.first() {
        let _ = engine.place_cursor(buffer, first.line, first.start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(query: &str) -> FindBar {
        FindBar::new(PathBuf::from("/f.rs"), query.into(), false)
    }

    #[test]
    fn find_honors_case_word_and_regex() {
        let doc = Doc::new("Foo foo food\nfoo_bar FOO\n");
        let mut find = bar("foo");
        assert_eq!(find.matches(&doc).len(), 5);
        find.case = true;
        assert_eq!(find.matches(&doc).len(), 3);
        find.case = false;
        find.word = true;
        assert_eq!(find.matches(&doc).len(), 3);
        find.word = false;
        find.regex = true;
        find.query = "fo+d".into();
        assert_eq!(
            find.matches(&doc).as_slice(),
            [FindMatch {
                line: 0,
                start: 8,
                end: 12
            }]
        );
        find.query = "(".into();
        assert!(find.matches(&doc).is_empty());
        assert!(find.error.is_some());
    }

    #[test]
    fn replace_all_is_one_undo_step_and_expands_groups() {
        let mut doc = Doc::new("let a = 1;\nlet b = 2;\n");
        let mut engine = VimEngine::new();
        let mut find = bar(r"let (\w)");
        find.regex = true;
        let matches = find.matches(&doc);
        let pattern = find.pattern().unwrap();
        replace_matches(&mut doc, &mut engine, &pattern, "const $1", &matches, true);
        assert_eq!(doc.text(), "const a = 1;\nconst b = 2;\n");
        engine.undo(&mut doc);
        assert_eq!(doc.text(), "let a = 1;\nlet b = 2;\n");
    }

    #[test]
    fn stepping_wraps_around_the_caret() {
        let matches = [
            FindMatch {
                line: 0,
                start: 0,
                end: 1,
            },
            FindMatch {
                line: 2,
                start: 4,
                end: 5,
            },
        ];
        assert_eq!(step(&matches, (1, 0), true), Some(1));
        assert_eq!(step(&matches, (3, 0), true), Some(0));
        assert_eq!(step(&matches, (1, 0), false), Some(0));
        assert_eq!(step(&matches, (0, 0), false), Some(1));
    }
}
