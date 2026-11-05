//! LSP snippet syntax (`insertTextFormat: 2`): tab stops, placeholders,
//! choices, variables and escapes, expanded to plain text plus the char
//! range of every stop.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Placeholders nest at most this deep; a deeper `$` stays text.
const MAX_DEPTH: usize = 32;

/// Nodes one expansion writes at most, so copies of copies stay small.
const MAX_NODES: usize = 100_000;

/// Chars one expansion writes at most, so many copies of a long
/// placeholder stay small too.
const MAX_CHARS: usize = 1 << 20;

/// A snippet expanded to the text it inserts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// Every stop as a char range of `text`. One index can occur more
    /// than once: its copies mirror each other.
    pub stops: Vec<SnippetStop>,
}

/// One tab stop: `$1` has index 1; `$0`, the final caret spot, has 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnippetStop {
    pub index: u32,
    pub start: usize,
    pub end: usize,
}

impl Snippet {
    /// Plain text without stops (an item that is not a snippet).
    #[must_use]
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            stops: Vec::new(),
        }
    }

    /// Where the caret goes when no stop session runs: the start of the
    /// lowest numbered stop, else `$0`, else the end.
    #[must_use]
    pub fn caret(&self) -> usize {
        let numbered = self.stops.iter().filter(|stop| stop.index > 0);
        numbered
            .min_by_key(|stop| (stop.index, stop.start))
            .or_else(|| self.final_stop())
            .map_or_else(|| self.text.chars().count(), |stop| stop.start)
    }

    /// The first `$0`.
    #[must_use]
    pub fn final_stop(&self) -> Option<&SnippetStop> {
        self.stops
            .iter()
            .filter(|stop| stop.index == 0)
            .min_by_key(|stop| stop.start)
    }
}

/// What snippet variables expand to where the snippet goes.
#[derive(Clone, Debug, Default)]
pub struct SnippetVars {
    /// The file: `TM_FILENAME`, `TM_FILENAME_BASE`, `TM_DIRECTORY` and
    /// `TM_FILEPATH`.
    pub path: Option<PathBuf>,
    /// Zero-based caret line: `TM_LINE_INDEX` and `TM_LINE_NUMBER`.
    pub line: usize,
    /// `TM_CURRENT_LINE`.
    pub line_text: String,
    /// `TM_CURRENT_WORD`.
    pub word: String,
    /// `TM_SELECTED_TEXT`.
    pub selected: String,
}

impl SnippetVars {
    /// The value of variable `name`; `None` when it is unknown or empty.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<String> {
        let path = self.path.as_deref();
        let text = |part: &std::ffi::OsStr| part.to_string_lossy().into_owned();
        let value = match name {
            "TM_SELECTED_TEXT" => self.selected.clone(),
            "TM_CURRENT_LINE" => self.line_text.clone(),
            "TM_CURRENT_WORD" => self.word.clone(),
            "TM_LINE_INDEX" => self.line.to_string(),
            "TM_LINE_NUMBER" => self.line.saturating_add(1).to_string(),
            "TM_FILENAME" => text(path?.file_name()?),
            "TM_FILENAME_BASE" => text(path?.file_stem()?),
            "TM_DIRECTORY" => text(path?.parent()?.as_os_str()),
            "TM_FILEPATH" => text(path?.as_os_str()),
            _ => return None,
        };
        (!value.is_empty()).then_some(value)
    }
}

/// Expand LSP snippet syntax. `$1`, `${1}` and `${1:text}` are stops
/// (placeholders nest); every copy of a stop shows the first placeholder
/// text of its index. `${1|a,b|}` inserts its first choice, and `$0` is
/// the final stop. A variable (`$TM_FILENAME`, `${NAME:default}`) gives
/// its value, else its default, else nothing; a transform after it is
/// not applied. `\$`, `\}` and `\\` are literal, and a `$` that starts
/// nothing stays text.
#[must_use]
pub fn parse_snippet(snippet: &str, vars: &SnippetVars) -> Snippet {
    let mut parser = Parser {
        chars: snippet.chars().collect(),
        at: 0,
        depth: 0,
        failed: HashSet::new(),
    };
    let nodes = parser.nodes(false);
    let mut defaults = HashMap::new();
    collect_defaults(&nodes, &mut defaults);
    let mut out = Expand {
        vars,
        defaults,
        text: String::new(),
        len: 0,
        stops: Vec::new(),
        filling: Vec::new(),
        budget: MAX_NODES,
    };
    out.nodes(&nodes);
    Snippet {
        text: out.text,
        stops: out.stops,
    }
}

