use super::*;

const NOW: u64 = 100 * DAY_MS;
const EXPIRED: u64 = NOW - 10 * DAY_MS;
const RECENT: u64 = NOW - DAY_MS;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::in_data(dir.path());
    (dir, store)
}

fn record(store: &Store, path: &Path, bytes: &[u8], now_ms: u64) -> Option<Entry> {
    store
        .record(Record {
            path,
            bytes,
            source: Source::External,
            label: None,
            now_ms,
        })
        .unwrap()
}

#[test]
fn identical_content_is_not_duplicated() {
    let (_dir, store) = store();
    let path = Path::new("/p/a.rs");
    assert!(record(&store, path, b"one", 1).is_some());
    assert!(record(&store, path, b"one", 2).is_none());
    assert!(record(&store, path, b"two", 3).is_some());
    assert!(record(&store, path, b"one", 4).is_some());
    assert_eq!(store.list(path).unwrap().len(), 3);
}

#[test]
fn binary_and_large_files_are_skipped() {
    let (_dir, store) = store();
    let path = Path::new("/p/bin");
    assert!(record(&store, path, b"a\0b", 1).is_none());
    assert!(record(&store, path, &[0xff, 0xfe], 1).is_none());
    let large = vec![b'a'; MAX_BYTES + 1];
    assert!(record(&store, path, &large, 1).is_none());
    assert!(!store.has_history(path));
}

#[test]
fn entries_list_newest_first_and_read_back() {
    let (_dir, store) = store();
    let path = Path::new("/p/a.rs");
    record(&store, path, b"old", 10);
    record(&store, path, b"new", 20);
    let entries = store.list(path).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].timestamp_ms, 20);
    assert_eq!(store.read(&entries[0].id).unwrap(), "new");
    assert_eq!(store.read(&entries[1].id).unwrap(), "old");
    assert!(store.list(Path::new("/p/other.rs")).unwrap().is_empty());
}

#[test]
fn labels_appear_in_the_timeline_of_files_in_the_project() {
    let (_dir, store) = store();
    let path = Path::new("/p/src/a.rs");
    record(&store, path, b"old", 10);
    store.label(Path::new("/p"), BEFORE_AGENT_RUN, 15).unwrap();
    store.label(Path::new("/q"), "elsewhere", 16).unwrap();
    record(&store, path, b"new", 20);
    let entries = store.list(path).unwrap();
    let sources: Vec<_> = entries.iter().map(|e| e.source).collect();
    assert_eq!(
        sources,
        [Source::External, Source::Label, Source::External],
        "{entries:?}"
    );
    assert_eq!(entries[1].label.as_deref(), Some(BEFORE_AGENT_RUN));
}

#[test]
fn path_traversal_ids_are_rejected() {
    let (_dir, store) = store();
    for id in [
        "../lock",
        "/etc/passwd",
        "",
        &"A".repeat(64),
        &"0".repeat(63),
    ] {
        assert!(matches!(store.read(id), Err(LocalHistoryError::InvalidId)));
    }
    assert!(matches!(
        store.read(&"0".repeat(64)),
        Err(LocalHistoryError::Missing)
    ));
}

#[test]
fn prune_removes_only_expired_entries_and_orphaned_blobs() {
    let (dir, store) = store();
    let path = Path::new("/p/a.rs");
    let old = record(&store, path, b"old", EXPIRED).unwrap();
    let kept = record(&store, path, b"kept", RECENT).unwrap();
    store.label(Path::new("/p"), "old label", EXPIRED).unwrap();
    let orphan = dir.path().join(DIR).join("blobs").join(hash(b"orphan"));
    fs::write(&orphan, b"orphan").unwrap();
    let pruned = store.prune(NOW, week()).unwrap();
    assert_eq!(
        pruned,
        Pruned {
            entries: 1,
            blobs: 2
        }
    );
    let entries = store.list(path).unwrap();
    assert_eq!(entries, vec![kept.clone()]);
    assert!(matches!(
        store.read(&old.id),
        Err(LocalHistoryError::Missing)
    ));
    assert_eq!(store.read(&kept.id).unwrap(), "kept");
    assert!(!orphan.exists());
}

