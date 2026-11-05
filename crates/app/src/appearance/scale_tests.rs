//! The shared type and radius scale, and a source guard in the spirit of
//! `cargo xtask async-boundary`: a literal font size or corner radius in
//! `crates/app/src` outside `appearance/scale.rs` fails this suite.
use super::scale::*;
use super::{apply, install, menu_style};
use eframe::egui::{self, FontId, TextStyle};
use std::fs;
use std::path::{Path, PathBuf};
use terminator_core::appearance::AppearanceConfig;

/// Calls whose argument at the index is a font size or a corner radius.
const SIZED_CALLS: [(&str, usize); 12] = [
    ("FontId::proportional(", 0),
    ("FontId::monospace(", 0),
    ("FontId::new(", 0),
    (".size(", 0),
    ("CornerRadius::same(", 0),
    (".corner_radius(", 0),
    ("rect_filled(", 1),
    ("rect_stroke(", 1),
    (".rect(", 1),
    ("RectShape::new(", 1),
    ("RectShape::filled(", 1),
    ("RectShape::stroke(", 1),
];

/// Struct fields that hold a font size or a corner radius.
const SIZED_FIELDS: [&str; 3] = ["FontId{size:", "CornerRadius{nw:", "corner_radius:"];

/// Deliberate literals: the file under `src/`, and the call as the guard
/// reports it.
const ALLOWED: [(&str, &str); 4] = [
    // The Markdown preview sets document reading sizes, not app chrome.
    ("markdown/previews/render.rs", "FontId::proportional(16.0)"),
    ("markdown/previews/render.rs", "FontId::new(28.0,"),
    ("markdown/previews/render.rs", "FontId::monospace(14.0)"),
    // Player spectrum bars keep a hairline corner at any bar width.
    ("player/ui/mod.rs", "rect_filled(bar,1.0,"),
];

/// Folders that keep their own sizes: the VNC page, cards, and overlays.
const ALLOWED_DIRS: [&str; 1] = ["vnc_ui/"];

#[test]
fn font_roles_step_from_the_base_like_jbfont() {
    let roles = [
        FONT_MINI,
        FONT_CAPTION,
        FONT_SMALL,
        FONT_BODY,
        FONT_LARGE,
        FONT_HEADING,
        FONT_TITLE,
    ];
    assert_eq!(
        roles.map(|size| size - FONT_BASE),
        [-4.0, -2.0, -1.0, 0.0, 1.0, 3.0, 5.0]
    );
    assert_eq!(FONT_BASE, 13.0);
}

#[test]
fn radius_steps_grow_from_marks_to_containers() {
    assert_eq!(
        [RADIUS_XSMALL, RADIUS_SMALL, RADIUS_MEDIUM, RADIUS_LARGE],
        [2, 4, 6, 10]
    );
    assert_eq!(WIDGET_RADIUS, RADIUS_MEDIUM);
    assert_eq!((PANEL_RADIUS, MENU_RADIUS), (RADIUS_LARGE, RADIUS_LARGE));
}

#[test]
fn egui_text_styles_follow_the_scale() {
    let ctx = egui::Context::default();
    install(&ctx);
    // `apply` also sets the process-wide chrome tints. Apply the theme that
    // `intellij_theme_tokens_match_new_ui_geometry` applies, so tests that
    // share one `cargo test` process see no other theme switch.
    apply(&ctx, &AppearanceConfig::intellij_dark());
    let style = ctx.style_of(egui::Theme::Dark);
    let font = |role: TextStyle| style.text_styles.get(&role).cloned();
    assert_eq!(
        font(TextStyle::Small),
        Some(FontId::proportional(FONT_SMALL))
    );
    assert_eq!(font(TextStyle::Body), Some(FontId::proportional(FONT_BODY)));
    assert_eq!(
        font(TextStyle::Button),
        Some(FontId::proportional(FONT_BODY))
    );
    assert_eq!(
        font(TextStyle::Monospace),
        Some(FontId::monospace(FONT_BODY))
    );
    assert_eq!(
        font(TextStyle::Heading),
        Some(FontId::new(FONT_HEADING, semibold()))
    );
    let mut menu = egui::Style::default();
    menu_style(&mut menu);
    let menu_font = |role: TextStyle| menu.text_styles.get(&role).cloned();
    assert_eq!(
        menu_font(TextStyle::Body),
        Some(FontId::proportional(FONT_SMALL))
    );
    assert_eq!(
        menu_font(TextStyle::Small),
        Some(FontId::proportional(FONT_CAPTION))
    );
}