/// The snippet as a tree.
enum Node {
    Text(String),
    /// A stop and its placeholder (empty for `$1`).
    Stop(u32, Vec<Node>),
    /// A variable and its default.
    Var(String, Vec<Node>),
}

/// Per index, the first placeholder that has text (`$0` has none).
fn collect_defaults<'a>(nodes: &'a [Node], defaults: &mut HashMap<u32, &'a [Node]>) {
    for node in nodes {
        match node {
            Node::Text(_) => {}
            Node::Stop(index, inner) => {
                if *index > 0 && !inner.is_empty() {
                    defaults.entry(*index).or_insert(inner.as_slice());
                }
                collect_defaults(inner, defaults);
            }
            Node::Var(_, default) => collect_defaults(default, defaults),
        }
    }
}

/// Writes the text and records where each stop lands.
struct Expand<'a> {
    vars: &'a SnippetVars,
    defaults: HashMap<u32, &'a [Node]>,
    text: String,
    /// Chars in `text`.
    len: usize,
    stops: Vec<SnippetStop>,
    /// Indices whose shared placeholder is being written: a copy inside
    /// it keeps its own text, so the expansion ends.
    filling: Vec<u32>,
    budget: usize,
}

impl Expand<'_> {
    fn nodes(&mut self, nodes: &[Node]) {
        for node in nodes {
            let Some(budget) = self.budget.checked_sub(1) else {
                return;
            };
            self.budget = budget;
            match node {
                Node::Text(text) => self.push(text),
                Node::Stop(index, inner) => self.stop(*index, inner),
                Node::Var(name, default) => match self.vars.resolve(name) {
                    Some(value) => self.push(&value),
                    None => self.nodes(default),
                },
            }
        }
    }

    /// A stop writes its index's shared placeholder, except inside that
    /// placeholder or [`MAX_DEPTH`] placeholders deep (a chain of copies
    /// could recurse past the stack): then its own text.
    fn stop(&mut self, index: u32, inner: &[Node]) {
        let start = self.len;
        let shared = self.defaults.get(&index).copied();
        let nested = self.filling.len() < MAX_DEPTH && !self.filling.contains(&index);
        match shared.filter(|_| nested) {
            Some(shared) => {
                self.filling.push(index);
                self.nodes(shared);
                self.filling.pop();
            }
            None => self.nodes(inner),
        }
        self.stops.push(SnippetStop {
            index,
            start,
            end: self.len,
        });
    }

    /// Write `text`. Past [`MAX_CHARS`] the expansion ends: the rest of the
    /// snippet is left out.
    fn push(&mut self, text: &str) {
        let len = self.len.saturating_add(text.chars().count());
        if len > MAX_CHARS {
            self.budget = 0;
            return;
        }
        self.text.push_str(text);
        self.len = len;
    }
}

