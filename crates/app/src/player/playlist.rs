//! Playlist mutations. No GUI.
use super::supported;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const AUDIO_WALK_CAP: usize = 2000;

pub fn collect_audio(root: &Path, cap: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if out.len() >= cap {
                out.sort();
                return out;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && supported(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

pub fn next_list_name(names: &[String]) -> String {
    let mut n = 1_u32;
    loop {
        let name = format!("Playlist {n}");
        if names.iter().all(|existing| existing != &name) {
            return name;
        }
        n = n.saturating_add(1);
    }
}

pub fn follow_path(tracks: &[PathBuf], path: Option<&Path>) -> Option<usize> {
    path.and_then(|path| tracks.iter().position(|track| track.as_path() == path))
}

pub struct RemoveOut {
    pub tracks: Vec<PathBuf>,
    pub current: Option<usize>,
    pub stop: bool,
}

pub fn remove_tracks(
    tracks: Vec<PathBuf>,
    drop: &HashSet<usize>,
    playing: Option<&Path>,
) -> RemoveOut {
    let tracks: Vec<PathBuf> = tracks
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !drop.contains(index))
        .map(|(_, path)| path)
        .collect();
    let current = follow_path(&tracks, playing);
    let stop = playing.is_some() && current.is_none();
    RemoveOut {
        tracks,
        current,
        stop,
    }
}

pub fn crop_drop(len: usize, keep: &HashSet<usize>) -> HashSet<usize> {
    (0..len).filter(|index| !keep.contains(index)).collect()
}

pub fn invert_selection(len: usize, selected: &HashSet<usize>) -> HashSet<usize> {
    (0..len).filter(|index| !selected.contains(index)).collect()
}

pub fn range_selection(anchor: usize, index: usize) -> HashSet<usize> {
    let start = anchor.min(index);
    let end = anchor.max(index);
    (start..=end).collect()
}

pub fn sort_by_title(tracks: &mut [PathBuf]) {
    tracks.sort_by(|left, right| {
        file_title(left)
            .to_lowercase()
            .cmp(&file_title(right).to_lowercase())
            .then_with(|| left.cmp(right))
    });
}

pub fn file_title(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub fn next_rng(seed: &mut u64) -> u64 {
    let mut x = (*seed).max(1);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    x
}

pub fn fisher_yates<T>(items: &mut [T], seed: &mut u64) {
    if items.len() < 2 {
        return;
    }
    for i in (1..items.len()).rev() {
        let j = (next_rng(seed) as usize) % (i + 1);
        items.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_audio_finds_nested_tracks_and_skips_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("album");
        fs::create_dir(&nested).unwrap();
        fs::write(dir.path().join("readme.txt"), b"x").unwrap();
        fs::write(dir.path().join("one.mp3"), b"x").unwrap();
        fs::write(nested.join("two.FLAC"), b"x").unwrap();
        let found = collect_audio(dir.path(), 10);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|path| path.ends_with("one.mp3")));
        assert!(found.iter().any(|path| path.ends_with("two.FLAC")));
    }

    #[test]
    fn remove_tracks_stops_when_the_playing_file_is_dropped() {
        let tracks = vec![PathBuf::from("a.mp3"), PathBuf::from("b.mp3")];
        let drop = HashSet::from([0]);
        let out = remove_tracks(tracks, &drop, Some(Path::new("a.mp3")));
        assert_eq!(out.tracks, [PathBuf::from("b.mp3")]);
        assert!(out.stop);
        assert_eq!(out.current, None);
    }

    #[test]
    fn remove_tracks_rewrites_the_playing_index() {
        let tracks = vec![
            PathBuf::from("a.mp3"),
            PathBuf::from("b.mp3"),
            PathBuf::from("c.mp3"),
        ];
        let drop = HashSet::from([0]);
        let out = remove_tracks(tracks, &drop, Some(Path::new("c.mp3")));
        assert!(!out.stop);
        assert_eq!(out.current, Some(1));
    }

    #[test]
    fn crop_keeps_only_selected_indices() {
        let drop = crop_drop(4, &HashSet::from([1, 2]));
        assert_eq!(drop, HashSet::from([0, 3]));
    }

    #[test]
    fn unique_playlist_names_skip_existing() {
        let names = vec!["Playlist 2".into()];
        assert_eq!(next_list_name(&names), "Playlist 1");
    }
}
