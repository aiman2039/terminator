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
pub(crate) struct FindBarInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) bar: &'a mut FindBar,
    pub(crate) buffer: &'a mut Doc,
    pub(crate) engine: &'a mut VimEngine,
    pub(crate) read_only: bool,
}

pub(crate) fn find_bar(input: FindBarInput<'_>) -> (FindOutcome, Arc<Vec<FindMatch>>) {
    let FindBarInput {
        ui,
        bar,
        buffer,
        engine,
        read_only,
    } = input;
    let mut outcome = FindOutcome::default();
    let mut go = ui
        .horizontal(|ui| bar.query_row(ui, buffer, &mut outcome))
        .inner;
    let request = if bar.replacing && !read_only {
        ui.horizontal(|ui| bar.replace_row(ui)).inner
    } else {
        ReplaceRequest::default()
    };
    if bar.apply_replace(buffer, engine, request) {
        go = Some(true);
    }
    let matches = bar.step_to_match(engine, buffer, go);
    (outcome, matches)
}

/// Replace buttons pressed this frame.
#[derive(Default, Clone, Copy)]
struct ReplaceRequest {
    one: bool,
    all: bool,
}

impl FindBar {
    /// Query field, toggles, count, and navigation. Returns a requested
    /// step: `Some(true)` forward, `Some(false)` back.
    fn query_row(
        &mut self,
        ui: &mut egui::Ui,
        buffer: &Doc,
        outcome: &mut FindOutcome,
    ) -> Option<bool> {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut go = self.query_field(ui, outcome);
        self.option_toggles(ui);
        self.match_count(ui, buffer);
        if let Some(forward) = self.nav_buttons(ui, outcome) {
            go = Some(forward);
        }
        go
    }

    fn query_field(&mut self, ui: &mut egui::Ui, outcome: &mut FindOutcome) -> Option<bool> {
        let query_id = egui::Id::unique(("native-find-query", &self.path));
        let field = ui.add(
            appearance::singleline(&mut self.query)
                .id(query_id)
                .hint_text("Find")
                .desired_width((ui.available_width() - 260.0).max(120.0)),
        );
        if self.focus {
            field.request_focus();
            self.focus = false;
        }
        if field.changed() {
            self.current = None;
        }
        if !(field.has_focus() || field.lost_focus()) {
            return None;
        }
        let mut go = None;
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
        go
    }

    fn option_toggles(&mut self, ui: &mut egui::Ui) {
        for (flag, icon, tip) in [
            (&mut self.case, "CaseSensitive", "Match case"),
            (&mut self.word, "WholeWord", "Words"),
            (&mut self.regex, "Regex", "Regex"),
        ] {
            if appearance::selectable_icon(ui, icon, tip, *flag).clicked() {
                *flag = !*flag;
                self.current = None;
            }
        }
    }

    fn match_count(&mut self, ui: &mut egui::Ui, buffer: &Doc) {
        let matches = self.matches(buffer);
        let count = if matches.len() >= MAX_MATCHES {
            format!("{}+", matches.len())
        } else {
            matches.len().to_string()
        };
        match (&self.error, self.current) {
            (Some(_), _) => {
                ui.colored_label(ui.visuals().error_fg_color, "Bad regex")
                    .on_hover_text(self.error.clone().unwrap_or_default());
            }
            (None, Some(index)) => {
                ui.weak(format!("{}/{count}", index.saturating_add(1)));
            }
            (None, None) if !self.query.is_empty() => {
                ui.weak(format!("{count} results"));
            }
            _ => {}
        }
    }

