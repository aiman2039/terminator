use super::*;

#[test]
fn moved_maps_the_path_and_everything_under_it() {
    let (from, to) = (Path::new("/r/a"), Path::new("/r/b"));
    assert_eq!(
        moved(Path::new("/r/a"), from, to),
        Some(PathBuf::from("/r/b"))
    );
    assert_eq!(
        moved(Path::new("/r/a/x.rs"), from, to),
        Some(PathBuf::from("/r/b/x.rs"))
    );
    assert_eq!(moved(Path::new("/r/ab"), from, to), None);
}
