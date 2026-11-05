use super::super::search::run;
use super::edits::Stale;
use super::*;
use crate::native_editor::rename::apply_rename;
use crate::native_editor::test_support;
use crate::search::{Query, build_regex};

fn write(root: &Path, name: &str, body: &str) -> PathBuf {
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
    path
}

/// Run the window's search to the end, on this thread.
fn search_again(find: &mut FindInFiles) {
    let (request, cancel, generation) = find.next_search().unwrap();
    let mut files = Vec::new();
    let finish = run(&request, &cancel, &mut |file| {
        files.push(file);
        true
    })
    .unwrap();
    for file in files {
        find.accept(generation, SearchEvent::File(file));
    }
    let truncated = finish.truncated;
    find.accept(generation, SearchEvent::Done { truncated });
}

/// A window on `root` whose search for `query` ran to the end.
fn searched(root: &Path, query: &str, regex: bool) -> FindInFiles {
    let scope = FindScope {
        root: root.to_path_buf(),
        folder: None,
    };
    let mut find = FindInFiles::new(scope, query.into());
    find.regex = regex;
    search_again(&mut find);
    find
}

/// Replace in Files open on `root`, searched for `query`, with `text` as
/// the replacement.
fn replacing(app: &mut App, root: &Path, (query, text): (&str, &str)) {
    let mut find = searched(root, query, false);
    find.set_replace_mode(true);
    find.replace.text = text.into();
    app.find_in_files = Some(find);
}

