//! GUI-only Winamp-style player. Audio dies with the GUI.
mod engine;
mod playlist;
pub(super) mod radio;
mod tap;
mod ui;

use super::*;
use crate::preferences::RadioStation;
use engine::{Handle, Outcome, Playable, Status};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use radio::parse_stream_url;

pub(super) fn audio_from_dir(root: &Path) -> Vec<PathBuf> {
    playlist::collect_audio(root, playlist::AUDIO_WALK_CAP)
}

const SAMPLE_TRACKS: &[(&str, &[u8])] = &[
    ("pulse.wav", include_bytes!("../../assets/audio/pulse.wav")),
    ("hum.wav", include_bytes!("../../assets/audio/hum.wav")),
    ("chime.wav", include_bytes!("../../assets/audio/chime.wav")),
];

pub(super) fn install_samples(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if std::fs::create_dir_all(dir).is_err() {
        return out;
    }
    for (name, bytes) in SAMPLE_TRACKS {
        let path = dir.join(name);
        if !path.exists() && std::fs::write(&path, bytes).is_err() {
            continue;
        }
        if path.is_file() {
            out.push(path);
        }
    }
    out
}

pub(super) const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "opus", "aac"];

pub fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stack {
    Add,
    Rem,
    Sel,
    Misc,
    List,
    Stations,
}

pub struct Skip<'a> {
    pub project: &'a str,
    pub playlist: &'a [PathBuf],
    pub delta: isize,
    pub shuffle: bool,
    pub repeat: bool,
}

pub struct PollInput<'a> {
    pub project: &'a str,
    pub playlist: &'a [PathBuf],
    pub shuffle: bool,
    pub repeat: bool,
}

#[derive(Default)]
struct RadioCache {
    custom: Vec<RadioStation>,
    base: usize,
    all: std::sync::Arc<Vec<radio::Station>>,
    query: String,
    category: String,
    visible_base: usize,
    visible: std::sync::Arc<Vec<radio::Station>>,
}
pub struct Controller {
    engine: Handle,
    status: Status,
    file_index: Option<usize>,
    pub(super) station_index: Option<usize>,
    pub(super) project: Option<String>,
    station_draft: String,
    playlist_draft: String,
    volume: f32,
    selected: HashSet<usize>,
    last_clicked: Option<usize>,
    stack: Option<Stack>,
    eq_open: bool,
    pl_open: bool,
    eq_on: bool,
    eq_preamp: f32,
    eq_bands: [f32; 10],
    remaining_time: bool,
    url_prompt: bool,
    shuffle_bag: Vec<usize>,
    rng: u64,
    current_path: Option<PathBuf>,
    durations: HashMap<PathBuf, Duration>,
    spectrum: [f32; tap::BARS],
    pub(super) radio_base: std::sync::Arc<Vec<radio::Station>>,
    radio_cache: std::cell::RefCell<RadioCache>,
    radio_mode: bool,
    radio_query: String,
    radio_category: String,
    radio_draft_name: String,
}

