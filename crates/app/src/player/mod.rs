//! GUI-only Winamp-style player. Audio dies with the GUI.
mod engine;
mod radio;

use super::*;
use crate::preferences::RadioStation;
use engine::{Handle, Outcome, Playable, Status};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use radio::parse_stream_url;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "opus", "aac"];

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

pub struct Controller {
    engine: Handle,
    status: Status,
    file_index: Option<usize>,
    pub(super) project: Option<String>,
    station_draft: String,
    volume: f32,
}

impl Controller {
    #[cfg(test)]
    pub(super) fn finished_fixture(project: &str, file_index: Option<usize>) -> Self {
        Self {
            engine: Handle::finished_fixture(),
            status: Status::Stopped,
            file_index,
            project: Some(project.into()),
            station_draft: String::new(),
            volume: 0.8,
        }
    }

    pub fn new() -> Self {
        Self {
            engine: Handle::spawn(),
            status: Status::Stopped,
            file_index: None,
            project: None,
            station_draft: String::new(),
            volume: 0.8,
        }
    }

    pub fn poll(&mut self, project: &str, playlist: &[PathBuf]) -> Option<usize> {
        let mut finished = false;
        for outcome in self.engine.poll() {
            match outcome {
                Outcome::Status(status) => self.status = status,
                Outcome::Finished => finished = true,
            }
        }
        if finished && self.project.as_deref() == Some(project) && self.file_index.is_some() {
            return self.play_offset(project, playlist, 1);
        }
        None
    }

    pub fn stop(&mut self) {
        self.engine.stop();
        self.file_index = None;
        self.project = None;
        self.status = Status::Stopped;
    }

    pub fn toggle(&mut self) {
        match self.status {
            Status::Playing { .. } => self.engine.pause(),
            Status::Paused { .. } => self.engine.resume(),
            Status::Stopped | Status::Error(_) => {}
        }
    }

    pub fn play_file(&mut self, project: &str, path: PathBuf, index: usize) {
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.project = Some(project.into());
        self.file_index = Some(index);
        self.engine.volume(self.volume);
        self.engine.play(Playable::File { path, title });
    }

    pub fn play_station(&mut self, project: &str, name: String, url: String) {
        self.project = Some(project.into());
        self.file_index = None;
        self.engine.volume(self.volume);
        self.engine.play(Playable::Stream { url, title: name });
    }

    pub fn play_offset(
        &mut self,
        project: &str,
        playlist: &[PathBuf],
        delta: isize,
    ) -> Option<usize> {
        if playlist.is_empty() {
            self.stop();
            return None;
        }
        let current = if self.project.as_deref() == Some(project) {
            self.file_index.unwrap_or(0)
        } else {
            0
        };
        let next = current.saturating_add_signed(delta).min(playlist.len() - 1);
        if delta > 0 && next == current && current + 1 >= playlist.len() {
            self.stop();
            return None;
        }
        self.play_file(project, playlist[next].clone(), next);
        Some(next)
    }

    pub fn seek_fraction(&mut self, fraction: f32, duration: Duration) {
        let millis = duration.as_millis() as f32 * fraction.clamp(0.0, 1.0);
        self.engine.seek(Duration::from_millis(millis as u64));
    }

    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        if (self.volume - volume).abs() <= 0.001 {
            return;
        }
        self.volume = volume;
        self.engine.volume(self.volume);
    }

    fn playing(&self) -> bool {
        matches!(self.status, Status::Playing { .. })
    }
}

impl App {
    pub(super) fn poll_player(&mut self) {
        let project = self.player.project.clone().unwrap_or_default();
        let playlist = self.playlist(&project);
        if let Some(index) = self.player.poll(&project, &playlist) {
            self.preferences.player_index.insert(project, index);
        }
    }