/// Settle the replace in flight, or fail after 10 s.
fn finish(app: &mut App, ctx: &egui::Context) {
    let start = Instant::now();
    while app
        .find_in_files
        .as_ref()
        .is_some_and(|find| find.replace.busy())
    {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        app.drive_replace(ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Search results for `path` with hits on `lines` (`foo N` each).
fn hits(path: &str, lines: &[u32]) -> FileHits {
    FileHits {
        path: PathBuf::from(path),
        hits: lines
            .iter()
            .map(|&line| LineHit {
                line,
                character: 0,
                preview: format!("foo {line}"),
                matches: vec![(0, 3)],
            })
            .collect(),
        lines: lines
            .iter()
            .map(|&line| (line, format!("foo {line}")))
            .collect(),
    }
}

/// A window on `/p` listing `files` for `foo`, the search done.
fn listed(files: Vec<FileHits>) -> FindInFiles {
    let scope = FindScope {
        root: PathBuf::from("/p"),
        folder: None,
    };
    let mut find = FindInFiles::new(scope, "foo".into());
    let (_, _, generation) = find.next_search().unwrap();
    for file in files {
        find.accept(generation, SearchEvent::File(file));
    }
    find.accept(generation, SearchEvent::Done { truncated: false });
    find
}

/// The planned lines by file.
fn planned(find: &FindInFiles, selected_only: bool) -> Vec<(String, Vec<u32>)> {
    let plan = find.replace_plan(selected_only).unwrap();
    plan.files
        .iter()
        .map(|file| {
            let lines = file.lines.iter().map(|(line, _)| *line).collect();
            (file.path.display().to_string(), lines)
        })
        .collect()
}

fn entry(path: &str, lines: &[u32]) -> (String, Vec<u32>) {
    (path.to_owned(), lines.to_vec())
}

#[test]
fn regex_groups_expand_only_in_regex_mode() {
    let regex = build_regex(&Query {
        text: r"(\w+)_id".into(),
        use_regex: true,
        ..Default::default()
    })
    .unwrap();
    let groups = LineReplace {
        regex: &regex,
        replacement: "${1}Id",
        expand: true,
    };
    let change = change_line("let user_id = item_id;", &groups);
    assert_eq!(change.new, "let userId = itemId;");
    // Char ranges of each match and of what replaced it, for the preview.
    assert_eq!(change.old_marks, [(4, 11), (14, 21)]);
    assert_eq!(change.new_marks, [(4, 10), (13, 19)]);
    // Without regex mode the replacement goes in as typed.
    let literal = LineReplace {
        expand: false,
        ..groups
    };
    assert_eq!(change_line("user_id", &literal).new, "${1}Id");
    // `$$` is one dollar sign.
    let dollar = LineReplace {
        replacement: "$$${1}",
        expand: true,
        ..literal
    };
    assert_eq!(change_line("a_id", &dollar).new, "$a");
}

#[test]
fn match_case_and_words_decide_which_spellings_change() {
    let line = "Foo foo FOO food";
    let replaced = |match_case, whole_word| {
        let regex = build_regex(&Query {
            text: "foo".into(),
            match_case,
            whole_word,
            ..Default::default()
        })
        .unwrap();
        let how = LineReplace {
            regex: &regex,
            replacement: "bar",
            expand: false,
        };
        change_line(line, &how).new
    };
    assert_eq!(replaced(false, false), "bar bar bar bard");
    assert_eq!(replaced(true, false), "Foo bar FOO bard");
    assert_eq!(replaced(false, true), "bar bar bar food");
    // A plain query is literal: `a.b` leaves `axb` alone.
    let dot = build_regex(&Query {
        text: "a.b".into(),
        ..Default::default()
    })
    .unwrap();
    let how = LineReplace {
        regex: &dot,
        replacement: "c",
        expand: false,
    };
    assert_eq!(change_line("a.b axb", &how).new, "c axb");
}

#[test]
fn unchecked_files_and_lines_stay_out_of_the_plan() {
    let mut find = listed(vec![hits("/p/a.rs", &[1, 4, 9]), hits("/p/b.rs", &[2, 7])]);
    let a = Path::new("/p/a.rs");
    let b = Path::new("/p/b.rs");
    assert_eq!(
        planned(&find, false),
        [entry("/p/a.rs", &[1, 4, 9]), entry("/p/b.rs", &[2, 7])]
    );
    // One line of a.rs out: its file box shows partial.
    find.toggle_include(a, Some(4));
    assert_eq!(find.replace.file_check(&find.files[0]), (true, true));
    // b.rs out whole.
    find.toggle_include(b, None);
    assert_eq!(find.replace.file_check(&find.files[1]), (false, false));
    assert_eq!(planned(&find, false), [entry("/p/a.rs", &[1, 9])]);
    assert_eq!(find.included_counts(), (2, 1));
    // Checking a line of an unchecked file takes back that line only.
    find.toggle_include(b, Some(7));
    assert_eq!(
        planned(&find, false),
        [entry("/p/a.rs", &[1, 9]), entry("/p/b.rs", &[7])]
    );
    // The file box takes a partial file out, then back whole.
    find.toggle_include(b, None);
    find.toggle_include(b, None);
    assert_eq!(planned(&find, false)[1], entry("/p/b.rs", &[2, 7]));
    // Replace in Selected File plans the cursor's file only, with each
    // line's text as the search saw it.
    find.select(3);
    assert_eq!(planned(&find, true), [entry("/p/b.rs", &[2, 7])]);
    let plan = find.replace_plan(true).unwrap();
    assert_eq!(plan.files[0].lines[0], (2, "foo 2".to_owned()));
    // A new search counts every line again.
    find.query = "foo bar".into();
    let (_, _, generation) = find.next_search().unwrap();
    find.accept(generation, SearchEvent::File(hits("/p/a.rs", &[4])));
    find.accept(generation, SearchEvent::Done { truncated: false });
    assert_eq!(planned(&find, false), [entry("/p/a.rs", &[4])]);
}

#[test]
fn the_plan_waits_for_the_search_and_refuses_too_much_at_once() {
    let scope = FindScope {
        root: PathBuf::from("/p"),
        folder: None,
    };
    let refusal = |find: &FindInFiles| find.replace_plan(false).err().unwrap_or_default();
    let mut find = FindInFiles::new(scope, "foo".into());
    let (_, _, generation) = find.next_search().unwrap();
    find.accept(generation, SearchEvent::File(hits("/p/a.rs", &[0])));
    assert_eq!(refusal(&find), "Wait for the search to finish.");
    find.accept(generation, SearchEvent::Done { truncated: true });
    assert!(refusal(&find).contains("refine the search"));
    let mut find = listed(vec![hits("/p/a.rs", &[0])]);
    find.toggle_include(Path::new("/p/a.rs"), None);
    assert_eq!(refusal(&find), "Check at least one result.");
    // More files than the staging writes at once.
    let many = (0..=MAX_RENAME_FILES)
        .map(|n| hits(&format!("/p/{n}.rs"), &[0]))
        .collect();
    assert!(refusal(&listed(many)).contains("at most 64 files"));
}

#[test]
fn replace_writes_only_checked_lines_and_keeps_line_breaks() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a.rs",
        "let user_id = 1;\r\nlet other_id = 2;\r\nuser_id\n",
    );
    let b = write(dir.path(), "b.txt", "no match\n");
    let mut find = searched(dir.path(), r"(\w+)_id", true);
    find.replace.text = "${1}Id".into();
    // `other_id` (line 2) stays as it is.
    find.toggle_include(&a, Some(1));
    let prepared = prepare_replace(&find.replace_plan(false).unwrap()).unwrap();
    assert_eq!(prepared.file_count(), 1);
    assert_eq!(apply_rename(prepared).unwrap(), std::slice::from_ref(&a));
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "let userId = 1;\r\nlet other_id = 2;\r\nuserId\n"
    );
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "no match\n");
}

