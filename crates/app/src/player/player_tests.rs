use super::*;
use engine::Status;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_samples_writes_wav_files() {
        let dir = tempfile::tempdir().unwrap();
        let tracks = install_samples(dir.path());
        assert_eq!(tracks.len(), 3);
        for path in &tracks {
            assert!(path.is_file());
            assert!(supported(path));
        }
    }

    #[test]
    fn audio_extensions_are_supported() {
        assert!(supported(Path::new("song.MP3")));
        assert!(supported(Path::new("/tmp/a.flac")));
        assert!(!supported(Path::new("song.mp4")));
        assert!(!supported(Path::new("readme.md")));
    }

    fn tracks() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/a/one.mp3"),
            PathBuf::from("/a/two.mp3"),
            PathBuf::from("/a/three.mp3"),
        ]
    }

    #[test]
    fn finishing_a_track_keeps_chrome_until_the_next_starts() {
        let mut player = Controller::finished_fixture("a", Some(0));
        player.status = Status::Playing {
            title: "one".into(),
            position: Duration::from_secs(3),
            duration: Some(Duration::from_secs(3)),
            seekable: true,
        };
        let next = player.poll(PollInput {
            project: "a",
            playlist: &tracks(),
            shuffle: false,
            repeat: false,
        });
        assert_eq!(next, Some(1));
        assert!(!matches!(player.status, Status::Stopped));
    }

    #[test]
    fn sequential_next_stops_at_the_end_without_repeat() {
        let mut player = Controller::finished_fixture("a", Some(2));
        player.current_path = Some(PathBuf::from("/a/three.mp3"));
        let next = player.play_offset(Skip {
            project: "a",
            playlist: &tracks(),
            delta: 1,
            shuffle: false,
            repeat: false,
        });
        assert!(next.is_none());
        assert!(matches!(player.status, Status::Stopped));
    }

    #[test]
    fn sequential_next_wraps_when_repeat_is_on() {
        let mut player = Controller::finished_fixture("a", Some(2));
        let next = player.play_offset(Skip {
            project: "a",
            playlist: &tracks(),
            delta: 1,
            shuffle: false,
            repeat: true,
        });
        assert_eq!(next, Some(0));
    }

    #[test]
    fn shuffle_picks_a_different_remaining_track() {
        let mut player = Controller::finished_fixture("a", Some(0));
        player.rng = 1;
        let next = player.play_offset(Skip {
            project: "a",
            playlist: &tracks(),
            delta: 1,
            shuffle: true,
            repeat: false,
        });
        assert_ne!(next, Some(0));
        assert!(next.is_some());
    }

    #[test]
    fn shift_click_selects_a_range() {
        let mut player = Controller::finished_fixture("a", Some(0));
        player.click_track(0, false, false);
        player.click_track(2, true, false);
        assert_eq!(player.selected, HashSet::from([0, 1, 2]));
    }
}
