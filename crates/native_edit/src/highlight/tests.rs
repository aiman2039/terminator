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
fn dark_theme_uses_islands_dark_colors() {
    let lines = highlight_text(
        Language::Rust,
        "fn main() {\n    // note\n    let x = 1;\n    let s = \"hi\";\n}",
        true,
    )
    .expect("rust highlights");
    let has = |line: usize, color: [u8; 3]| lines[line].iter().any(|span| span.color == color);
    assert!(has(0, [0xCF, 0x8E, 0x6D]), "keyword orange: {:?}", lines[0]);
    assert!(has(0, [0x56, 0xA8, 0xF5]), "function blue: {:?}", lines[0]);
    assert!(has(1, [0x7A, 0x7E, 0x85]), "comment gray: {:?}", lines[1]);
    assert!(has(2, [0x2A, 0xAC, 0xB8]), "number teal: {:?}", lines[2]);
    assert!(has(3, [0x6A, 0xAB, 0x73]), "string green: {:?}", lines[3]);
    assert!(!has(4, [0x7A, 0x7E, 0x85]));
    assert!(
        has(2, [0xBC, 0xBE, 0xC4]),
        "plain text stays Islands Dark foreground: {:?}",
        lines[2]
    );
}

#[test]
fn islands_dark_theme_matches_the_jetbrains_scheme() {
    let theme = theme(true).expect("bundled dark theme");
    assert_eq!(theme.name.as_deref(), Some("Islands Dark"));
    let foreground = theme.settings.foreground.expect("foreground");
    assert_eq!(
        [foreground.r, foreground.g, foreground.b],
        [0xBC, 0xBE, 0xC4]
    );
    let background = theme.settings.background.expect("background");
    assert_eq!(
        [background.r, background.g, background.b],
        [0x19, 0x1A, 0x1C]
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
        // `\r\n` is one line break: spans cover the text before it.
        let visible = source.trim_end_matches('\r');
        assert_eq!(column, visible.chars().count(), "{path}: {source:?}");
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
    assert!(lines[1].iter().any(|span| span.color == [0x7A, 0x7E, 0x85]));
    assert!(lines[3].iter().any(|span| span.color == [0x7A, 0x7E, 0x85]));
    assert!(lines[2].iter().any(|span| span.color == [0x6A, 0xAB, 0x73]));
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
        text: "fn a() {}".into(),
    });
    let first = cache.line(0).to_vec();
    assert!(!first.is_empty());
    cache.ensure(EnsureInput {
        language: Language::Rust,
        dark: true,
        revision: 1,
        text: "fn replaced() {}".into(),
    });
    assert_eq!(cache.line(0), first.as_slice());
    cache.ensure(EnsureInput {
        language: Language::Rust,
        dark: true,
        revision: 2,
        text: "fn b() {}".into(),
    });
    assert_eq!(cache.line(0).len(), first.len());
    assert!(cache.is_current(Language::Rust, true, 2));
    assert!(!cache.is_current(Language::Rust, false, 2));
}

#[test]
fn a_large_buffer_highlights_on_a_worker_without_blocking_the_frame() {
    let ctx = egui::Context::default();
    let text = "fn a() { let x = 1; }\n".repeat(SYNC_HIGHLIGHT_BYTES / 20);
    assert!(text.len() > SYNC_HIGHLIGHT_BYTES);
    let mut cache = HighlightCache::default();
    let input = |revision| EnsureInput {
        language: Language::Rust,
        dark: true,
        revision,
        text: text.as_str().into(),
    };
    cache.ensure_async(input(1), &ctx);
    // The call returns before the parse: nothing is current yet.
    assert!(!cache.is_current(Language::Rust, true, 1));
    // While the worker runs (or once it landed) no new text is wanted.
    assert!(!cache.wants_text(Language::Rust, true, 1));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !cache.is_current(Language::Rust, true, 1) {
        assert!(std::time::Instant::now() < deadline, "worker never landed");
        cache.wants_text(Language::Rust, true, 1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!cache.line(0).is_empty());
    // A newer revision wants text again and keeps the old spans painted.
    assert!(cache.wants_text(Language::Rust, true, 2));
    assert!(!cache.line(0).is_empty());
}

#[test]
fn a_small_buffer_still_highlights_in_the_same_call() {
    let ctx = egui::Context::default();
    let mut cache = HighlightCache::default();
    cache.ensure_async(
        EnsureInput {
            language: Language::Rust,
            dark: true,
            revision: 1,
            text: "fn a() {}".into(),
        },
        &ctx,
    );
    assert!(cache.is_current(Language::Rust, true, 1));
    assert!(!cache.line(0).is_empty());
}

#[test]
fn stylesheet_and_html_extensions_keep_bundled_grammars() {
    for (path, language) in [
        ("index.html", Language::Html),
        ("site.css", Language::Css),
        ("site.scss", Language::Scss),
        ("site.less", Language::Less),
    ] {
        assert_eq!(language_for_path(path), language);
        let extension = language.extension().expect("extension");
        assert!(
            syntax_set().find_syntax_by_extension(extension).is_some(),
            "{path} has no grammar"
        );
    }
}

#[test]
fn lone_carriage_returns_split_lines_like_the_buffer() {
    // The rope counts `\r`, `\r\n` and `\n` as breaks: spans must line up.
    let text = "a = 1\rb = 2\r\nc = 3\n";
    let lines = highlight_text(Language::Toml, text, true).expect("toml highlights");
    let doc = crate::doc::Doc::new(text);
    assert_eq!(lines.len(), crate::doc::Buffer::line_count(&doc));
    assert!(lines.iter().take(3).all(|spans| !spans.is_empty()));
    assert!(lines.iter().flatten().all(|span| span.end <= 5));
}

#[test]
fn split_views_share_one_highlight_per_revision() {
    let mut first = HighlightCache::default();
    first.ensure(EnsureInput {
        language: Language::Rust,
        dark: true,
        revision: 7,
        text: "fn a() {}".into(),
    });
    let mut second = HighlightCache::default();
    second.share_spans(&first);
    assert!(second.is_current(Language::Rust, true, 7));
    assert_eq!(second.line(0), first.line(0));
}

#[test]
fn restored_folds_carry_into_a_forked_view_but_not_into_large_files() {
    let doc = crate::doc::Doc::new("fn a() {\n    x();\n}\n");
    let mut cache = HighlightCache::default();
    cache.restore_folds(&doc, Language::Rust, &[0]);
    assert_eq!(cache.folds().folded_starts(), [0]);
    assert_eq!(cache.fork_folds().folds().folded_starts(), [0]);
    // Large-file mode finds no regions, so nothing folds.
    let big = crate::doc::Doc::new(format!("fn a() {{\n{}}}\n", "    x();\n".repeat(40_000)));
    let mut large = HighlightCache::default();
    large.restore_folds(&big, Language::Rust, &[0]);
    assert!(large.folds().folded_starts().is_empty());
}

#[test]
fn an_over_cap_buffer_skips_building_its_text() {
    let mut cache = HighlightCache::default();
    assert!(cache.skip_large((Language::Rust, true), 3, MAX_HIGHLIGHT_BYTES + 1));
    assert!(cache.is_current(Language::Rust, true, 3));
    assert!(!cache.wants_text(Language::Rust, true, 3));
    assert!(!cache.skip_large((Language::Rust, true), 4, 10));
    cache.set_unlimited(true);
    assert!(!cache.skip_large((Language::Rust, true), 5, MAX_HIGHLIGHT_BYTES + 1));
}