#[test]
fn a_file_changed_since_the_search_stops_the_replace_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.txt", "foo\n");
    let b = write(dir.path(), "b.txt", "x\nfoo\n");
    let mut find = searched(dir.path(), "foo", false);
    find.replace.text = "bar".into();
    let plan = find.replace_plan(false).unwrap();
    // Another line now sits where the planned one was.
    std::fs::write(&b, "foo\nx\n").unwrap();
    let error = prepare_replace(&plan).err().unwrap();
    assert!(error.downcast_ref::<Stale>().is_some(), "{error:#}");
    assert!(error.to_string().contains("b.txt changed since the search"));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "foo\n");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "foo\nx\n");
}

#[test]
fn a_read_only_file_is_refused_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.txt", "foo\n");
    let mut find = searched(dir.path(), "foo", false);
    find.replace.text = "bar".into();
    // Perforce leaves files that are not open for edit read-only.
    let mut permissions = std::fs::metadata(&a).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&a, permissions).unwrap();
    let error = prepare_replace(&find.replace_plan(false).unwrap())
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("a.txt is read-only"),
        "{error:#}"
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "foo\n");
}

#[test]
fn an_open_clean_buffer_takes_the_replace_as_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let path = write(&root, "a.rs", "let foo = 1;\n");
    let mut app = test_support::app(dir.path());
    let native = test_support::open(&app, &path);
    app.native_docs.insert(path.clone(), native);
    replacing(&mut app, &root, ("foo", "bar"));
    let ctx = egui::Context::default();
    app.start_replace(false);
    assert!(app.find_in_files.as_ref().unwrap().replace.busy());
    finish(&mut app, &ctx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "let bar = 1;\n");
    // Replace All closes the window and says what changed.
    assert!(!app.find_in_files.as_ref().unwrap().shown);
    assert_eq!(app.info.as_deref(), Some("Replaced 1 match in 1 file."));
    // The clean buffer follows the disk as one undoable edit.
    let mut native = app.native_docs.remove(&path).unwrap();
    assert!(native.disk.reload_wanted);
    test_support::pump(&mut native, &app, |native| {
        !native.disk.reload_wanted && !native.loading
    });
    assert!(!native.dirty());
    let buffer = native.doc.as_mut().unwrap();
    assert_eq!(buffer.text(), "let bar = 1;\n");
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "let foo = 1;\n");
}

