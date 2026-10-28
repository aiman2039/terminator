use super::*;

fn test_theme() -> Theme {
    let (syntaxes, themes) = crate::highlight::test_assets();
    let _ = syntaxes;
    themes
        .themes
        .get("Nord")
        .or_else(|| themes.themes.values().next())
        .cloned()
        .expect("theme")
}

#[test]
fn rust_comment_and_keyword_span() {
    let theme = test_theme();
    let lines = highlight_text(Language::Rust, "// hi\nfn f() {}", &theme).expect("spans");
    assert_eq!(lines.len(), 2);
    assert_eq!((lines[0][0].start, lines[0][0].end), (0, 5));
    assert!(lines[1].iter().any(|span| span.start == 0 && span.end == 2));
}

#[test]
fn python_function_spans() {
    let theme = test_theme();
    let lines = highlight_text(Language::Python, "def f():\n    pass", &theme).expect("spans");
    assert!(!lines[0].is_empty());
    assert!(!lines[1].is_empty());
}

#[test]
fn rust_query_captures_constants_attributes_and_fields() {
    let theme = crate::highlight::theme(true).expect("darcula");
    let text = "#[derive(Debug)]\npub const SCALE: u64 = 1_000_000;\nstruct S { a: u64 }\nfn f(s: &S) -> u64 { s.a }\npub enum E {\n    Variant,\n}\n";
    let lines = highlight_text(Language::Rust, text, theme).expect("spans");
    let plain = theme.settings.foreground.expect("foreground");
    let plain = [plain.r, plain.g, plain.b];

    let span_at = |row: usize, col: usize| {
        lines[row]
            .iter()
            .find(|span| span.start <= col && col < span.end)
    };
    // `derive` attribute, and `SCALE` constant name.
    assert!(
        span_at(0, 2).is_some_and(|span| span.color != plain),
        "attribute"
    );
    assert!(
        span_at(1, 10).is_some_and(|span| span.color != plain),
        "const"
    );
    // User-defined type tints and italicizes; primitives stay upright.
    let user_type = span_at(2, 7).expect("type span");
    assert!(user_type.style.italic, "type italic");
    assert_ne!(user_type.color, plain, "type tint");
    let primitive = span_at(2, 14).expect("primitive span");
    assert_ne!(primitive.color, plain, "primitive tint");
    assert_ne!(primitive.color, user_type.color, "type vs primitive");
    assert!(!primitive.style.italic, "primitives are upright");
    // Enum variants are italic, like `IntelliJ`.
    let variant = span_at(5, 4).expect("variant span");
    assert!(variant.style.italic, "enum variant italic");
    assert_ne!(variant.color, plain, "enum variant tint");
    // Field access `s.a`, not the struct declaration.
    let field_line = "fn f(s: &S) -> u64 { s.a }";
    assert_eq!(text.split('\n').nth(3), Some(field_line));
    let field_col = field_line.find('a').expect("field char");
    assert!(
        span_at(3, field_col).is_some_and(|span| span.color != plain),
        "field access"
    );
    assert_eq!(lines.len(), text.split('\n').count());
}

#[test]
fn rows_match_split_lines_and_partition() {
    let theme = test_theme();
    for text in ["", "fn f() {}", "fn f() {}\n", "// hi\nfn f() {}\n", "\n\n"] {
        let lines = highlight_text(Language::Rust, text, &theme).expect("spans");
        assert_eq!(lines.len(), text.split('\n').count(), "rows for {text:?}");
        for (row, spans) in lines.iter().enumerate() {
            let mut col = 0;
            for span in spans {
                assert_eq!(span.start, col, "gap in row {row} of {text:?}");
                col = span.end;
            }
            let width = text.split('\n').nth(row).unwrap_or("").chars().count();
            assert_eq!(col, width, "row {row} of {text:?}");
        }
    }
}

#[test]
fn outline_lists_nested_declarations_in_order() {
    let text = "mod m {\n    pub struct S;\n    impl Clone for S {\n        fn clone(&self) -> S { S }\n    }\n}\nfn main() {}\n";
    let symbols = outline(Language::Rust, text).expect("outline");
    let shape: Vec<(&str, usize, u32)> = symbols
        .iter()
        .map(|symbol| {
            (
                symbol.name.as_str(),
                symbol.depth,
                symbol.selection.start.line,
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("m", 0, 0),
            ("S", 1, 1),
            ("impl Clone for S", 1, 2),
            ("clone", 2, 3),
            ("main", 0, 6),
        ]
    );
    let python = outline(
        Language::Python,
        "class A:\n    def f(self):\n        pass\n",
    )
    .unwrap();
    assert_eq!(
        python
            .iter()
            .map(|s| (s.name.as_str(), s.depth))
            .collect::<Vec<_>>(),
        [("A", 0), ("f", 1)]
    );
    assert!(outline(Language::Json, "{}").is_none());
}

#[test]
fn unsupported_language_falls_back() {
    let theme = test_theme();
    assert!(highlight_text(Language::Toml, "a = 1", &theme).is_none());
}

#[test]
fn oversized_buffers_fall_back() {
    let theme = test_theme();
    let big = "x\n".repeat(MAX_HIGHLIGHT_BYTES / 2 + 1);
    assert!(highlight_text(Language::Rust, &big, &theme).is_none());
}