#[test]
fn app_sources_take_font_sizes_and_radii_from_the_scale() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(files.len() > 100, "the guard must see the app sources");
    let mut violations = Vec::new();
    let mut used = [false; ALLOWED.len()];
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if exempt(&relative) {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        for (line, call) in literal_sizes(&source) {
            let allowed = ALLOWED
                .iter()
                .position(|&(file, text)| file == relative && text == call);
            match allowed.and_then(|index| used.get_mut(index)) {
                Some(slot) => *slot = true,
                None => violations.push(format!("{relative}:{line}: {call}")),
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Use an appearance scale token (FONT_*, RADIUS_*, WIDGET_RADIUS, \
         egui::CornerRadius::ZERO) instead of:\n{}",
        violations.join("\n")
    );
    let stale: Vec<_> = ALLOWED
        .iter()
        .zip(used)
        .filter(|(_, seen)| !seen)
        .map(|(entry, _)| entry)
        .collect();
    assert!(stale.is_empty(), "remove stale exceptions: {stale:?}");
}

#[test]
fn guard_reads_calls_across_lines_and_skips_comments_strings_and_tokens() {
    let source = r##"
        // painter.rect_filled(rect, 4.0, fill) in a comment is fine.
        /* FontId::proportional(12.0) */
        const PILL_FONT: f32 = 11.0;
        const LABEL: f32 = appearance::FONT_CAPTION;
        fn paint(ui: &egui::Ui) {
            ui.painter().rect_stroke(
                rect,
                2,
                stroke,
                egui::StrokeKind::Inside,
            );
            let a = egui::FontId::proportional(PILL_FONT);
            let b = egui::FontId::proportional(LABEL);
            let c = RichText::new("rect_filled(r, 3, f)").size(12.0);
            let d = r#"FontId::monospace(10.0)"#;
            let e = ['(', '"', '\''];
            painter.rect(rect, height * 0.5, fill, stroke, kind);
            painter.rect_filled(rect, appearance::RADIUS_SMALL, fill);
            let f = egui::FontId::monospace(settings.font_size);
            let g = egui::Frame::new().corner_radius(egui::CornerRadius::ZERO);
        }
    "##;
    assert_eq!(
        literal_sizes(source),
        [
            (7, "rect_stroke(rect,2,".to_owned()),
            (13, "FontId::proportional(PILL_FONT)".to_owned()),
            (15, ".size(12.0)".to_owned()),
        ]
    );
}

/// Files the guard leaves alone: the scale itself, tests, and the
/// folders in [`ALLOWED_DIRS`].
fn exempt(relative: &str) -> bool {
    relative == "appearance/scale.rs"
        || relative.ends_with("tests.rs")
        || relative.split('/').any(|part| part == "tests")
        || ALLOWED_DIRS.iter().any(|dir| relative.starts_with(dir))
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// Font-size and corner-radius arguments in `source` that are a number, or
/// a constant this file sets to a number, as (line, compact call text).
fn literal_sizes(source: &str) -> Vec<(usize, String)> {
    let (code, lines) = compact(source);
    let mut found = Vec::new();
    for (call, index) in SIZED_CALLS {
        for (at, _) in code.match_indices(call) {
            let start = at.saturating_add(call.len());
            let rest = code.get(start..).unwrap_or_default();
            let Some(&(from, to)) = arguments(rest).get(index) else {
                continue;
            };
            if is_literal(&code, rest.get(from..to).unwrap_or_default()) {
                let end = start.saturating_add(to).saturating_add(1);
                found.push((line_at(&lines, at), text_at(&code, at, end)));
            }
        }
    }
    for field in SIZED_FIELDS {
        for (at, _) in code.match_indices(field) {
            let start = at.saturating_add(field.len());
            let rest = code.get(start..).unwrap_or_default();
            let value = rest.split([',', '}']).next().unwrap_or_default();
            if is_literal(&code, value) {
                let end = start.saturating_add(value.len());
                found.push((line_at(&lines, at), text_at(&code, at, end)));
            }
        }
    }
    found.sort();
    found
}

fn line_at(lines: &[usize], at: usize) -> usize {
    lines.get(at).copied().unwrap_or_default()
}

fn text_at(code: &str, from: usize, to: usize) -> String {
    code.get(from..to).unwrap_or_default().to_owned()
}

/// Byte ranges of the top-level arguments in `rest`, up to the closing
/// parenthesis.
fn arguments(rest: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (at, c) in rest.char_indices() {
        match c {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' if depth == 0 => {
                found.push((start, at));
                break;
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                found.push((start, at));
                start = at.saturating_add(1);
            }
            _ => {}
        }
    }
    found
}

/// A number, or a constant `code` sets to a number.
fn is_literal(code: &str, argument: &str) -> bool {
    if is_number(argument) {
        return true;
    }
    let name = argument.rsplit("::").next().unwrap_or_default();
    let constant = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    constant
        && code.match_indices(&format!("const{name}:")).any(|(at, _)| {
            code.get(at..)
                .and_then(|rest| rest.split_once('='))
                .and_then(|(_, value)| value.split(';').next())
                .is_some_and(is_number)
        })
}

fn is_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    digits.starts_with(|c: char| c.is_ascii_digit())
        && digits
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

/// Lexer state for [`compact`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lex {
    Code,
    LineComment,
    BlockComment,
    Str,
    RawStr(usize),
}