    pub(super) fn open_audio(&mut self, project: &str, path: PathBuf, split: Option<&str>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        {
            let playlist = self
                .preferences
                .player_playlists
                .entry(project.into())
                .or_default();
            if !playlist.contains(&path) {
                playlist.push(path.clone());
            }
        }
        let index = self
            .preferences
            .player_playlists
            .get(project)
            .and_then(|playlist| playlist.iter().position(|item| item == &path))
            .unwrap_or(0);
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), split);
        let _ = self.update_tx.send(Update::OpenPlayer(
            project.into(),
            after,
            Some((path, index)),
        ));
    }

    pub(super) fn open_player_tab(&mut self, project: &str) {
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), None);
        let _ = self
            .update_tx
            .send(Update::OpenPlayer(project.into(), after, None));
    }

    pub(super) fn player_view(&mut self, ui: &mut egui::Ui) {
        let Some(project) = self.selected.clone() else {
            ui.weak("Select a project to use the player.");
            return;
        };
        self.player.set_volume(self.preferences.player_volume);
        ui.ctx().request_repaint_after(Duration::from_millis(200));
        self.player_chrome(ui, &project);
        ui.separator();
        self.player_playlist(ui, &project);
        ui.separator();
        self.player_radio(ui, &project);
    }

    fn player_chrome(&mut self, ui: &mut egui::Ui, project: &str) {
        let owns_player = self.player.project.as_deref() == Some(project);
        let status = if owns_player {
            self.player.status.clone()
        } else {
            Status::Stopped
        };
        match &status {
            Status::Playing { title, .. } | Status::Paused { title, .. } => {
                ui.strong(title);
            }
            Status::Stopped => {
                ui.weak("Stopped");
            }
            Status::Error(error) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
        }
        if let Status::Playing {
            position,
            duration,
            seekable,
            ..
        }
        | Status::Paused {
            position,
            duration,
            seekable,
            ..
        } = &status
        {
            self.player_seek(ui, *position, *duration, *seekable);
        }
        ui.horizontal(|ui| {
            if ui.button("|<").clicked() {
                let playlist = self.playlist(project);
                self.player.play_offset(project, &playlist, -1);
            }
            let play_label = if owns_player && self.player.playing() {
                "Pause"
            } else {
                "Play"
            };
            if ui.button(play_label).clicked() {
                if matches!(status, Status::Stopped | Status::Error(_)) {
                    let playlist = self.playlist(project);
                    if let Some(path) = playlist.first() {
                        self.player.play_file(project, path.clone(), 0);
                    }
                } else {
                    self.player.toggle();
                }
            }
            if ui.button("Stop").clicked() && owns_player {
                self.player.stop();
            }
            if ui.button(">|").clicked() {
                let playlist = self.playlist(project);
                self.player.play_offset(project, &playlist, 1);
            }
            ui.label("Vol");
            let mut volume = self.preferences.player_volume;
            if ui
                .add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false))
                .changed()
            {
                self.preferences.player_volume = volume;
                self.player.set_volume(volume);
            }
        });
        let typing = ui.memory(|memory| memory.has_focus(egui::Id::new("player-station-url")));
        if owns_player && !typing && ui.input(|i| i.key_pressed(egui::Key::Space)) {
            ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
            self.player.toggle();
        }
    }

    fn player_seek(
        &mut self,
        ui: &mut egui::Ui,
        position: Duration,
        duration: Option<Duration>,
        seekable: bool,
    ) {
        ui.horizontal(|ui| {
            ui.monospace(format_clock(position));
            if let Some(total) = duration {
                let mut fraction = if total.is_zero() {
                    0.0
                } else {
                    (position.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
                };
                ui.add_enabled_ui(seekable, |ui| {
                    if ui
                        .add(egui::Slider::new(&mut fraction, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        self.player.seek_fraction(fraction, total);
                    }
                });
                ui.monospace(format_clock(total));
            } else {
                let mut live = 0.0;
                ui.add_enabled_ui(false, |ui| {
                    ui.add(egui::Slider::new(&mut live, 0.0..=1.0).show_value(false));
                });
                ui.weak("live");
            }
        });
    }

    fn player_playlist(&mut self, ui: &mut egui::Ui, project: &str) {
        ui.strong("Playlist");
        let playlist = self.playlist(project);
        if playlist.is_empty() {
            ui.weak("Open an mp3, flac, ogg, wav, m4a, opus, or aac file.");
            return;
        }
        let mut remove = None;
        let mut play = None;
        egui::ScrollArea::vertical()
            .id_salt("player-playlist")
            .max_height(180.0)
            .show(ui, |ui| {
                for (index, path) in playlist.iter().enumerate() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    ui.horizontal(|ui| {
                        let selected = self.player.project.as_deref() == Some(project)
                            && self.player.file_index == Some(index);
                        if ui.selectable_label(selected, name).double_clicked() {
                            play = Some(index);
                        }
                        if ui.small_button("×").clicked() {
                            remove = Some(index);
                        }
                    });
                }
            });
        if let Some(index) = play
            && let Some(path) = playlist.get(index)
        {
            self.player.play_file(project, path.clone(), index);
            self.preferences.player_index.insert(project.into(), index);
        }
        if let Some(index) = remove {
            if let Some(list) = self.preferences.player_playlists.get_mut(project)
                && index < list.len()
            {
                list.remove(index);
            }
            if self.player.project.as_deref() == Some(project)
                && self.player.file_index == Some(index)
            {
                self.player.stop();
            } else if self.player.project.as_deref() == Some(project)
                && let Some(current) = self.player.file_index.as_mut()
                && index < *current
            {
                *current -= 1;
            }
        }
    }

    fn player_radio(&mut self, ui: &mut egui::Ui, project: &str) {
        ui.strong("Radio");
        for station in radio::bundled() {
            if ui.button(station.name).clicked() {
                self.player
                    .play_station(project, station.name.into(), station.url.into());
            }
        }
        ui.separator();
        ui.label("Custom stations");
        let mut play = None;
        let mut remove = None;
        for (index, station) in self.preferences.radio_stations.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.button(&station.name).clicked() {
                    play = Some(index);
                }
                if ui.small_button("×").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = play
            && let Some(station) = self.preferences.radio_stations.get(index)
        {
            self.player
                .play_station(project, station.name.clone(), station.url.clone());
        }
        if let Some(index) = remove
            && index < self.preferences.radio_stations.len()
        {
            self.preferences.radio_stations.remove(index);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.player.station_draft)
                    .hint_text("https://host/stream")
                    .desired_width(240.0)
                    .id(egui::Id::new("player-station-url")),
            );
            if ui.button("Add").clicked() {
                self.add_custom_station(project);
            }
        });
    }

    fn add_custom_station(&mut self, project: &str) {
        match parse_stream_url(&self.player.station_draft) {
            Ok(url) => {
                let name = url.clone();
                self.preferences.radio_stations.push(RadioStation {
                    name,
                    url: url.clone(),
                });
                self.player.station_draft.clear();
                self.player.play_station(
                    project,
                    self.preferences
                        .radio_stations
                        .last()
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| url.clone()),
                    url,
                );
            }
            Err(error) => {
                self.player.status = Status::Error(format!("{error:#}"));
            }
        }
    }

    fn playlist(&self, project: &str) -> Vec<PathBuf> {
        self.preferences
            .player_playlists
            .get(project)
            .cloned()
            .unwrap_or_default()
    }
}

fn format_clock(duration: Duration) -> String {
    let total = duration.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_extensions_are_supported() {
        assert!(supported(Path::new("song.MP3")));
        assert!(supported(Path::new("/tmp/a.flac")));
        assert!(!supported(Path::new("song.mp4")));
        assert!(!supported(Path::new("readme.md")));
    }
}