impl Controller {
    fn with_engine(
        engine: Handle,
        status: Status,
        file_index: Option<usize>,
        project: Option<String>,
        rng: u64,
    ) -> Self {
        Self {
            engine,
            status,
            file_index,
            station_index: None,
            project,
            station_draft: String::new(),
            playlist_draft: String::new(),
            volume: 0.8,
            selected: HashSet::new(),
            last_clicked: None,
            stack: None,
            eq_open: true,
            pl_open: true,
            eq_on: true,
            eq_preamp: 0.5,
            eq_bands: [0.5; 10],
            remaining_time: false,
            url_prompt: false,
            shuffle_bag: Vec::new(),
            rng,
            current_path: None,
            durations: HashMap::new(),
            spectrum: [0.0; tap::BARS],
            radio_base: if cfg!(test) {
                std::sync::Arc::new(radio::catalog().to_vec())
            } else {
                Default::default()
            },
            radio_cache: Default::default(),
            radio_mode: false,
            radio_query: String::new(),
            radio_category: String::new(),
            radio_draft_name: String::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn finished_fixture(project: &str, file_index: Option<usize>) -> Self {
        Self::with_engine(
            Handle::finished_fixture(),
            Status::Stopped,
            file_index,
            Some(project.into()),
            0x9E37_79B9_7F4A_7C15,
        )
    }

    pub fn new(services: crate::gui_services::Services) -> Self {
        let rng = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(1)
            .max(1);
        Self::with_engine(Handle::spawn(services), Status::Stopped, None, None, rng)
    }

    #[cfg(feature = "test-support")]
    pub fn fixture_play(&mut self, project: &str, url: String) {
        self.volume = 0.0;
        self.engine.volume(0.0);
        self.play_station(project, "Fixture radio".into(), url, 0);
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_diagnostics(&self) -> serde_json::Value {
        self.engine.diagnostics()
    }
    pub fn poll(&mut self, input: PollInput<'_>) -> Option<usize> {
        let PollInput {
            project,
            playlist,
            shuffle,
            repeat,
        } = input;
        let mut finished = false;
        let mut status = None;
        for outcome in self.engine.poll() {
            match outcome {
                Outcome::Status(next) => status = Some(next),
                Outcome::Finished => finished = true,
            }
        }
        if let Some(next) = status
            && !(finished && matches!(next, Status::Stopped))
        {
            self.status = next;
        }
        self.cache_duration();
        self.refresh_spectrum();
        if finished && self.project.as_deref() == Some(project) && self.file_index.is_some() {
            return self.play_offset(Skip {
                project,
                playlist,
                delta: 1,
                shuffle,
                repeat,
            });
        }
        None
    }

    fn refresh_spectrum(&mut self) {
        if matches!(self.status, Status::Playing { .. })
            && let Some(next) = self.engine.spectrum_bars()
        {
            for (bar, target) in self.spectrum.iter_mut().zip(next) {
                *bar = if target > *bar {
                    target
                } else {
                    *bar * 0.78 + target * 0.22
                };
            }
            return;
        }
        for bar in &mut self.spectrum {
            *bar *= 0.86;
        }
    }

    fn cache_duration(&mut self) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        let duration = match &self.status {
            Status::Playing {
                duration: Some(duration),
                ..
            }
            | Status::Paused {
                duration: Some(duration),
                ..
            } => *duration,
            _ => return,
        };
        self.durations.insert(path, duration);
    }

    pub fn stop(&mut self) {
        self.engine.stop();
        self.file_index = None;
        self.station_index = None;
        self.project = None;
        self.current_path = None;
        self.status = Status::Stopped;
    }

    pub fn pause(&mut self) {
        match self.status.clone() {
            Status::Playing {
                title,
                position,
                duration,
                seekable,
            } => {
                self.engine.pause();
                self.status = Status::Paused {
                    title,
                    position,
                    duration,
                    seekable,
                };
            }
            Status::Paused { .. } => self.resume(),
            Status::Loading { title }
            | Status::Buffering { title }
            | Status::Reconnecting { title, .. } => {
                self.engine.pause();
                self.status = Status::Paused {
                    title,
                    position: Duration::ZERO,
                    duration: None,
                    seekable: self.file_index.is_some(),
                };
            }
            Status::Stopped | Status::Error(_) => {}
        }
    }
    pub fn toggle(&mut self) {
        self.pause();
    }
    pub fn resume(&mut self) {
        if let Status::Paused {
            title,
            position,
            duration,
            seekable,
        } = self.status.clone()
        {
            self.engine.resume();
            self.status = if seekable {
                Status::Playing {
                    title,
                    position,
                    duration,
                    seekable,
                }
            } else {
                Status::Loading { title }
            };
        }
    }
    pub fn needs_poll(&self) -> bool {
        matches!(
            self.status,
            Status::Loading { .. }
                | Status::Buffering { .. }
                | Status::Reconnecting { .. }
                | Status::Playing { .. }
        )
    }

    pub fn play_file(&mut self, project: &str, path: PathBuf, index: usize) {
        let title = playlist::file_title(&path);
        self.project = Some(project.into());
        self.file_index = Some(index);
        self.station_index = None;
        self.current_path = Some(path.clone());
        self.shuffle_bag.retain(|item| *item != index);
        self.engine.volume(self.volume);
        self.status = Status::Loading {
            title: title.clone(),
        };
        self.engine.play(Playable::File { path, title });
    }

    pub fn play_station(&mut self, project: &str, name: String, url: String, index: usize) {
        self.project = Some(project.into());
        self.file_index = None;
        self.station_index = Some(index);
        self.current_path = None;
        self.engine.volume(self.volume);
        self.status = Status::Loading {
            title: name.clone(),
        };
        self.engine.play(Playable::Stream { url, title: name });
    }

    pub fn radio(&self) -> bool {
        self.project.is_some() && self.file_index.is_none()
    }

    pub fn play_offset(&mut self, skip: Skip<'_>) -> Option<usize> {
        let Skip {
            project,
            playlist,
            delta,
            shuffle,
            repeat,
        } = skip;
        if playlist.is_empty() {
            self.stop();
            return None;
        }
        if shuffle {
            return self.play_shuffled(project, playlist, repeat);
        }
        self.play_sequential(project, playlist, delta, repeat)
    }

    fn play_sequential(
        &mut self,
        project: &str,
        playlist: &[PathBuf],
        delta: isize,
        repeat: bool,
    ) -> Option<usize> {
        let current = if self.project.as_deref() == Some(project) {
            self.file_index.unwrap_or(0)
        } else {
            0
        };
        let last = playlist.len() - 1;
        let next = current as isize + delta;
        let index = if next < 0 {
            if repeat { last } else { 0 }
        } else if next as usize > last {
            if repeat {
                0
            } else {
                self.stop();
                return None;
            }
        } else {
            next as usize
        };
        self.play_file(project, playlist[index].clone(), index);
        Some(index)
    }

    fn play_shuffled(
        &mut self,
        project: &str,
        playlist: &[PathBuf],
        repeat: bool,
    ) -> Option<usize> {
        let current = self.file_index.unwrap_or(0);
        if self.shuffle_bag.is_empty() {
            self.refill_bag(playlist.len(), current);
        }
        let Some(index) = self.shuffle_bag.pop() else {
            if repeat {
                self.play_file(project, playlist[current].clone(), current);
                return Some(current);
            }
            self.stop();
            return None;
        };
        let index = index.min(playlist.len() - 1);
        self.play_file(project, playlist[index].clone(), index);
        Some(index)
    }

    fn refill_bag(&mut self, len: usize, current: usize) {
        self.shuffle_bag = (0..len).filter(|index| *index != current).collect();
        playlist::fisher_yates(&mut self.shuffle_bag, &mut self.rng);
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

    pub(super) fn playing(&self) -> bool {
        matches!(self.status, Status::Playing { .. })
    }

    fn click_track(&mut self, index: usize, shift: bool, ctrl: bool) {
        if shift && let Some(anchor) = self.last_clicked {
            self.selected = playlist::range_selection(anchor, index);
            return;
        }
        if ctrl {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.last_clicked = Some(index);
            return;
        }
        self.selected.clear();
        self.selected.insert(index);
        self.last_clicked = Some(index);
    }
}

impl App {
    pub(super) fn poll_player(&mut self) {
        let project = self.player.project.clone().unwrap_or_default();
        let playlist = self.playlist();
        if let Some(index) = self.player.poll(PollInput {
            project: &project,
            playlist: &playlist,
            shuffle: self.preferences.player_shuffle,
            repeat: self.preferences.player_repeat,
        }) {
            self.preferences
                .player_index
                .insert(self.preferences.selected_playlist.clone(), index);
        }
    }

    pub(super) fn open_audio(&mut self, project: &str, path: PathBuf, _split: Option<&str>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let index = self.enqueue_audio(path.clone());
        self.preferences
            .player_index
            .insert(self.preferences.selected_playlist.clone(), index);
        self.player.set_volume(self.preferences.player_volume);
        self.player.play_file(project, path, index);
    }

    pub(super) fn add_audio_files(&mut self, paths: Vec<PathBuf>) {
        let mut first = None;
        for path in paths {
            if !supported(&path) {
                continue;
            }
            let path = std::path::absolute(&path).unwrap_or(path);
            let index = self.enqueue_audio(path.clone());
            if first.is_none() {
                first = Some((path, index));
            }
        }
        let Some((path, index)) = first else {
            return;
        };
        if self.player_active() {
            return;
        }
        let Some(project) = self.player_owner() else {
            return;
        };
        self.preferences
            .player_index
            .insert(self.preferences.selected_playlist.clone(), index);
        self.player.set_volume(self.preferences.player_volume);
        self.player.play_file(&project, path, index);
    }

    fn enqueue_audio(&mut self, path: PathBuf) -> usize {
        self.preferences.ensure_default_playlist();
        let Some(tracks) = self.preferences.selected_tracks_mut() else {
            return 0;
        };
        if !tracks.contains(&path) {
            tracks.push(path.clone());
        }
        tracks.iter().position(|item| item == &path).unwrap_or(0)
    }

    pub(super) fn open_player(&mut self) {
        self.seed_sample_playlist();
        self.dismiss_player_tabs();
        self.player.radio_mode = self.preferences.player_radio_mode;
        self.settings_open = false;
        self.player_open = true;
    }

    fn seed_sample_playlist(&mut self) {
        if self.preferences.player_samples_seeded {
            return;
        }
        self.preferences.ensure_default_playlist();
        self.preferences.player_samples_seeded = true;
        let data = self.paths.data.join("player-samples");
        let service = self.services.clone();
        let context = terminator_core::async_service::OperationContext::new(
            "catalog",
            "player-samples".into(),
            terminator_core::async_service::Policy::OrderedMutation,
        );
        if self
            .services
            .handle()
            .submit(
                context,
                terminator_core::async_service::CancellationToken::new(),
                async move {
                    let samples = service
                        .client()
                        .catalog
                        .run(
                            &terminator_core::async_service::CancellationToken::new(),
                            move || Ok(install_samples(&data)),
                        )
                        .await?;
                    Ok(vec![Update::PlayerSamples(samples)])
                },
            )
            .is_err()
        {
            self.preferences.player_samples_seeded = false;
            self.error = Some("Services are busy; reopen Player to install sample tracks".into());
        }
    }
    pub(super) fn apply_player_samples(&mut self, samples: Vec<PathBuf>) {
        if samples.is_empty() {
            return;
        }
        let Some(tracks) = self.preferences.selected_tracks_mut() else {
            return;
        };
        if !tracks.is_empty() {
            return;
        }
        tracks.extend(samples);
    }

    fn dismiss_player_tabs(&mut self) {
        for workspace in self.layouts.values_mut() {
            workspace.strip_player();
        }
    }

    pub(super) fn player_center(&mut self, ui: &mut egui::Ui) {
        ui::center(self, ui);
    }

    pub(super) fn player_toggle_button(&mut self, ui: &mut egui::Ui) {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
        let icon = Self::player_icon_button(
            ui,
            "AudioLines",
            if self.preferences.player_chrome_collapsed && self.player_active() {
                "Show player"
            } else {
                "Player"
            },
            self.player_active(),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "player-chrome", icon.rect);
        if icon.clicked() {
            if self.preferences.player_chrome_collapsed && self.player_active() {
                self.preferences.player_chrome_collapsed = false;
            } else {
                self.open_player();
            }
        }
    }

    pub(super) fn player_live_controls(&mut self, ui: &mut egui::Ui) {
        if !self.player_active() || self.preferences.player_chrome_collapsed {
            return;
        }
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());
            self.player_mini_now_playing(ui);
            ui.horizontal(|ui| {
                self.player_transport(ui, false);
            });
        });
    }