    /// Previous/next, the Replace toggle, and Close.
    fn nav_buttons(&mut self, ui: &mut egui::Ui, outcome: &mut FindOutcome) -> Option<bool> {
        let mut go = None;
        if appearance::sidebar_action(ui, "ArrowUp", "Previous occurrence (Shift+Enter)").clicked()
        {
            go = Some(false);
        }
        if appearance::sidebar_action(ui, "ArrowDown", "Next occurrence (Enter)").clicked() {
            go = Some(true);
        }
        if appearance::selectable_icon(ui, "Pencil", "Replace", self.replacing).clicked() {
            self.replacing = !self.replacing;
        }
        if appearance::sidebar_action(ui, "X", "Close (Esc)").clicked() {
            outcome.close = true;
        }
        go
    }

    fn replace_row(&mut self, ui: &mut egui::Ui) -> ReplaceRequest {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(
            appearance::singleline(&mut self.replacement)
                .hint_text(if self.regex {
                    "Replace ($1 for groups)"
                } else {
                    "Replace"
                })
                .desired_width((ui.available_width() - 200.0).max(120.0)),
        );
        let one = ui.button("Replace").clicked();
        let all = ui.button("Replace All").clicked();
        ReplaceRequest { one, all }
    }

    /// Run a Replace / Replace All. Returns true when the bar should then
    /// step to the next match (after Replace).
    fn apply_replace(
        &mut self,
        buffer: &mut Doc,
        engine: &mut VimEngine,
        request: ReplaceRequest,
    ) -> bool {
        let matches = self.matches(buffer);
        if request.all && !matches.is_empty() {
            if let Ok(pattern) = self.pattern() {
                replace_matches(ReplaceMatchesInput {
                    buffer,
                    engine,
                    pattern: &pattern,
                    replacement: &self.replacement,
                    matches: &matches,
                    regex: self.regex,
                });
            }
            self.current = None;
            return false;
        }
        if !request.one {
            return false;
        }
        // Replace the selected match, then move on to the next one.
        if let (Some(index), Ok(pattern)) = (self.current, self.pattern())
            && let Some(found) = matches.get(index).copied()
        {
            replace_matches(ReplaceMatchesInput {
                buffer,
                engine,
                pattern: &pattern,
                replacement: &self.replacement,
                matches: &[found],
                regex: self.regex,
            });
        }
        true
    }

    /// The current matches; with `go`, step from the selected match (or
    /// the caret) and select the result.
    fn step_to_match(
        &mut self,
        engine: &mut VimEngine,
        buffer: &mut Doc,
        go: Option<bool>,
    ) -> Arc<Vec<FindMatch>> {
        let matches = self.matches(buffer);
        let Some(forward) = go else {
            return matches;
        };
        let cursor = engine.cursor();
        let caret = match (self.current.and_then(|i| matches.get(i)), forward) {
            // From the selected match, step past it.
            (Some(current), true) => (current.line, current.end),
            (Some(current), false) => (current.line, current.start.saturating_add(1)),
            (None, _) => (cursor.line, cursor.col),
        };
        self.current = step(&matches, caret, forward);
        if let Some(found) = self.current.and_then(|i| matches.get(i)).copied() {
            select(engine, buffer, found);
        }
        matches
    }
}

/// Replace `matches` (sorted) with `replacement`, last first so earlier
/// positions stay valid, as one undo step. Regex mode expands `$1`-style
/// groups against each matched text.
pub(crate) struct ReplaceMatchesInput<'a> {
    pub(crate) buffer: &'a mut Doc,
    pub(crate) engine: &'a mut VimEngine,
    pub(crate) pattern: &'a regex::Regex,
    pub(crate) replacement: &'a str,
    pub(crate) matches: &'a [FindMatch],
    pub(crate) regex: bool,
}

fn replace_matches(input: ReplaceMatchesInput<'_>) {
    let ReplaceMatchesInput {
        buffer,
        engine,
        pattern,
        replacement,
        matches,
        regex,
    } = input;
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
        replace_matches(ReplaceMatchesInput {
            buffer: &mut doc,
            engine: &mut engine,
            pattern: &pattern,
            replacement: "const $1",
            matches: &matches,
            regex: true,
        });
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
