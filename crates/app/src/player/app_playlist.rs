use super::eq::EqPreset;
use super::*;
use crate::preferences::RadioStation;
use engine::Status;
use std::collections::HashSet;
use std::path::PathBuf;
impl App {
    pub(super) fn player_play_index(&mut self, project: &str, index: usize) {
        let playlist = self.playlist();
        let Some(path) = playlist.get(index).cloned() else {
            return;
        };
        self.player.play_file(project, path, index);
        self.preferences
            .player_index
            .insert(self.preferences.selected_playlist.clone(), index);
    }

    pub(super) fn player_apply_remove(&mut self, drop: HashSet<usize>) {
        if drop.is_empty() {
            return;
        }
        let playing = self.player.current_path.clone();
        let Some(tracks) = self.preferences.selected_tracks_mut() else {
            return;
        };
        let result = playlist::remove_tracks(std::mem::take(tracks), &drop, playing.as_deref());
        *tracks = result.tracks;
        self.player.selected.clear();
        self.player.last_clicked = None;
        self.player.shuffle_bag.clear();
        if result.stop {
            self.player.stop();
        } else {
            self.player.file_index = result.current;
        }
    }

    pub(super) fn player_rem_selected(&mut self) {
        let drop = self.player.selected.clone();
        self.player_apply_remove(drop);
    }

    pub(super) fn player_crop(&mut self) {
        if self.player.selected.is_empty() {
            return;
        }
        let len = self.playlist().len();
        let drop = playlist::crop_drop(len, &self.player.selected);
        self.player_apply_remove(drop);
    }

    pub(super) fn player_clear_tracks(&mut self) {
        let len = self.playlist().len();
        self.player_apply_remove((0..len).collect());
    }

    pub(super) fn player_sel_all(&mut self) {
        let len = self.playlist().len();
        self.player.selected = (0..len).collect();
    }

    pub(super) fn player_sel_none(&mut self) {
        self.player.selected.clear();
    }

    pub(super) fn player_sel_invert(&mut self) {
        let len = self.playlist().len();
        self.player.selected = playlist::invert_selection(len, &self.player.selected);
    }

    pub(super) fn player_rewrite_tracks(
        &mut self,
        mut rewrite: impl FnMut(&mut Vec<PathBuf>, &mut u64),
    ) {
        let playing = self.player.current_path.clone();
        let mut rng = self.player.rng;
        let index = {
            let Some(tracks) = self.preferences.selected_tracks_mut() else {
                return;
            };
            rewrite(tracks, &mut rng);
            playlist::follow_path(tracks, playing.as_deref())
        };
        self.player.rng = rng;
        self.player.file_index = index;
        self.player.selected.clear();
        self.player.shuffle_bag.clear();
    }

    pub(super) fn player_sort_title(&mut self) {
        self.player_rewrite_tracks(|tracks, _| playlist::sort_by_title(tracks));
    }

    pub(super) fn player_reverse(&mut self) {
        self.player_rewrite_tracks(|tracks, _| tracks.reverse());
    }

    pub(super) fn player_randomize(&mut self) {
        self.player_rewrite_tracks(|tracks, rng| playlist::fisher_yates(tracks, rng));
    }

    pub(super) fn player_new_list(&mut self) {
        let names: Vec<String> = self
            .preferences
            .playlists
            .iter()
            .map(|playlist| playlist.name.clone())
            .collect();
        let name = if self.player.playlist_draft.trim().is_empty() {
            playlist::next_list_name(&names)
        } else {
            self.player.playlist_draft.trim().to_string()
        };
        if self.preferences.create_playlist(&name) {
            self.player.playlist_draft.clear();
            self.player.selected.clear();
            self.player.shuffle_bag.clear();
        }
    }

    pub(super) fn player_rename_list(&mut self) {
        let name = self.player.playlist_draft.clone();
        if self.preferences.rename_selected_playlist(&name) {
            self.player.playlist_draft.clear();
        }
    }

    pub(super) fn player_delete_list(&mut self) {
        self.preferences.delete_selected_playlist();
        self.player.selected.clear();
        self.player.shuffle_bag.clear();
    }

    pub(super) fn player_switch_list(&mut self, name: String) {
        if self
            .preferences
            .playlists
            .iter()
            .any(|list| list.name == name)
        {
            self.preferences.selected_playlist = name;
            self.player.selected.clear();
            self.player.shuffle_bag.clear();
        }
    }

    pub(super) fn add_custom_station(&mut self, project: &str) {
        match parse_stream_url(&self.player.station_draft) {
            Ok(url) => {
                let name = if self.player.radio_draft_name.trim().is_empty() {
                    url.clone()
                } else {
                    self.player.radio_draft_name.trim().to_string()
                };
                self.preferences.radio_stations.push(RadioStation {
                    name: name.clone(),
                    url: url.clone(),
                    category: "custom".into(),
                    icon: String::new(),
                });
                self.player.station_draft.clear();
                self.player.radio_draft_name.clear();
                self.player.url_prompt = false;
                self.play_radio(
                    project,
                    radio::Station {
                        name,
                        url,
                        country: String::new(),
                        language: String::new(),
                        category: "custom".into(),
                        homepage: String::new(),
                        icon: String::new(),
                    },
                );
            }
            Err(error) => {
                self.player.status = Status::Error(format!("{error:#}"));
            }
        }
    }

    pub(super) fn playlist(&self) -> Vec<PathBuf> {
        self.preferences.selected_tracks().to_vec()
    }

    pub(super) fn player_eq_preset(&mut self, preset: EqPreset) {
        let EqPreset { preamp, bands } = preset;
        self.player.eq_preamp = preamp;
        self.player.eq_bands = bands;
    }
}
