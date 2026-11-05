use super::cursor::{Row, keep_visible};
use super::preview::{JobColors, line_job, preview_lines};
use super::search::{Finish, file_hits, run};
use super::view::summary;
use super::*;

fn write(root: &Path, name: &str, body: &str) {
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn request(root: &Path, text: &str, limit: usize) -> SearchRequest {
    SearchRequest {
        root: root.to_path_buf(),
        folder: root.to_path_buf(),
        query: crate::search::Query {
            text: text.into(),
            ..Default::default()
        },
        limit,
    }
}

/// Run a search to the end and collect what it streamed.
fn collect(request: &SearchRequest) -> (Vec<FileHits>, Finish) {
    let mut files = Vec::new();
    let finish = run(request, &AtomicBool::new(false), &mut |file| {
        files.push(file);
        true
    })
    .unwrap();
    (files, finish)
}

/// Paths below `root` of the files with hits, sorted.
fn names(root: &Path, files: &[FileHits]) -> Vec<String> {
    let mut names: Vec<String> = files
        .iter()
        .map(|file| {
            let relative = file.path.strip_prefix(root).unwrap();
            relative.to_string_lossy().replace('\\', "/")
        })
        .collect();
    names.sort();
    names
}

/// An app with the local project `a` at `/a` selected.
fn app_with_project() -> (App, egui::Context, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.state.projects.push(Project {
        id: "a".into(),
        name: "a".into(),
        path: "/a".into(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("a".into());
    (app, ctx, dir)
}

fn scope(root: &str) -> FindScope {
    FindScope {
        root: PathBuf::from(root),
        folder: None,
    }
}

fn file_with_hits(path: &str, lines: &[u32]) -> FileHits {
    FileHits {
        path: PathBuf::from(path),
        hits: lines
            .iter()
            .map(|&line| LineHit {
                line,
                character: 0,
                preview: format!("hit {line}"),
                matches: vec![(0, 3)],
            })
            .collect(),
        lines: lines
            .iter()
            .map(|&line| (line, format!("hit {line}")))
            .collect(),
    }
}

#[test]
fn results_group_by_file_with_utf16_columns_previews_and_match_ranges() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", "fn main() {}\n  let é = needle;\n");
    write(dir.path(), "b.txt", "needle\nno\nneedle again\n");
    let (files, finish) = collect(&request(dir.path(), "needle", 100));
    assert_eq!(finish.total, 3);
    assert!(!finish.truncated);
    let a = files
        .iter()
        .find(|file| file.path.ends_with("a.rs"))
        .unwrap();
    assert_eq!(
        a.hits,
        [LineHit {
            line: 1,
            character: 10,
            preview: "let é = needle;".into(),
            matches: vec![(8, 14)],
        }]
    );
    // The preview keeps the untrimmed lines around the hit.
    assert_eq!(a.lines.get(&0).map(String::as_str), Some("fn main() {}"));
    assert_eq!(
        a.lines.get(&1).map(String::as_str),
        Some("  let é = needle;")
    );
}

#[test]
fn search_caps_results_and_reports_truncation() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", &"hit\n".repeat(30));
    write(dir.path(), "b.txt", &"hit\n".repeat(30));
    let (files, finish) = collect(&request(dir.path(), "hit", 40));
    assert_eq!(finish.total, 40);
    assert!(finish.truncated);
    let kept: usize = files.iter().map(|file| file.hits.len()).sum();
    assert_eq!(kept, 40);
}

#[test]
fn file_mask_limits_the_walk() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "needle\n");
    write(dir.path(), "src/a.md", "needle\n");
    let mut wanted = request(dir.path(), "needle", 10);
    wanted.query.include = "*.rs".into();
    let (files, _) = collect(&wanted);
    assert_eq!(names(dir.path(), &files), ["src/a.rs"]);
}

#[test]
fn search_includes_dotfiles_but_skips_ignored_files_and_git_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (name, body) in [
        (".github/workflows/ci.yml", "needle\n"),
        (".env.example", "needle\n"),
        (".secret", "needle\n"),
        (".gitignore", ".secret\nbuild/\n"),
        ("build/out.txt", "needle\n"),
        // Git metadata: the `.git` folder and a linked worktree's file.
        (".git/HEAD", "needle\n"),
        ("sub/.git", "gitdir: needle\n"),
        ("sub/lib.rs", "needle\n"),
    ] {
        write(root, name, body);
    }
    let (files, _) = collect(&request(root, "needle", 100));
    assert_eq!(
        names(root, &files),
        [".env.example", ".github/workflows/ci.yml", "sub/lib.rs"]
    );
}

