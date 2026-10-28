use super::*;

#[test]
fn extensions_map_to_languages() {
    assert_eq!(language_for_path("main.rs"), Language::Rust);
    assert_eq!(language_for_path("app.py"), Language::Python);
    assert_eq!(language_for_path("ui.tsx"), Language::TypeScript);
    assert_eq!(language_for_path("data.yaml"), Language::Yaml);
    assert_eq!(language_for_path("notes.md"), Language::Markdown);
    assert_eq!(language_for_path("nope.zq"), Language::Plain);
    assert_ne!(language_for_path("Makefile"), Language::Plain);
}

#[test]
fn rust_keyword_gets_own_span() {
    let lines = highlight_text(Language::Rust, "fn main() {}", true).expect("rust highlights");
    assert_eq!(lines.len(), 1);
    let first = lines[0].first().expect("first span");
    assert_eq!((first.start, first.end), (0, 2));
    // A real theme paints more than one color on this line.
    let mut colors: Vec<[u8; 3]> = lines[0].iter().map(|span| span.color).collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(colors.len() > 1, "spans: {:?}", lines[0]);
}

#[test]
fn dark_theme_uses_darcula_colors() {
    let lines = highlight_text(
        Language::Rust,
        "fn main() {\n    // note\n    let x = 1;\n}",
        true,
    )
    .expect("rust highlights");
    let has = |line: usize, color: [u8; 3]| lines[line].iter().any(|span| span.color == color);
    assert!(has(0, [0xCC, 0x78, 0x32]), "keyword orange: {:?}", lines[0]);
    assert!(
        has(0, [0xFF, 0xC6, 0x6D]),
        "function yellow: {:?}",
        lines[0]
    );
    assert!(has(1, [0x80, 0x80, 0x80]), "comment gray: {:?}", lines[1]);
    assert!(has(2, [0x68, 0x97, 0xBB]), "number blue: {:?}", lines[2]);
    assert!(!has(3, [0x80, 0x80, 0x80]));
    assert!(
        lines[2].iter().any(|span| span.color == [0xA9, 0xB7, 0xC6]),
        "plain text stays Darcula foreground: {:?}",
        lines[2]
    );
}

#[test]
fn javascript_config_has_keyword_and_string_colors() {
    let lines = highlight_text(
        Language::JavaScript,
        "import js from \"@eslint/js\";\nconst value = 42;",
        true,
    )
    .expect("JavaScript highlights");
    let mut colors: Vec<[u8; 3]> = lines
        .iter()
        .flat_map(|line| line.iter().map(|span| span.color))
        .collect();
    colors.sort_unstable();
    colors.dedup();
    assert!(
        colors.len() > 1,
        "JavaScript painted in one color: {lines:?}"
    );
    let color_at = |line: usize, column: usize| {
        lines[line]
            .iter()
            .find(|span| span.start <= column && column < span.end)
            .map(|span| span.color)
            .expect("colored position")
    };
    assert_ne!(color_at(0, 0), color_at(0, 7), "import keyword is plain");
    assert_ne!(color_at(0, 16), color_at(0, 7), "module string is plain");
}

#[test]
fn shell_shebang_and_comments_do_not_leak_into_later_lines() {
    for path in [
        "script.sh",
        "script.bash",
        "script.zsh",
        "script.ksh",
        ".bashrc",
        ".zshrc",
    ] {
        assert_ne!(language_for_path(path), Language::Plain, "{path}");
        assert_ne!(syntax_for_path(path).name, "Plain Text", "{path}");
        for ending in ["\n", "\r\n"] {
            let text = [
                "#!/bin/sh",
                "# comment",
                "if [ -n \"$HOME\" ]; then",
                "  echo \"héllo $HOME\"",
                "fi",
                "",
            ]
            .join(ending);
            for dark in [true, false] {
                assert_shell_script_highlights(path, &text, dark);
            }
        }
    }
}

/// The comment stays on its line, the command line has syntax colors, and
/// every row's spans partition it.
fn assert_shell_script_highlights(path: &str, text: &str, dark: bool) {
    let lines = highlight_text(language_for_path(path), text, dark).expect("shell highlights");
    let comment = lines[1][0].color;
    assert_ne!(
        lines[2][0].color, comment,
        "comment leaked: {path}: {lines:?}"
    );
    let colors: std::collections::HashSet<_> = lines[3].iter().map(|span| span.color).collect();
    assert!(
        colors.len() > 1,
        "shell command has no syntax colors: {path}: {lines:?}"
    );
    for (source, spans) in text.split('\n').zip(&lines) {
        let mut column = 0;
        for span in spans {
            assert_eq!(span.start, column);
            column = span.end;
        }
        assert_eq!(column, source.chars().count(), "{path}: {source:?}");
    }
    assert_eq!(lines.len(), text.split('\n').count());
}