/// `source` without comments, whitespace, or string and char contents,
/// and the line of each kept byte, so a call split over lines reads as one.
fn compact(source: &str) -> (String, Vec<usize>) {
    let mut code = String::new();
    let mut lines = Vec::new();
    let mut line = 1_usize;
    let mut lex = Lex::Code;
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        let next = chars.clone().next();
        let before = lex;
        lex = match lex {
            Lex::Code => after_code(c, next, &mut chars),
            Lex::LineComment if c == '\n' => Lex::Code,
            Lex::BlockComment if c == '*' && next == Some('/') => {
                chars.next();
                Lex::Code
            }
            Lex::Str if c == '\\' => {
                if chars.next() == Some('\n') {
                    line = line.saturating_add(1);
                }
                Lex::Str
            }
            Lex::Str if c == '"' => Lex::Code,
            Lex::RawStr(hashes) if c == '"' && closes_raw(&mut chars, hashes) => Lex::Code,
            state => state,
        };
        let kept = match (before, lex) {
            (Lex::Code, Lex::Code) if !c.is_whitespace() && c != '\'' => Some(c),
            (Lex::Code, Lex::Str | Lex::RawStr(_)) | (Lex::Str | Lex::RawStr(_), Lex::Code) => {
                Some('"')
            }
            _ => None,
        };
        if let Some(kept) = kept {
            code.push(kept);
            lines.extend(std::iter::repeat_n(line, kept.len_utf8()));
        }
        if c == '\n' {
            line = line.saturating_add(1);
        }
    }
    (code, lines)
}

fn after_code(c: char, next: Option<char>, chars: &mut std::str::Chars<'_>) -> Lex {
    match c {
        '/' if next == Some('/') => Lex::LineComment,
        '/' if next == Some('*') => Lex::BlockComment,
        '"' => Lex::Str,
        '\'' => {
            skip_char_literal(chars);
            Lex::Code
        }
        'r' => raw_string_open(chars).map_or(Lex::Code, Lex::RawStr),
        _ => Lex::Code,
    }
}

/// After an opening `'`: consume a char literal, or nothing for a lifetime.
fn skip_char_literal(chars: &mut std::str::Chars<'_>) {
    let mut ahead = chars.clone();
    match (ahead.next(), ahead.next()) {
        (Some('\\'), _) => {
            chars.next();
            chars.next();
            for c in chars.by_ref() {
                if c == '\'' {
                    break;
                }
            }
        }
        (Some(_), Some('\'')) => {
            chars.next();
            chars.next();
        }
        _ => {}
    }
}

/// After an `r`: the `#` count of a raw string that opens here.
fn raw_string_open(chars: &mut std::str::Chars<'_>) -> Option<usize> {
    let mut ahead = chars.clone();
    let mut hashes = 0_usize;
    loop {
        match ahead.next() {
            Some('#') => hashes = hashes.saturating_add(1),
            Some('"') => break,
            _ => return None,
        }
    }
    *chars = ahead;
    Some(hashes)
}

/// After a `"` inside a raw string: whether `hashes` `#` close it here.
fn closes_raw(chars: &mut std::str::Chars<'_>, hashes: usize) -> bool {
    let mut ahead = chars.clone();
    if (0..hashes).all(|_| ahead.next() == Some('#')) {
        *chars = ahead;
        true
    } else {
        false
    }
}