#[test]
fn search_skips_the_metadata_of_other_version_control_systems() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in [
        ".svn/pristine/ab/abcdef.svn-base",
        ".hg/store/data/a.txt.i",
        ".bzr/branch/last-revision",
        "_darcs/pristine.hashed/a",
        ".jj/repo/store/x",
        "CVS/Entries",
        "a.txt",
    ] {
        write(root, name, "needle\n");
    }
    let (files, _) = collect(&request(root, "needle", 100));
    assert_eq!(names(root, &files), ["a.txt"]);
}

#[test]
fn a_long_search_gets_no_read_deadline() {
    // The 30 s default would end the job while the walk still streams.
    assert_eq!(super::search_context(Path::new("/a")).deadline, None);
}

#[test]
fn exclude_globs_prune_folders_and_files_and_combine_with_the_mask() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in [
        "src/a.rs",
        "src/a.md",
        "src/gen/b.rs",
        "target/c.rs",
        "web/app.js",
        "web/app.min.js",
    ] {
        write(root, name, "needle\n");
    }
    let mut wanted = request(root, "needle", 100);
    wanted.query.exclude = "target, *.min.js, src/gen/**".into();
    assert_eq!(
        names(root, &collect(&wanted).0),
        ["src/a.md", "src/a.rs", "web/app.js"]
    );
    wanted.query.include = "*.rs, *.js".into();
    assert_eq!(names(root, &collect(&wanted).0), ["src/a.rs", "web/app.js"]);
}

#[test]
fn a_folder_scope_walks_only_that_folder_with_project_relative_globs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["src/ui/a.rs", "src/core/b.rs", "docs/c.rs"] {
        write(root, name, "needle\n");
    }
    let mut wanted = request(root, "needle", 100);
    wanted.folder = root.join("src");
    assert_eq!(
        names(root, &collect(&wanted).0),
        ["src/core/b.rs", "src/ui/a.rs"]
    );
    wanted.query.exclude = "src/core/**".into();
    assert_eq!(names(root, &collect(&wanted).0), ["src/ui/a.rs"]);
}

#[test]
fn files_stream_to_the_sink_and_a_closed_sink_stops_the_walk() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        write(dir.path(), name, "needle\n");
    }
    let mut seen = 0usize;
    let finish = run(
        &request(dir.path(), "needle", 100),
        &AtomicBool::new(false),
        &mut |_| {
            seen = seen.saturating_add(1);
            false
        },
    )
    .unwrap();
    assert_eq!(seen, 1);
    assert!(finish.cancelled);
}

#[test]
fn a_cancelled_search_stops_and_reports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", "needle\n");
    let mut files = Vec::new();
    let finish = run(
        &request(dir.path(), "needle", 10),
        &AtomicBool::new(true),
        &mut |file| {
            files.push(file);
            true
        },
    )
    .unwrap();
    assert!(finish.cancelled);
    assert!(files.is_empty());
}

#[test]
fn a_new_query_cancels_the_search_in_flight_and_drops_its_answers() {
    let mut find = FindInFiles::new(scope("/p"), "one".into());
    let (_, first_cancel, first) = find.next_search().unwrap();
    // Unchanged input does not search again.
    assert!(find.next_search().is_none());
    find.query = "two".into();
    let (_, second_cancel, second) = find.next_search().unwrap();
    assert!(first_cancel.load(Ordering::Relaxed));
    assert!(!second_cancel.load(Ordering::Relaxed));
    find.accept(first, SearchEvent::File(file_with_hits("/p/old.rs", &[0])));
    assert!(find.files.is_empty() && find.running);
    find.accept(
        second,
        SearchEvent::File(file_with_hits("/p/a.rs", &[0, 4])),
    );
    assert_eq!(find.total, 2);
    assert!(find.running);
    find.accept(second, SearchEvent::Done { truncated: false });
    assert!(!find.running);
    // Clearing the query cancels too and shows nothing.
    find.query.clear();
    assert!(find.next_search().is_none());
    assert!(second_cancel.load(Ordering::Relaxed));
    assert!(find.files.is_empty());
}