#[test]
fn shell_heredoc_ends_before_the_next_command() {
    let text = "#!/bin/sh\ncat <<'EOF'\nhello\nEOF\necho \"done\"";
    let lines = highlight_text(Language::Shell, text, true).expect("shell highlights");
    assert_eq!(lines.len(), 5);
    let heredoc_color = lines[2][0].color;
    assert_ne!(
        lines[4][0].color, heredoc_color,
        "heredoc leaked: {lines:?}"
    );
    assert_eq!(
        lines[4].last().expect("last span").end,
        "echo \"done\"".len()
    );
}

#[test]
fn syntect_preserves_empty_and_unterminated_last_rows() {
    for text in ["", "echo hi", "echo hi\n", "# comment\necho hi"] {
        let lines = highlight_text(Language::Shell, text, true).expect("shell highlights");
        assert_eq!(lines.len(), text.split('\n').count());
        for (source, spans) in text.split('\n').zip(lines) {
            assert_eq!(
                spans.last().map_or(0, |span| span.end),
                source.chars().count()
            );
        }
    }
}

#[test]
fn spans_partition_the_whole_line() {
    let lines = highlight_text(Language::Rust, "let x = 1;", true).expect("rust highlights");
    let mut col = 0;
    for span in &lines[0] {
        assert_eq!(span.start, col);
        col = span.end;
    }
    assert_eq!(col, "let x = 1;".chars().count());
}

#[test]
fn oversized_buffers_paint_plain() {
    let big = "x\n".repeat(MAX_HIGHLIGHT_BYTES / 2 + 1);
    assert!(highlight_text(Language::Rust, &big, true).is_none());
}

#[test]
fn plain_language_paints_plain() {
    assert!(highlight_text(Language::Plain, "fn main() {}", true).is_none());
}

#[test]
fn jsonc_highlights_comments_and_keeps_urls_as_strings() {
    assert_eq!(language_for_path("wrangler.jsonc"), Language::Json);
    let text = "{\n  // Comment\n  \"url\": \"https://example.test\",\n  /* Block */ \"enabled\": true,\n}\n";
    let lines =
        highlight_text(language_for_path("wrangler.jsonc"), text, true).expect("JSONC highlight");
    assert!(lines[1].iter().any(|span| span.color == [0x80, 0x80, 0x80]));
    assert!(lines[3].iter().any(|span| span.color == [0x80, 0x80, 0x80]));
    assert!(lines[2].iter().any(|span| span.color == [0x6A, 0x87, 0x59]));
}

#[test]
fn common_format_inventory_uses_the_bundled_grammars() {
    for path in [
        "index.html",
        "style.css",
        "style.scss",
        "query.sql",
        "Dockerfile",
        "Makefile",
        "query.graphql",
        "main.lua",
        "main.rb",
        "main.php",
        "main.swift",
        "main.kt",
        "main.cs",
        "main.tf",
        ".env",
        "config.xml",
        "app.vue",
    ] {
        let language = language_for_path(path);
        assert_ne!(language, Language::Plain, "missing grammar: {path}");
        assert!(
            highlight_text(language, "value = 1", true).is_some(),
            "missing highlight: {path}"
        );
    }
}

#[test]
fn cache_only_recomputes_on_key_change() {
    let mut cache = HighlightCache::default();
    cache.ensure(EnsureInput {
        language: Language::Rust,
        dark: true,
        revision: 1,
        text: "fn a() {}",
    });
    let first = cache.line(0).to_vec();
    assert!(!first.is_empty());
    cache.ensure(EnsureInput {
        language: Language::Rust,
        dark: true,
        revision: 1,
        text: "fn replaced() {}",
    });
    assert_eq!(cache.line(0), first.as_slice());
    cache.ensure(EnsureInput {
        language: Language::Rust,
        dark: true,
        revision: 2,
        text: "fn b() {}",
    });
    assert_eq!(cache.line(0).len(), first.len());
    assert!(cache.is_current(Language::Rust, true, 2));
    assert!(!cache.is_current(Language::Rust, false, 2));
}