    fn player_active(&self) -> bool {
        self.player.project.is_some() && !matches!(self.player.status, Status::Stopped)
    }

    fn player_mini_now_playing(&self, ui: &mut egui::Ui) {
        let (title, position, duration) = match &self.player.status {
            Status::Playing {
                title,
                position,
                duration,
                ..
            }
            | Status::Paused {
                title,
                position,
                duration,
                ..
            } => (title.as_str(), *position, *duration),
            Status::Loading { title }
            | Status::Buffering { title }
            | Status::Reconnecting { title, .. } => (title.as_str(), Duration::ZERO, None),
            Status::Error(error) => (error.as_str(), Duration::ZERO, None),
            Status::Stopped => ("Player", Duration::ZERO, None),
        };
        let total = duration.map(format_clock).unwrap_or_else(|| "--:--".into());
        let clock = format!("{:>5} / {total:<5}", format_clock(position));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 18.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.weak(egui::RichText::new(clock).monospace().size(12.0));
                ui.add(egui::Label::new(title).truncate())
                    .on_hover_text(title);
            },
        );
    }

    fn player_transport(&mut self, ui: &mut egui::Ui, details: bool) {
        if Self::player_tool_button(ui, "SkipBack", "Previous").clicked() {
            self.player_skip(-1);
        }
        let playing = self.player.playing();
        let (icon, tip) = if playing {
            ("Pause", "Pause")
        } else {
            ("Play", "Play")
        };
        if Self::player_tool_button(ui, icon, tip).clicked() {
            self.player_play_pause();
        }
        if details
            && Self::player_tool_button(ui, "Square", "Stop").clicked()
            && self.player.project.is_some()
        {
            self.player.stop();
        }
        if Self::player_tool_button(ui, "SkipForward", "Next").clicked() {
            self.player_skip(1);
        }
        if !details && Self::player_tool_button(ui, "ListMusic", "Player").clicked() {
            self.open_player();
        }
        if !details && Self::player_tool_button(ui, "PanelTopClose", "Hide player").clicked() {
            self.preferences.player_chrome_collapsed = true;
        }
    }

    fn player_tool_button(ui: &mut egui::Ui, name: &str, tip: &str) -> egui::Response {
        Self::player_icon_button(ui, name, tip, false)
    }

    fn player_icon_button(ui: &mut egui::Ui, name: &str, tip: &str, lit: bool) -> egui::Response {
        let response = ui
            .allocate_response(egui::vec2(32.0, 32.0), egui::Sense::click())
            .on_hover_text(tip);
        if lit {
            ui.painter()
                .rect_filled(response.rect, 6.0, ui.visuals().selection.bg_fill);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(response.rect, 6.0, ui.visuals().widgets.hovered.bg_fill);
        }
        let tint = if lit {
            ui.visuals().selection.stroke.color
        } else {
            appearance::ICON_COLOR
        };
        egui::Image::new(icons::source(name)).tint(tint).paint_at(
            ui,
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(18.0, 18.0)),
        );
        response
    }

    fn player_owner(&self) -> Option<String> {
        self.player
            .project
            .clone()
            .or_else(|| self.selected.clone())
    }

    fn player_play_pause(&mut self) {
        let Some(project) = self.player_owner() else {
            return;
        };
        if matches!(self.player.status, Status::Stopped | Status::Error(_)) {
            self.player_start(&project);
        } else {
            self.player.toggle();
        }
    }

    fn player_start(&mut self, project: &str) {
        if self.player.radio_mode
            && let Some(station) = self.radio_visible().first().cloned()
        {
            self.play_radio(project, station);
            return;
        }
        let playlist = self.playlist();
        if let Some(path) = playlist.first() {
            self.player.play_file(project, path.clone(), 0);
        } else if let Some(station) = self.radio_visible().first().cloned() {
            self.play_radio(project, station);
        } else {
            self.pick_audio = true;
        }
    }

    fn player_play_button(&mut self) {
        let Some(project) = self.player_owner() else {
            return;
        };
        match &self.player.status {
            Status::Paused { .. } => self.player.resume(),
            Status::Playing {
                duration: Some(duration),
                seekable: true,
                ..
            } => {
                let duration = *duration;
                self.player.seek_fraction(0.0, duration);
            }
            Status::Playing { .. }
            | Status::Loading { .. }
            | Status::Buffering { .. }
            | Status::Reconnecting { .. } => {}
            Status::Stopped | Status::Error(_) => self.player_start(&project),
        }
    }

    fn player_skip(&mut self, delta: isize) {
        let Some(project) = self.player_owner() else {
            return;
        };
        if self.player.radio() {
            self.play_station_offset(&project, delta);
            return;
        }
        let playlist = self.playlist();
        if let Some(index) = self.player.play_offset(Skip {
            project: &project,
            playlist: &playlist,
            delta,
            shuffle: self.preferences.player_shuffle,
            repeat: self.preferences.player_repeat,
        }) {
            self.preferences
                .player_index
                .insert(self.preferences.selected_playlist.clone(), index);
        }
    }

    fn radio_listing(&self) -> std::sync::Arc<Vec<radio::Station>> {
        let base = std::sync::Arc::as_ptr(&self.player.radio_base) as usize;
        let mut cache = self.player.radio_cache.borrow_mut();
        if cache.base != base || cache.custom != self.preferences.radio_stations {
            let mut all = self.player.radio_base.as_ref().clone();
            for custom in &self.preferences.radio_stations {
                if all.iter().any(|station| station.url == custom.url) {
                    continue;
                }
                all.push(radio::Station {
                    name: custom.name.clone(),
                    url: custom.url.clone(),
                    country: String::new(),
                    language: String::new(),
                    category: if custom.category.is_empty() {
                        "custom".into()
                    } else {
                        custom.category.clone()
                    },
                    homepage: String::new(),
                    icon: custom.icon.clone(),
                });
            }
            cache.all = std::sync::Arc::new(all);
            cache.custom.clone_from(&self.preferences.radio_stations);
            cache.base = base;
        }
        cache.all.clone()
    }
    fn radio_visible(&self) -> std::sync::Arc<Vec<radio::Station>> {
        let all = self.radio_listing();
        let base = std::sync::Arc::as_ptr(&all) as usize;
        let mut cache = self.player.radio_cache.borrow_mut();
        if cache.visible_base != base
            || cache.query != self.player.radio_query
            || cache.category != self.player.radio_category
        {
            cache.visible = std::sync::Arc::new(
                all.iter()
                    .filter(|station| {
                        radio::matches_filter(
                            station,
                            &self.player.radio_query,
                            &self.player.radio_category,
                        )
                    })
                    .cloned()
                    .collect(),
            );
            cache.visible_base = base;
            cache.query.clone_from(&self.player.radio_query);
            cache.category.clone_from(&self.player.radio_category);
        }
        cache.visible.clone()
    }

    fn play_radio(&mut self, project: &str, station: radio::Station) {
        let listing = self.radio_listing();
        let index = listing
            .iter()
            .position(|item| item.url == station.url)
            .unwrap_or(0);
        self.player
            .play_station(project, station.name, station.url, index);
    }

    #[cfg(test)]
    pub(super) fn play_station_at(&mut self, project: &str, index: usize) {
        let stations = self.radio_visible();
        let Some(station) = stations.get(index).cloned() else {
            return;
        };
        self.play_radio(project, station);
    }

    pub(super) fn play_station_offset(&mut self, project: &str, delta: isize) {
        let stations = self.radio_visible();
        if stations.is_empty() {
            return;
        }
        let listing = self.radio_listing();
        let current = self
            .player
            .station_index
            .and_then(|index| listing.get(index).map(|station| station.url.as_str()));
        let pos = current
            .and_then(|url| stations.iter().position(|station| station.url == url))
            .unwrap_or(0);
        let len = stations.len() as isize;
        let next = (pos as isize + delta).rem_euclid(len) as usize;
        let Some(station) = stations.get(next).cloned() else {
            return;
        };
        self.play_radio(project, station);
    }

    fn player_play_index(&mut self, project: &str, index: usize) {
        let playlist = self.playlist();
        let Some(path) = playlist.get(index).cloned() else {
            return;
        };
        self.player.play_file(project, path, index);
        self.preferences
            .player_index
            .insert(self.preferences.selected_playlist.clone(), index);
    }

    fn player_apply_remove(&mut self, drop: HashSet<usize>) {
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

    fn player_rem_selected(&mut self) {
        let drop = self.player.selected.clone();
        self.player_apply_remove(drop);
    }

    fn player_crop(&mut self) {
        if self.player.selected.is_empty() {
            return;
        }
        let len = self.playlist().len();
        let drop = playlist::crop_drop(len, &self.player.selected);
        self.player_apply_remove(drop);
    }

    fn player_clear_tracks(&mut self) {
        let len = self.playlist().len();
        self.player_apply_remove((0..len).collect());
    }

    fn player_sel_all(&mut self) {
        let len = self.playlist().len();
        self.player.selected = (0..len).collect();
    }

    fn player_sel_none(&mut self) {
        self.player.selected.clear();
    }

    fn player_sel_invert(&mut self) {
        let len = self.playlist().len();
        self.player.selected = playlist::invert_selection(len, &self.player.selected);
    }

    fn player_rewrite_tracks(&mut self, mut rewrite: impl FnMut(&mut Vec<PathBuf>, &mut u64)) {
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

    fn player_sort_title(&mut self) {
        self.player_rewrite_tracks(|tracks, _| playlist::sort_by_title(tracks));
    }

    fn player_reverse(&mut self) {
        self.player_rewrite_tracks(|tracks, _| tracks.reverse());
    }

    fn player_randomize(&mut self) {
        self.player_rewrite_tracks(|tracks, rng| playlist::fisher_yates(tracks, rng));
    }

    fn player_new_list(&mut self) {
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

    fn player_rename_list(&mut self) {
        let name = self.player.playlist_draft.clone();
        if self.preferences.rename_selected_playlist(&name) {
            self.player.playlist_draft.clear();
        }
    }

    fn player_delete_list(&mut self) {
        self.preferences.delete_selected_playlist();
        self.player.selected.clear();
        self.player.shuffle_bag.clear();
    }

    fn player_switch_list(&mut self, name: String) {
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

    fn add_custom_station(&mut self, project: &str) {
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

    fn playlist(&self) -> Vec<PathBuf> {
        self.preferences.selected_tracks().to_vec()
    }

    fn player_eq_preset(&mut self, preset: EqPreset) {
        let EqPreset { preamp, bands } = preset;
        self.player.eq_preamp = preamp;
        self.player.eq_bands = bands;
    }
}

#[derive(Clone, Copy)]
struct EqPreset {
    preamp: f32,
    bands: [f32; 10],
}

fn eq_flat() -> EqPreset {
    EqPreset {
        preamp: 0.5,
        bands: [0.5; 10],
    }
}

fn eq_bass() -> EqPreset {
    EqPreset {
        preamp: 0.55,
        bands: [0.82, 0.76, 0.68, 0.58, 0.52, 0.5, 0.48, 0.5, 0.5, 0.5],
    }
}

fn eq_treble() -> EqPreset {
    EqPreset {
        preamp: 0.5,
        bands: [0.42, 0.45, 0.48, 0.5, 0.52, 0.58, 0.66, 0.74, 0.8, 0.84],
    }
}

fn format_clock(duration: Duration) -> String {
    let total = duration.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
pub(crate) async fn fixture_download(url: String) -> Result<()> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    let (sender, _receiver) = tokio::sync::mpsc::channel(14);
    engine::download(
        &client,
        url,
        sender,
        Duration::from_secs(8),
        Duration::from_secs(10),
    )
    .await
}

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