#[test]
fn a_cancelled_line_scan_stops_inside_the_file_even_without_matches() {
    let regex = regex::Regex::new("needle").unwrap();
    let text = "hay\n".repeat(1000);
    assert_eq!(file_hits(&text, &regex, 10, &AtomicBool::new(true)), None);
    let text = "needle\n".repeat(5);
    let hits = file_hits(&text, &regex, 3, &AtomicBool::new(false)).unwrap();
    assert_eq!(hits.len(), 3);
}

#[test]
fn arrows_move_the_cursor_through_hits_across_files_and_wrap() {
    let mut find = FindInFiles::new(scope("/p"), "hit".into());
    let (_, _, generation) = find.next_search().unwrap();
    find.accept(
        generation,
        SearchEvent::File(file_with_hits("/p/a.rs", &[2, 7])),
    );
    find.accept(
        generation,
        SearchEvent::File(file_with_hits("/p/b.rs", &[0])),
    );
    let at = |find: &FindInFiles| {
        find.selected_location()
            .map(|location| (location.path, location.line))
    };
    assert_eq!(at(&find), Some((PathBuf::from("/p/a.rs"), 2)));
    find.step(true);
    assert_eq!(at(&find), Some((PathBuf::from("/p/a.rs"), 7)));
    find.step(true);
    assert_eq!(at(&find), Some((PathBuf::from("/p/b.rs"), 0)));
    // Down on the last hit wraps to the first; Up there wraps back.
    find.step(true);
    assert_eq!(find.selected, 0);
    find.step(false);
    assert_eq!(find.selected, 2);
    assert!(find.reveal);
    // Each file adds a header row above its hits.
    assert_eq!(find.row_count(), 5);
    assert_eq!(find.row_of(0), Some(1));
    assert_eq!(find.row_of(2), Some(4));
    assert!(matches!(
        find.rows(3..5).as_slice(),
        [Row::File(file), Row::Hit { index: 2, .. }] if file.path.ends_with("b.rs")
    ));
    // A file that streams in later lands below; the cursor stays put.
    find.accept(
        generation,
        SearchEvent::File(file_with_hits("/p/c.rs", &[1])),
    );
    assert_eq!(at(&find), Some((PathBuf::from("/p/b.rs"), 0)));
    // Enter opens at the first match of the hit.
    let location = find.selected_location().unwrap();
    assert_eq!((location.line, location.character), (0, 0));
    // A new query starts again at the top.
    find.query = "other".into();
    assert!(find.next_search().is_some());
    assert_eq!(find.selected, 0);
    assert!(find.selected_location().is_none());
}

#[test]
fn keyboard_moves_scroll_only_as_far_as_needed() {
    // Already visible: no scroll.
    assert_eq!(keep_visible(44.0, 22.0, 264.0, 0.0), 0.0);
    // Below the viewport: scroll until the row's bottom shows.
    assert_eq!(keep_visible(300.0, 22.0, 264.0, 0.0), 58.0);
    // Above the viewport: scroll up to the row's top.
    assert_eq!(keep_visible(22.0, 22.0, 264.0, 100.0), 22.0);
}

#[test]
fn the_status_counts_streamed_hits_and_asks_to_refine_at_the_limit() {
    let mut find = FindInFiles::new(scope("/p"), String::new());
    assert_eq!(summary(&find), "Type to search");
    find.query = "hit".into();
    let (_, _, generation) = find.next_search().unwrap();
    assert_eq!(summary(&find), "Searching…");
    find.accept(
        generation,
        SearchEvent::File(file_with_hits("/p/a.rs", &[1, 2])),
    );
    assert_eq!(summary(&find), "Searching… 2 matches in 1 file");
    find.accept(generation, SearchEvent::Done { truncated: false });
    assert_eq!(summary(&find), "2 matches in 1 file");
    find.query = "many".into();
    let (_, _, generation) = find.next_search().unwrap();
    let lines: Vec<u32> = (0..1000).collect();
    find.accept(
        generation,
        SearchEvent::File(file_with_hits("/p/b.rs", &lines)),
    );
    find.accept(generation, SearchEvent::Done { truncated: true });
    assert_eq!(summary(&find), "1000+ results, refine the search");
}

