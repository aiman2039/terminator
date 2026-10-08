use super::eq::{EqPreset, format_clock};
use super::*;
use crate::preferences::RadioStation;
use engine::Status;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
impl App {
    pub(crate) fn poll_player(&mut self) {
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

    pub(crate) fn open_audio(&mut self, project: &str, path: PathBuf, _split: Option<&str>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let index = self.enqueue_audio(path.clone());
        self.preferences
            .player_index
            .insert(self.preferences.selected_playlist.clone(), index);
        self.player.set_volume(self.preferences.player_volume);
        self.player.play_file(project, path, index);
    }

    pub(crate) fn add_audio_files(&mut self, paths: Vec<PathBuf>) {
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

    pub(super) fn enqueue_audio(&mut self, path: PathBuf) -> usize {
        self.preferences.ensure_default_playlist();
        let Some(tracks) = self.preferences.selected_tracks_mut() else {
            return 0;
        };
        if !tracks.contains(&path) {
            tracks.push(path.clone());
        }
        tracks.iter().position(|item| item == &path).unwrap_or(0)
    }

    pub(crate) fn open_player(&mut self) {
        self.seed_sample_playlist();
        self.dismiss_player_tabs();
        self.player.radio_mode = self.preferences.player_radio_mode;
        self.hide_center_overlay();
        self.player_open = true;
    }

    pub(super) fn seed_sample_playlist(&mut self) {
        if self.preferences.player_samples_seeded {
            return;
        }
        self.preferences.ensure_default_playlist();
        let samples = install_samples(&self.paths.data.join("player-samples"));
        if samples.is_empty() {
            self.error = Some("Could not install sample tracks".into());
            return;
        }
        self.preferences.player_samples_seeded = true;
        self.apply_player_samples(samples);
    }
    pub(crate) fn apply_player_samples(&mut self, samples: Vec<PathBuf>) {
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

    pub(super) fn dismiss_player_tabs(&mut self) {
        for workspace in self.layouts.values_mut() {
            workspace.strip_player();
        }
    }

    pub(crate) fn player_center(&mut self, ui: &mut egui::Ui) {
        ui::center(self, ui);
    }

    pub(crate) fn player_chrome_expanded(&self) -> bool {
        self.player_active() && !self.preferences.player_chrome_collapsed
    }

    pub(super) fn player_chrome_tip(&self) -> &'static str {
        if self.preferences.player_chrome_collapsed && self.player_active() {
            "Show player"
        } else {
            "Player"
        }
    }

    pub(super) fn open_player_from_chrome(&mut self) {
        if self.preferences.player_chrome_collapsed && self.player_active() {
            self.preferences.player_chrome_collapsed = false;
        } else {
            self.open_player();
        }
    }

    pub(crate) fn player_toggle_button(&mut self, ui: &mut egui::Ui) {
        // Only playback needs a steady frame cadence for the now-playing stamp.
        // Requesting this unconditionally pinned the window to 5 fps even with
        // no audio and no player view.
        if matches!(self.player.status, Status::Playing { .. }) {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        let icon = Self::paint_player_icon(
            ui,
            PlayerIconButton {
                name: "AudioLines",
                tip: self.player_chrome_tip(),
                lit: self.player_active(),
                size: 28.0,
                glyph: 16.0,
            },
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "player-chrome", icon.rect);
        if icon.clicked() {
            self.open_player_from_chrome();
        }
    }

    /// Project-header Player control. Matches the hide-sidebar square.
    pub(crate) fn header_player_button(&mut self, ui: &mut egui::Ui) {
        if matches!(self.player.status, Status::Playing { .. }) {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        let active = self.player_active();
        let tint = if active {
            ui.visuals().selection.stroke.color
        } else {
            appearance::ICON_COLOR
        };
        let response = appearance::framed_icon_button(
            ui,
            "AudioLines",
            self.player_chrome_tip(),
            active,
            tint,
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "player-chrome", response.rect);
        if response.clicked() {
            self.open_player_from_chrome();
        }
    }

    pub(crate) fn header_player_menu(&mut self, ui: &mut egui::Ui) -> bool {
        let mark = if self.player_active() { "✓" } else { "" };
        let clicked = appearance::menu_item(ui, "Player", "AudioLines", mark).clicked();
        if clicked {
            self.open_player_from_chrome();
        }
        clicked
    }

    /// Compact IDE status-bar row: icon (opens the Player view),
    /// now-playing stamp, and play/pause + next. Rendered only while audio
    /// is active so a fresh install shows no dead controls.
    pub(crate) fn player_status_row(&mut self, ui: &mut egui::Ui) {
        if !self.player_active() {
            return;
        }
        self.player_toggle_button(ui);
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
        ui.weak(
            egui::RichText::new(format_clock(position))
                .monospace()
                .size(12.0),
        );
        ui.add(egui::Label::new(title).truncate())
            .on_hover_text(format!("{title} / {total}"));
        let playing = self.player.playing();
        let (icon, tip) = if playing {
            ("Pause", "Pause")
        } else {
            ("Play", "Play")
        };
        let toggle = Self::player_tool_button(ui, icon, tip);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-player", toggle.rect);
        if toggle.clicked() {
            self.player_play_pause();
        }
        if Self::player_tool_button(ui, "SkipForward", "Next").clicked() {
            self.player_skip(1);
        }
    }

    pub(crate) fn player_live_controls(&mut self, ui: &mut egui::Ui) {
        if !self.player_chrome_expanded() {
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

    pub(super) fn player_active(&self) -> bool {
        self.player.project.is_some() && !matches!(self.player.status, Status::Stopped)
    }

    pub(super) fn player_mini_now_playing(&self, ui: &mut egui::Ui) {
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

    pub(super) fn player_transport(&mut self, ui: &mut egui::Ui, details: bool) {
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

    pub(super) fn player_tool_button(ui: &mut egui::Ui, name: &str, tip: &str) -> egui::Response {
        Self::player_icon_button(ui, name, tip, false)
    }

    pub(super) fn player_icon_button(
        ui: &mut egui::Ui,
        name: &str,
        tip: &str,
        lit: bool,
    ) -> egui::Response {
        Self::paint_player_icon(
            ui,
            PlayerIconButton {
                name,
                tip,
                lit,
                size: 32.0,
                glyph: 18.0,
            },
        )
    }

    pub(super) fn paint_player_icon(
        ui: &mut egui::Ui,
        button: PlayerIconButton<'_>,
    ) -> egui::Response {
        let PlayerIconButton {
            name,
            tip,
            lit,
            size,
            glyph,
        } = button;
        let response = ui
            .allocate_response(egui::vec2(size, size), egui::Sense::click())
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
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(glyph, glyph)),
        );
        response
    }

    pub(super) fn player_owner(&self) -> Option<String> {
        self.player
            .project
            .clone()
            .or_else(|| self.selected.clone())
    }

    pub(super) fn player_play_pause(&mut self) {
        let Some(project) = self.player_owner() else {
            return;
        };
        if matches!(self.player.status, Status::Stopped | Status::Error(_)) {
            self.player_start(&project);
        } else {
            self.player.toggle();
        }
    }

    pub(super) fn player_start(&mut self, project: &str) {
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

    pub(super) fn player_play_button(&mut self) {
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

    pub(super) fn player_skip(&mut self, delta: isize) {
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

    pub(super) fn radio_listing(&self) -> std::sync::Arc<Vec<radio::Station>> {
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
        std::sync::Arc::clone(&cache.all)
    }
    pub(super) fn radio_visible(&self) -> std::sync::Arc<Vec<radio::Station>> {
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
        std::sync::Arc::clone(&cache.visible)
    }

    pub(super) fn play_radio(&mut self, project: &str, station: radio::Station) {
        let listing = self.radio_listing();
        let index = listing
            .iter()
            .position(|item| item.url == station.url)
            .unwrap_or(0);
        self.player
            .play_station(project, station.name, station.url, index);
    }

    #[cfg(test)]
    pub(crate) fn play_station_at(&mut self, project: &str, index: usize) {
        let stations = self.radio_visible();
        let Some(station) = stations.get(index).cloned() else {
            return;
        };
        self.play_radio(project, station);
    }

    pub(crate) fn play_station_offset(&mut self, project: &str, delta: isize) {
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
        let len = stations.len().cast_signed();
        let Some(sum) = pos.cast_signed().checked_add(delta) else {
            return;
        };
        if len <= 0 {
            return;
        }
        let next = sum.rem_euclid(len).cast_unsigned();
        let Some(station) = stations.get(next).cloned() else {
            return;
        };
        self.play_radio(project, station);
    }

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
