use super::search::{LineHit, run};
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
        query: crate::search::Query {
            text: text.into(),
            ..Default::default()
        },
        limit,
    }
}

#[test]
fn results_group_by_file_with_utf16_columns_and_previews() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", "fn main() {}\n  let é = needle;\n");
    write(dir.path(), "b.txt", "needle\nno\nneedle again\n");
    let outcome = run(&request(dir.path(), "needle", 100), &AtomicBool::new(false)).unwrap();
    assert_eq!(outcome.total, 3);
    assert!(!outcome.truncated);
    let a = outcome
        .files
        .iter()
        .find(|file| file.path.ends_with("a.rs"))
        .unwrap();
    assert_eq!(
        a.hits,
        [LineHit {
            line: 1,
            character: 10,
            preview: "let é = needle;".into(),
        }]
    );
}

#[test]
fn search_caps_results_and_reports_truncation() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", &"hit\n".repeat(30));
    write(dir.path(), "b.txt", &"hit\n".repeat(30));
    let outcome = run(&request(dir.path(), "hit", 40), &AtomicBool::new(false)).unwrap();
    assert_eq!(outcome.total, 40);
    assert!(outcome.truncated);
    let kept: usize = outcome.files.iter().map(|file| file.hits.len()).sum();
    assert_eq!(kept, 40);
}

#[test]
fn file_mask_limits_the_walk() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "needle\n");
    write(dir.path(), "src/a.md", "needle\n");
    let mut wanted = request(dir.path(), "needle", 10);
    wanted.query.include = "*.rs".into();
    let outcome = run(&wanted, &AtomicBool::new(false)).unwrap();
    assert_eq!(outcome.files.len(), 1);
    assert!(outcome.files[0].path.ends_with("src/a.rs"));
}

#[test]
fn a_cancelled_search_stops_and_reports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", "needle\n");
    let outcome = run(&request(dir.path(), "needle", 10), &AtomicBool::new(true)).unwrap();
    assert!(outcome.cancelled);
    assert!(outcome.files.is_empty());
}

#[test]
fn a_new_query_cancels_the_search_in_flight_and_drops_its_answer() {
    let mut find = FindInFiles::new(PathBuf::from("/p"), "one".into());
    let (_, first_cancel, first) = find.next_search().unwrap();
    // Unchanged input does not search again.
    assert!(find.next_search().is_none());
    find.query = "two".into();
    let (_, second_cancel, second) = find.next_search().unwrap();
    assert!(first_cancel.load(Ordering::Relaxed));
    assert!(!second_cancel.load(Ordering::Relaxed));
    find.accept(first, Ok(SearchOutcome::default()));
    assert!(find.outcome.is_none() && find.running);
    find.accept(second, Ok(SearchOutcome::default()));
    assert!(find.outcome.is_some() && !find.running);
    // Clearing the query cancels too and shows nothing.
    find.query.clear();
    assert!(find.next_search().is_none());
    assert!(second_cancel.load(Ordering::Relaxed));
    assert!(find.outcome.is_none());
}