#[test]
fn closing_keeps_the_query_and_reopening_selects_it() {
    let mut find = FindInFiles::new(scope("/p"), "needle".into());
    let (_, cancel, generation) = find.next_search().unwrap();
    find.accept(
        generation,
        SearchEvent::File(file_with_hits("/p/a.rs", &[1])),
    );
    find.hide();
    assert!(!find.shown && !find.running);
    assert!(cancel.load(Ordering::Relaxed));
    assert!(find.files.is_empty());
    find.focus = false;
    find.select_query = false;
    find.reopen(scope("/p"), String::new());
    assert!(find.shown && find.focus && find.select_query);
    assert_eq!(find.query, "needle");
    // Reopening runs the search again.
    assert!(find.next_search().is_some());
    // An editor selection replaces the query.
    find.reopen(scope("/p"), "seed".into());
    assert_eq!(find.query, "seed");
}

#[test]
fn the_preview_shows_lines_around_the_hit_with_matches_and_colors() {
    let dir = tempfile::tempdir().unwrap();
    let body: String = (1..=20)
        .map(|n| {
            if n == 10 {
                "    let needle = 1;\n".to_owned()
            } else {
                format!("let line_{n} = {n};\n")
            }
        })
        .collect();
    write(dir.path(), "a.rs", &body);
    let (files, _) = collect(&request(dir.path(), "needle", 10));
    let (file, hit) = (&files[0], &files[0].hits[0]);
    assert_eq!(hit.line, 9);
    let lines = preview_lines(file, hit);
    let numbers: Vec<u32> = lines.iter().map(|line| line.number).collect();
    assert_eq!(numbers, (4..=14).collect::<Vec<u32>>());
    let own = lines.iter().find(|line| line.number == 9).unwrap();
    assert_eq!(own.text, "    let needle = 1;");
    // The list shows `let needle = 1;`; the preview line keeps the indent.
    assert_eq!(own.matches, [(8, 14)]);
    assert!(
        lines
            .iter()
            .filter(|line| line.number != 9)
            .all(|line| line.matches.is_empty())
    );
    let mut find = FindInFiles::new(scope(&dir.path().to_string_lossy()), "needle".into());
    let (_, _, generation) = find.next_search().unwrap();
    for file in files {
        find.accept(generation, SearchEvent::File(file));
    }
    let preview = find.preview(true).unwrap();
    assert_eq!(preview.line, 9);
    assert!(
        preview.lines.iter().any(|line| !line.spans.is_empty()),
        "rust preview is highlighted"
    );
}

#[test]
fn preview_text_keeps_syntax_colors_and_puts_matches_on_the_found_background() {
    let plain = egui::Color32::from_rgb(1, 2, 3);
    let found = egui::Color32::from_rgb(9, 9, 9);
    let keyword = egui::Color32::from_rgb(200, 0, 0);
    let spans = [terminator_native_edit::highlight::Span {
        start: 0,
        end: 3,
        color: [200, 0, 0],
        style: Default::default(),
    }];
    let job = line_job("let needle", &spans, &[(4, 10)], JobColors { plain, found });
    assert_eq!(job.text, "let needle");
    // `let`, the space, then `needle`.
    let looks: Vec<_> = job
        .sections
        .iter()
        .map(|section| (section.format.color, section.format.background))
        .collect();
    let clear = egui::Color32::TRANSPARENT;
    assert_eq!(looks, [(keyword, clear), (plain, clear), (plain, found)]);
}

#[test]
fn scope_is_the_project_or_a_folder_inside_it() {
    let project = Path::new("/p");
    let at = |root: &str, folder: Option<&str>| {
        Some(FindScope {
            root: root.into(),
            folder: folder.map(PathBuf::from),
        })
    };
    assert_eq!(FindScope::resolve(Some(project), None), at("/p", None));
    assert_eq!(
        FindScope::resolve(Some(project), Some(Path::new("/p/src"))),
        at("/p", Some("/p/src"))
    );
    assert_eq!(
        FindScope::resolve(Some(project), Some(project)),
        at("/p", None)
    );
    // A folder outside the project (`/p2` is not inside `/p`) is its own root.
    assert_eq!(
        FindScope::resolve(Some(project), Some(Path::new("/p2/src"))),
        at("/p2/src", None)
    );
    assert_eq!(
        FindScope::resolve(None, Some(Path::new("/x"))),
        at("/x", None)
    );
    assert_eq!(FindScope::resolve(None, None), None);
}

