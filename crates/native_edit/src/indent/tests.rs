use super::*;
use crate::doc::{Buffer, Doc};
use crate::vim::parse_vimrc;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, text).expect("write");
}

#[test]
fn detection_finds_tabs_or_the_space_step() {
    let two = IndentStyle::detect("a {\n  b {\n    c\n  }\n}\n".lines());
    assert_eq!(two, Some(IndentStyle::spaces(2)));
    let four = IndentStyle::detect("fn a() {\n    b();\n        c();\n}\n".lines());
    assert_eq!(four, Some(IndentStyle::spaces(4)));
    let tabs = IndentStyle::detect("a {\n\tb\n\t\tc\n  d\n}\n".lines());
    assert_eq!(tabs, Some(IndentStyle::tabs(DEFAULT_WIDTH)));
    // An odd alignment step falls back to four spaces, and an aligned
    // continuation line does not hide a 4-space step.
    assert_eq!(
        IndentStyle::detect("x\n   y\n".lines()),
        Some(IndentStyle::spaces(4))
    );
    let python = "def f(a,\n      b):\n    return 1\n\nclass C:\n    def g(self):\n        pass\n";
    assert_eq!(
        IndentStyle::detect(python.lines()),
        Some(IndentStyle::spaces(4))
    );
    // Closing two levels at once does not make a 2-space file look wider.
    let yaml = "a:\n  b:\n    c: 1\nd:\n  e: 2\n";
    assert_eq!(
        IndentStyle::detect(yaml.lines()),
        Some(IndentStyle::spaces(2))
    );
    assert_eq!(IndentStyle::detect("flat\n\n  \nlines\n".lines()), None);
    // A buffer starts with what its text uses, else four spaces.
    assert_eq!(Doc::new("\tx\n").indent_style(), IndentStyle::tabs(4));
    assert_eq!(Doc::new("x\n").indent_style(), IndentStyle::spaces(4));
}

#[test]
fn go_and_makefiles_default_to_tabs() {
    for name in ["main.go", "Makefile", "GNUmakefile", "rules.mk", "go.mod"] {
        assert_eq!(
            IndentStyle::for_path(Path::new(name)),
            Some(IndentStyle::tabs(DEFAULT_WIDTH)),
            "{name}"
        );
    }
    assert_eq!(IndentStyle::for_path(Path::new("main.rs")), None);
    // Detection still wins: a Go file indented with spaces keeps them.
    let config = EditorConfig::default();
    let spaced = config.indent(Path::new("a.go"), "func a() {\n  b()\n}\n", None);
    assert_eq!(spaced, IndentStyle::spaces(2));
    assert!(config.indent(Path::new("a.go"), "", None).tabs);
}

#[test]
fn vimrc_indentation_applies_only_when_set() {
    assert_eq!(IndentStyle::from_vimrc(&parse_vimrc("set number\n")), None);
    let spaces = IndentStyle::from_vimrc(&parse_vimrc("set sw=2 et\n"));
    assert_eq!(spaces, Some(IndentStyle::spaces(2)));
    let tabs = IndentStyle::from_vimrc(&parse_vimrc("set noet sw=8\n"));
    assert_eq!(tabs, Some(IndentStyle::tabs(8)));
    // An absurd width counts as unset.
    assert_eq!(IndentStyle::from_vimrc(&parse_vimrc("set ts=999\n")), None);
    // The vimrc comes after the language default and before four spaces.
    let config = EditorConfig::default();
    assert_eq!(
        config.indent(Path::new("a.rs"), "", spaces),
        IndentStyle::spaces(2)
    );
    assert!(config.indent(Path::new("Makefile"), "", spaces).tabs);
    assert_eq!(
        config.indent(Path::new("a.rs"), "", None),
        IndentStyle::default()
    );
}

#[test]
fn units_tab_stops_and_outdents_follow_the_style() {
    let two = IndentStyle::spaces(2);
    assert_eq!(two.unit(), "  ");
    assert_eq!(two.tab_text(3), " ");
    assert_eq!(two.tab_text(4), "  ");
    assert_eq!(two.outdent_width("     x"), 2);
    assert_eq!(two.outdent_width("\t x"), 1);
    let tabs = IndentStyle::tabs(8);
    assert_eq!(
        (tabs.unit(), tabs.tab_text(3)),
        ("\t".to_owned(), "\t".to_owned())
    );
    assert_eq!(tabs.level(), 8);
    assert_eq!(
        tabs.format_options(),
        crate::lsp::FormatOptions {
            tab_size: 8,
            insert_spaces: false
        }
    );
    // A zero width never stalls or divides by zero.
    let zero = IndentStyle {
        tabs: false,
        width: 0,
        tab_width: 0,
    };
    assert_eq!((zero.level(), zero.tab_cells()), (1, 1));
    assert_eq!(zero.tab_text(5), " ");
}

