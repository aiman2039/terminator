//! Named player playlists stored with the UI preferences.
use super::{Playlist, UiPreferences};
use std::path::PathBuf;

impl UiPreferences {
    pub(super) fn migrate_playlists(&mut self) {
        if self.playlists.is_empty() && !self.player_playlists.is_empty() {
            let mut tracks = Vec::new();
            let mut owners: Vec<_> = self.player_playlists.keys().cloned().collect();
            owners.sort();
            for owner in owners {
                let Some(list) = self.player_playlists.get(&owner) else {
                    continue;
                };
                for path in list {
                    if !tracks.contains(path) {
                        tracks.push(path.clone());
                    }
                }
            }
            if !tracks.is_empty() {
                self.playlists.push(Playlist {
                    name: "Default".into(),
                    tracks,
                });
                if self.selected_playlist.is_empty() {
                    self.selected_playlist = "Default".into();
                }
            }
        }
        self.player_playlists.clear();
        if self
            .playlists
            .iter()
            .all(|playlist| playlist.name != self.selected_playlist)
        {
            self.selected_playlist = self
                .playlists
                .first()
                .map(|playlist| playlist.name.clone())
                .unwrap_or_default();
        }
    }

    pub fn ensure_default_playlist(&mut self) {
        if self.playlists.is_empty() {
            self.playlists.push(Playlist {
                name: "Default".into(),
                tracks: Vec::new(),
            });
        }
        if self
            .playlists
            .iter()
            .all(|playlist| playlist.name != self.selected_playlist)
            && let Some(playlist) = self.playlists.first()
        {
            self.selected_playlist.clone_from(&playlist.name);
        }
    }

    pub fn selected_tracks(&self) -> &[PathBuf] {
        self.playlists
            .iter()
            .find(|playlist| playlist.name == self.selected_playlist)
            .map(|playlist| playlist.tracks.as_slice())
            .unwrap_or(&[])
    }

    pub fn selected_tracks_mut(&mut self) -> Option<&mut Vec<PathBuf>> {
        let name = self.selected_playlist.clone();
        self.playlists
            .iter_mut()
            .find(|playlist| playlist.name == name)
            .map(|playlist| &mut playlist.tracks)
    }

    pub fn create_playlist(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.playlists.iter().any(|playlist| playlist.name == name) {
            return false;
        }
        self.playlists.push(Playlist {
            name: name.into(),
            tracks: Vec::new(),
        });
        self.selected_playlist = name.into();
        true
    }

    pub fn rename_selected_playlist(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty()
            || self
                .playlists
                .iter()
                .any(|playlist| playlist.name == name && playlist.name != self.selected_playlist)
        {
            return false;
        }
        let Some(playlist) = self
            .playlists
            .iter_mut()
            .find(|playlist| playlist.name == self.selected_playlist)
        else {
            return false;
        };
        if let Some(index) = self.player_index.remove(&playlist.name) {
            self.player_index.insert(name.into(), index);
        }
        playlist.name = name.into();
        self.selected_playlist = name.into();
        true
    }

    pub fn delete_selected_playlist(&mut self) {
        let name = self.selected_playlist.clone();
        self.playlists.retain(|playlist| playlist.name != name);
        self.player_index.remove(&name);
        self.selected_playlist = self
            .playlists
            .first()
            .map(|playlist| playlist.name.clone())
            .unwrap_or_default();
    }
}