#[test]
fn the_action_searches_a_focused_explorer_folder_else_the_project() {
    let (mut app, ctx, _dir) = app_with_project();
    let entry = |path: &str, directory| terminator_git::Entry {
        path: path.into(),
        directory,
        ignored: false,
    };
    // Project `a` lives at `/a`; the Explorer lists a folder and a file.
    app.dirs.insert(
        PathBuf::from("/a"),
        vec![entry("/a/src", true), entry("/a/README.md", false)],
    );
    app.explorer_selection = vec![PathBuf::from("/a/src")];
    // Each pick opens a closed window again.
    let open = |app: &mut App, ctx: &egui::Context| {
        if let Some(find) = app.find_in_files.as_mut() {
            find.hide();
        }
        app.find_in_files_action(ctx);
        let find = app.find_in_files.as_ref().unwrap();
        (find.root.clone(), find.folder.clone())
    };
    // Without Explorer focus, the action searches the project.
    assert_eq!(open(&mut app, &ctx), (PathBuf::from("/a"), None));
    // With the tree focused, its selected folder narrows the search.
    let tree = egui::Id::unique("explorer-tree-under-test");
    ctx.memory_mut(|memory| memory.request_focus(tree));
    note_explorer_focus(&ctx, tree);
    let src = (PathBuf::from("/a"), Some(PathBuf::from("/a/src")));
    assert_eq!(open(&mut app, &ctx), src);
    // A selected file is no scope.
    app.explorer_selection = vec![PathBuf::from("/a/README.md")];
    assert_eq!(open(&mut app, &ctx), (PathBuf::from("/a"), None));
    app.explorer_selection = vec![PathBuf::from("/a/src")];
    assert_eq!(open(&mut app, &ctx), src);
    // Focus moves on (to the query field, say). The open window keeps its
    // folder and query; the action only takes the keyboard back.
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("other")));
    app.find_in_files.as_mut().unwrap().query = "typed".into();
    app.find_in_files_action(&ctx);
    let find = app.find_in_files.as_ref().unwrap();
    assert_eq!(
        (find.folder.clone(), find.query.as_str()),
        (src.1.clone(), "typed")
    );
    assert!(find.focus && find.select_query);
    // An editor selection replaces the query and keeps the folder too.
    app.open_find_in_files(Some("a"), "picked".into());
    let find = app.find_in_files.as_ref().unwrap();
    assert_eq!(
        (find.folder.clone(), find.query.as_str()),
        (src.1, "picked")
    );
    // Once closed, the unfocused folder no longer counts.
    assert_eq!(open(&mut app, &ctx), (PathBuf::from("/a"), None));
}

#[test]
fn an_editor_seed_fills_the_query_and_remote_projects_get_a_note() {
    let (mut app, ctx, _dir) = app_with_project();
    app.open_find_in_files(Some("a"), "needle".into());
    let find = app.find_in_files.as_ref().unwrap();
    assert_eq!(
        (find.query.as_str(), find.root.as_path()),
        ("needle", Path::new("/a"))
    );
    assert!(find.shown && find.select_query);
    app.find_in_files.as_mut().unwrap().hide();
    let mut remote = crate::remote_ui::blank_project();
    remote.id = "r".into();
    remote.root = "/srv/app".into();
    app.state.remote_projects.push(remote);
    app.open_find_in_files(Some("r"), "needle".into());
    assert!(!app.find_in_files.as_ref().unwrap().shown);
    assert_eq!(
        app.info.as_deref(),
        Some("Find in Files searches local projects only")
    );
    app.info = None;
    app.selected = Some("r".into());
    app.find_in_files_action(&ctx);
    assert!(!app.find_in_files.as_ref().unwrap().shown);
    assert_eq!(
        app.info.as_deref(),
        Some("Find in Files searches local projects only")
    );
}

/// One frame of the window with `keys` pressed (no modifiers).
fn paint(app: &mut App, ctx: &egui::Context, keys: &[egui::Key]) {
    let events = keys
        .iter()
        .map(|&key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        })
        .collect();
    let input = egui::RawInput {
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.show_find_in_files_window(ui.ctx()));
    output.textures_delta.clear();
}

/// Every string one frame of the window paints.
fn painted_text(app: &mut App, ctx: &egui::Context) -> Vec<String> {
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.show_find_in_files_window(ui.ctx());
    });
    output.textures_delta.clear();
    let mut strings = Vec::new();
    let mut shapes: Vec<&egui::Shape> = output.shapes.iter().map(|clip| &clip.shape).collect();
    while let Some(shape) = shapes.pop() {
        match shape {
            egui::Shape::Vec(inner) => shapes.extend(inner),
            egui::Shape::Text(text) => strings.push(text.galley.text().to_owned()),
            _ => {}
        }
    }
    strings
}