/// Recursive descent over the snippet's chars.
struct Parser {
    chars: Vec<char>,
    at: usize,
    depth: usize,
    /// `$` positions that start nothing: a retry treats them as text.
    failed: HashSet<usize>,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) {
        self.at = self.at.saturating_add(1);
    }

    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        if hit {
            self.bump();
        }
        hit
    }

    /// Nodes up to the end, or (`nested`) up to the `}` that closes a
    /// placeholder, which stays unread.
    fn nodes(&mut self, nested: bool) -> Vec<Node> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        while let Some(c) = self.peek() {
            match c {
                '}' if nested => break,
                '$' => match self.dollar() {
                    Some(node) => {
                        flush(&mut nodes, &mut text);
                        nodes.push(node);
                    }
                    None => text.push('$'),
                },
                '\\' => text.push(self.escaped(&['$', '}', '\\'])),
                _ => {
                    self.bump();
                    text.push(c);
                }
            }
        }
        flush(&mut nodes, &mut text);
        nodes
    }

    /// At a backslash: the char of `escapes` after it, else the backslash.
    fn escaped(&mut self, escapes: &[char]) -> char {
        self.bump();
        match self.peek().filter(|next| escapes.contains(next)) {
            Some(next) => {
                self.bump();
                next
            }
            None => '\\',
        }
    }

    /// A stop, placeholder, choice or variable at `$`. `None`, just past
    /// the `$`, when it starts none of them: the `$` is text.
    fn dollar(&mut self) -> Option<Node> {
        let start = self.at;
        let node = if self.failed.contains(&start) || self.depth >= MAX_DEPTH {
            None
        } else {
            self.bump();
            self.depth = self.depth.saturating_add(1);
            let node = self.construct();
            self.depth = self.depth.saturating_sub(1);
            node
        };
        if node.is_none() {
            self.failed.insert(start);
            self.at = start.saturating_add(1);
        }
        node
    }

    /// After `$`: `1`, `NAME`, or `{` and a stop or variable body.
    fn construct(&mut self) -> Option<Node> {
        if let Some(index) = self.int() {
            return Some(Node::Stop(index, Vec::new()));
        }
        if let Some(name) = self.name() {
            return Some(Node::Var(name, Vec::new()));
        }
        if !self.eat('{') {
            return None;
        }
        if let Some(index) = self.int() {
            return self.body(true).map(|inner| Node::Stop(index, inner));
        }
        let name = self.name()?;
        self.body(false).map(|default| Node::Var(name, default))
    }

    fn int(&mut self) -> Option<u32> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
        let digits: String = self.chars.get(start..self.at)?.iter().collect();
        let index = digits.parse().ok();
        if index.is_none() {
            self.at = start;
        }
        index
    }

    fn name(&mut self) -> Option<String> {
        let first = self
            .peek()
            .filter(|c| c.is_ascii_alphabetic() || *c == '_')?;
        let mut name = String::from(first);
        self.bump();
        while let Some(c) = self
            .peek()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        {
            name.push(c);
            self.bump();
        }
        Some(name)
    }

    /// After `${1` or `${NAME`: `}`, `:placeholder}`, `|choices|}` (a
    /// stop only) or `/transform/}`. Returns the placeholder.
    fn body(&mut self, stop: bool) -> Option<Vec<Node>> {
        let c = self.peek()?;
        self.bump();
        match c {
            '}' => Some(Vec::new()),
            ':' => {
                let inner = self.nodes(true);
                self.eat('}').then_some(inner)
            }
            '|' if stop => self.choice().map(|first| vec![Node::Text(first)]),
            '/' => self.transform().then(Vec::new),
            _ => None,
        }
    }

    /// After `${1|`: the options up to `|}`. Returns the first one.
    fn choice(&mut self) -> Option<String> {
        let mut first: Option<String> = None;
        let mut option = String::new();
        loop {
            match self.peek()? {
                '\\' => option.push(self.escaped(&['$', '}', '\\', ',', '|'])),
                ',' => {
                    self.bump();
                    let done = std::mem::take(&mut option);
                    if first.is_none() {
                        first = Some(done);
                    }
                }
                '|' => {
                    self.bump();
                    return self.eat('}').then(|| first.unwrap_or(option));
                }
                c => {
                    self.bump();
                    option.push(c);
                }
            }
        }
    }

    /// After `/`: skip `regex/format/options}`.
    fn transform(&mut self) -> bool {
        self.skip_part(false) && self.skip_part(true) && self.skip_options()
    }

    /// Up to the next unescaped `/`; in a format, not inside `${…}`.
    fn skip_part(&mut self, format: bool) -> bool {
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            self.bump();
            match c {
                '\\' => self.bump(),
                '$' if format && self.peek() == Some('{') => {
                    self.bump();
                    depth = depth.saturating_add(1);
                }
                '}' if depth > 0 => depth = depth.saturating_sub(1),
                '/' if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    /// Regex flags up to the closing `}`.
    fn skip_options(&mut self) -> bool {
        while let Some(c) = self.peek() {
            self.bump();
            if c == '}' {
                return true;
            }
            if !c.is_ascii_alphabetic() {
                return false;
            }
        }
        false
    }
}

/// Move collected text into `nodes`.
fn flush(nodes: &mut Vec<Node>, text: &mut String) {
    if !text.is_empty() {
        nodes.push(Node::Text(std::mem::take(text)));
    }
}