#[test]
fn editorconfig_nested_files_override_their_parents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    write(
        &root.join(".editorconfig"),
        "root = true\n\n[*]\nindent_style = space\nindent_size = 2\ntrim_trailing_whitespace = true\n",
    );
    write(
        &root.join("sub/.editorconfig"),
        "[*.go]\nindent_style = tab\ntab_width = 8\n\n[*.md]\ntrim_trailing_whitespace = false\n",
    );
    let rust = EditorConfig::for_file(&root.join("sub/main.rs"));
    assert_eq!(
        (rust.tabs, rust.width, rust.tab_width),
        (Some(false), Some(2), Some(2))
    );
    assert!(rust.trim_trailing_whitespace);
    let go = EditorConfig::for_file(&root.join("sub/main.go"));
    assert_eq!(
        (go.tabs, go.width, go.tab_width),
        (Some(true), Some(2), Some(8))
    );
    let style = go.indent(Path::new("main.go"), "func a() {\n    b()\n}\n", None);
    assert_eq!((style.tabs, style.level()), (true, 8));
    let markdown = EditorConfig::for_file(&root.join("sub/README.md"));
    assert!(!markdown.trim_trailing_whitespace);
    // A field the file leaves out comes from the text.
    let partial = EditorConfig {
        width: Some(3),
        ..EditorConfig::default()
    };
    let style = partial.indent(Path::new("a.c"), "a\n\tb\n", None);
    assert_eq!((style.tabs, style.width), (true, 3));
}

#[test]
fn editorconfig_search_stops_at_root_true() {
    let dir = tempfile::tempdir().expect("tempdir");
    let top = dir.path();
    write(
        &top.join(".editorconfig"),
        "root = true\n[*]\nindent_size = 8\ninsert_final_newline = true\n",
    );
    write(
        &top.join("proj/.editorconfig"),
        "root = true\n[*]\nindent_style = tab\n",
    );
    let inner = EditorConfig::for_file(&top.join("proj/src/a.c"));
    assert_eq!(inner.tabs, Some(true));
    assert_eq!((inner.width, inner.tab_width), (None, None));
    assert!(!inner.final_newline, "the outer file is past the root");
    let outer = EditorConfig::for_file(&top.join("other/a.c"));
    assert_eq!(
        (outer.tabs, outer.width, outer.tab_width),
        (None, Some(8), Some(8))
    );
    assert!(outer.final_newline);
}

#[test]
fn editorconfig_reads_save_rules_and_ignores_broken_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        &dir.path().join(".editorconfig"),
        "root = true\n[*]\nend_of_line = crlf\ninsert_final_newline = true\n\
         trim_trailing_whitespace = true\nindent_size = 0\n",
    );
    let config = EditorConfig::for_file(&dir.path().join("a.txt"));
    assert_eq!(config.end_of_line, Some(EndOfLine::Crlf));
    assert!(config.final_newline && config.trim_trailing_whitespace);
    assert_eq!(config.width, None, "a zero width is ignored");
    let broken = tempfile::tempdir().expect("tempdir");
    write(
        &broken.path().join(".editorconfig"),
        "root = true\n[*]\nindent_style = tab\nnot a setting\n",
    );
    let path = broken.path().join("a.txt");
    assert_eq!(EditorConfig::for_file(&path), EditorConfig::default());
}

#[test]
fn save_rules_trim_lines_without_a_caret_and_end_with_a_newline() {
    let config = EditorConfig {
        final_newline: true,
        trim_trailing_whitespace: true,
        ..EditorConfig::default()
    };
    let mut doc = Doc::new("a  \nb\t \nc ");
    for (range, text) in config.save_edits(&doc, &[1]) {
        doc.delete(range.clone());
        doc.insert(range.start, &text);
    }
    assert_eq!(doc.text(), "a\nb\t \nc\n");
    // Nothing set, nothing left to change, or an empty buffer: no edits.
    assert!(EditorConfig::default().save_edits(&doc, &[]).is_empty());
    assert!(config.save_edits(&doc, &[1]).is_empty());
    assert!(config.save_edits(&Doc::new(""), &[]).is_empty());
    // CRLF breaks inside the buffer are not trailing whitespace.
    let mut crlf = Doc::new("a \r\nb\r\n");
    for (range, text) in config.save_edits(&crlf, &[]) {
        crlf.delete(range.clone());
        crlf.insert(range.start, &text);
    }
    assert_eq!(crlf.text(), "a\r\nb\r\n");
}