#[test]
fn an_unsaved_buffer_stops_the_replace_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let path = write(&root, "a.rs", "let foo = 1;\n");
    let mut app = test_support::app(dir.path());
    let dirty = NativeDoc::dirty_for_test(path.clone());
    app.native_docs.insert(path.clone(), dirty);
    replacing(&mut app, &root, ("foo", "bar"));
    app.start_replace(false);
    let find = app.find_in_files.as_ref().unwrap();
    assert!(!find.replace.busy());
    let note = find.replace.note.as_ref().unwrap();
    assert!(note.error);
    assert_eq!(
        note.text,
        format!("Save {} before replacing in it.", path.display())
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "let foo = 1;\n");
}

#[test]
fn replace_in_selected_file_keeps_the_other_files_listed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let a = write(&root, "a.txt", "foo\n");
    let b = write(&root, "b.txt", "foo foo\n");
    let mut app = test_support::app(dir.path());
    replacing(&mut app, &root, ("foo", "bar"));
    let find = app.find_in_files.as_mut().unwrap();
    let at = find.files.iter().position(|file| file.path == b).unwrap();
    let before: usize = find.files[..at].iter().map(|file| file.hits.len()).sum();
    find.select(before);
    let ctx = egui::Context::default();
    app.start_replace(true);
    finish(&mut app, &ctx);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "bar bar\n");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "foo\n");
    let find = app.find_in_files.as_ref().unwrap();
    assert!(find.shown);
    let left: Vec<&Path> = find.files.iter().map(|file| file.path.as_path()).collect();
    assert_eq!(left, [a.as_path()]);
    assert_eq!(find.total, 1);
    let note = find.replace.note.as_ref().unwrap();
    assert_eq!(
        (note.text.as_str(), note.error),
        ("Replaced 2 matches in 1 file.", false)
    );
}

#[test]
fn a_replace_that_fails_after_the_window_closed_says_so_in_the_status_bar() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let path = write(&root, "a.txt", "foo\n");
    let mut app = test_support::app(dir.path());
    replacing(&mut app, &root, ("foo", "bar"));
    std::fs::write(&path, "\nfoo\n").unwrap();
    let ctx = egui::Context::default();
    app.start_replace(false);
    app.find_in_files.as_mut().unwrap().hide();
    finish(&mut app, &ctx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "\nfoo\n");
    let info = app.info.as_deref().unwrap_or_default();
    assert!(
        info.starts_with("a.txt changed since the search."),
        "{info:?}"
    );
}

#[test]
fn an_open_clean_buffer_of_a_bom_file_takes_the_replace() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let path = write(&root, "a.cs", "\u{feff}let foo = 1;\n");
    let mut app = test_support::app(dir.path());
    let native = test_support::open(&app, &path);
    app.native_docs.insert(path.clone(), native);
    replacing(&mut app, &root, ("foo", "bar"));
    let ctx = egui::Context::default();
    app.start_replace(false);
    finish(&mut app, &ctx);
    let note = app.find_in_files.as_ref().unwrap().replace.note.as_ref();
    assert!(
        note.is_none_or(|note| !note.error),
        "{:?}",
        note.map(|n| &n.text)
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "\u{feff}let bar = 1;\n"
    );
}

#[test]
fn replace_keeps_the_spaces_typed_around_the_query() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let path = write(&root, "a.c", "int x;\nprint(x);\ninteger y;\n");
    let mut app = test_support::app(dir.path());
    replacing(&mut app, &root, ("int ", "long "));
    let ctx = egui::Context::default();
    app.start_replace(false);
    finish(&mut app, &ctx);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "long x;\nprint(x);\ninteger y;\n"
    );
}

#[test]
fn out_of_date_results_search_again_when_the_replace_stops() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let path = write(&root, "a.txt", "foo\n");
    let mut app = test_support::app(dir.path());
    replacing(&mut app, &root, ("foo", "bar"));
    std::fs::write(&path, "\nfoo\n").unwrap();
    let ctx = egui::Context::default();
    app.start_replace(false);
    finish(&mut app, &ctx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "\nfoo\n");
    let find = app.find_in_files.as_ref().unwrap();
    let note = find.replace.note.as_ref().unwrap();
    assert!(note.error);
    assert!(note.text.starts_with("a.txt changed since the search."));
    // The next frame searches again.
    assert!(find.searched.is_none());
}