#[test]
fn prune_with_zero_days_keeps_everything() {
    let (_dir, store) = store();
    let path = Path::new("/p/a.rs");
    record(&store, path, b"old", 1);
    let off = Limits {
        max_age_days: 0,
        total_bytes: 0,
        file_entries: 0,
    };
    assert_eq!(store.prune(NOW, off).unwrap(), Pruned::default());
    assert_eq!(store.list(path).unwrap().len(), 1);
}

fn week() -> Limits {
    Limits::new(7, DEFAULT_MIB, DEFAULT_FILE_ENTRIES)
}

#[test]
fn prune_keeps_only_the_newest_entries_of_each_file() {
    let (_dir, store) = store();
    let path = Path::new("/p/notes.txt");
    for n in 0..5u8 {
        record(&store, path, &[b'a' + n], RECENT + u64::from(n));
    }
    let limits = Limits {
        file_entries: 2,
        ..week()
    };
    let pruned = store.prune(NOW, limits).unwrap();
    assert_eq!(pruned.entries, 3);
    let texts: Vec<String> = store
        .list(path)
        .unwrap()
        .iter()
        .map(|e| store.read(&e.id).unwrap())
        .collect();
    assert_eq!(texts, ["e", "d"]);
}

#[test]
fn prune_drops_the_oldest_entries_of_all_files_until_under_the_size_budget() {
    let (_dir, store) = store();
    let (a, b) = (Path::new("/p/a.log"), Path::new("/p/b.log"));
    record(&store, a, &[b'1'; 400], RECENT);
    record(&store, b, &[b'2'; 400], RECENT + 1);
    record(&store, a, &[b'3'; 400], RECENT + 2);
    // A blob two entries share counts once.
    record(&store, b, &[b'3'; 400], RECENT + 3);
    let limits = Limits {
        total_bytes: 800,
        ..week()
    };
    let pruned = store.prune(NOW, limits).unwrap();
    assert_eq!(
        pruned,
        Pruned {
            entries: 1,
            blobs: 1
        }
    );
    assert_eq!(store.list(a).unwrap().len(), 1, "the oldest of a is gone");
    assert_eq!(store.list(b).unwrap().len(), 2);
    let tight = Limits {
        total_bytes: 400,
        ..week()
    };
    store.prune(NOW, tight).unwrap();
    assert!(store.list(b).unwrap().len() == 1 && store.list(a).unwrap().len() == 1);
    assert_eq!(
        store.read(&store.list(a).unwrap()[0].id).unwrap(),
        "3".repeat(400)
    );
}

#[test]
fn list_page_returns_the_timeline_in_pages_with_the_total() {
    let (_dir, store) = store();
    let path = Path::new("/p/a.rs");
    for n in 0..5u8 {
        record(&store, path, &[b'a' + n], u64::from(n));
    }
    let (page, total) = store.list_page(path, 0, 2).unwrap();
    assert_eq!((page.len(), total), (2, 5));
    assert_eq!(page[0].timestamp_ms, 4, "newest first");
    let (last, _) = store.list_page(path, 4, 2).unwrap();
    assert_eq!(last.len(), 1);
    assert!(store.list_page(path, 9, 2).unwrap().0.is_empty());
}

#[test]
fn a_shared_blob_survives_when_one_user_expires() {
    let (_dir, store) = store();
    let (a, b) = (Path::new("/p/a"), Path::new("/p/b"));
    let expired = record(&store, a, b"same", EXPIRED).unwrap();
    record(&store, b, b"same", NOW).unwrap();
    store.prune(NOW, week()).unwrap();
    assert!(!store.has_history(a));
    assert_eq!(store.read(&expired.id).unwrap(), "same");
}

#[cfg(unix)]
#[test]
fn store_files_are_private() {
    let (dir, store) = store();
    let path = Path::new("/p/a.rs");
    let entry = record(&store, path, b"secret", 1).unwrap();
    let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let root = dir.path().join(DIR);
    assert_eq!(mode(root.clone()), 0o700);
    assert_eq!(mode(root.join("blobs").join(entry.id)), 0o600);
    assert_eq!(mode(store.index_path(path)), 0o600);
}

#[test]
fn torn_index_lines_are_skipped() {
    let (_dir, store) = store();
    let path = Path::new("/p/a.rs");
    record(&store, path, b"one", 1);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(store.index_path(path))
        .unwrap();
    file.write_all(b"{\"id\":").unwrap();
    assert_eq!(store.list(path).unwrap().len(), 1);
}