#[test]
fn replace_mode_shows_each_change_and_keeps_the_window_keys() {
    let (mut app, ctx, _dir) = app_with_project();
    app.open_replace_in_files(Some("a"), "hit".into());
    let find = app.find_in_files.as_mut().unwrap();
    assert!(find.replace.on);
    let (_, _, generation) = find.next_search().unwrap();
    for file in [
        file_with_hits("/a/x.rs", &[2, 7]),
        file_with_hits("/a/y.rs", &[0]),
    ] {
        find.accept(generation, SearchEvent::File(file));
    }
    find.accept(generation, SearchEvent::Done { truncated: false });
    find.replace.text = "pin".into();
    // A new window takes a few frames to grow to its content.
    for _ in 0..3 {
        paint(&mut app, &ctx, &[]);
    }
    // A seeded query sends the keyboard to the replacement.
    let field = egui::Id::unique(("find-in-files", "replace"));
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(field));
    // The cursor's file, each line after the replace, and the count.
    let has = |text: &[String], wanted: &str| text.iter().any(|line| line == wanted);
    let text = painted_text(&mut app, &ctx);
    assert!(has(&text, "x.rs: 2 of 2 lines change"), "{text:?}");
    assert!(has(&text, "pin 2") && has(&text, "pin 7"), "{text:?}");
    assert!(has(&text, "3 lines in 2 files change"), "{text:?}");
    // The arrows still move the cursor from the replacement field.
    paint(
        &mut app,
        &ctx,
        &[egui::Key::ArrowDown, egui::Key::ArrowDown],
    );
    assert_eq!(app.find_in_files.as_ref().unwrap().selected, 2);
    let text = painted_text(&mut app, &ctx);
    assert!(has(&text, "y.rs: 1 of 1 lines change"), "{text:?}");
    // An unchecked line is counted out.
    let find = app.find_in_files.as_mut().unwrap();
    find.toggle_include(Path::new("/a/x.rs"), Some(7));
    let text = painted_text(&mut app, &ctx);
    assert!(has(&text, "2 lines in 2 files change"), "{text:?}");
}

/// Open the window on two hits in `/a/x.rs`, answered by hand, and let
/// the query field take focus.
fn open_on_two_hits(app: &mut App, ctx: &egui::Context) {
    app.open_find_in_files(Some("a"), "hit".into());
    let find = app.find_in_files.as_mut().unwrap();
    let (_, _, generation) = find.next_search().unwrap();
    find.accept(
        generation,
        SearchEvent::File(file_with_hits("/a/x.rs", &[2, 7])),
    );
    find.accept(generation, SearchEvent::Done { truncated: false });
    paint(app, ctx, &[]);
    paint(app, ctx, &[]);
}

#[test]
fn window_keys_move_the_cursor_open_the_hit_and_close() {
    let (mut app, ctx, _dir) = app_with_project();
    app.state.settings.editor_mode = EditorMode::Native;
    open_on_two_hits(&mut app, &ctx);
    paint(&mut app, &ctx, &[egui::Key::ArrowDown]);
    let find = app.find_in_files.as_ref().unwrap();
    assert_eq!(find.selected, 1);
    assert!(find.shown);
    // Two Ups: one to the first hit, one wrapping to the last.
    paint(&mut app, &ctx, &[egui::Key::ArrowUp, egui::Key::ArrowUp]);
    assert_eq!(app.find_in_files.as_ref().unwrap().selected, 1);
    // Enter opens the hit at its line and closes the window.
    paint(&mut app, &ctx, &[egui::Key::Enter]);
    assert!(!app.find_in_files.as_ref().unwrap().shown);
    assert_eq!(app.native_pending_line.get(Path::new("/a/x.rs")), Some(&7));
    // Esc closes without opening anything and gives the keyboard back.
    app.native_pending_line.clear();
    let editor = egui::Id::unique("editor-under-test");
    ctx.memory_mut(|memory| memory.request_focus(editor));
    open_on_two_hits(&mut app, &ctx);
    assert!(app.find_in_files.as_ref().unwrap().shown);
    assert_ne!(ctx.memory(|memory| memory.focused()), Some(editor));
    paint(&mut app, &ctx, &[egui::Key::Escape]);
    assert!(!app.find_in_files.as_ref().unwrap().shown);
    assert!(app.native_pending_line.is_empty());
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(editor));
}
