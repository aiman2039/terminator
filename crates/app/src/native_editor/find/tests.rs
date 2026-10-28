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
