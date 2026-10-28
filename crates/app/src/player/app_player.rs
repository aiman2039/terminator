use super::eq::format_clock;
use super::*;
use crate::player::PollInput;
use engine::Status;
use std::path::PathBuf;
use std::time::Duration;
pub(super) struct PlayerIconButtonInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) name: &'a str,
    pub(super) tip: &'a str,
    pub(super) lit: bool,
}

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
            appearance::icon_color()
        };
        let response = appearance::framed_icon_button(appearance::FramedIconButtonInput {
            ui,
            icon: "AudioLines",
            tip: self.player_chrome_tip(),
            selected: active,
            tint,
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "player-chrome", response.rect);
        if response.clicked() {
            self.open_player_from_chrome();
        }
    }

    pub(crate) fn header_player_menu(&mut self, ui: &mut egui::Ui) -> bool {
        let mark = if self.player_active() { "✓" } else { "" };
        let clicked = appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Player",
            icon: "AudioLines",
            shortcut: mark,
        })
        .clicked();
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
        Self::player_icon_button(PlayerIconButtonInput {
            ui,
            name,
            tip,
            lit: false,
        })
    }

    pub(super) fn player_icon_button(input: PlayerIconButtonInput<'_>) -> egui::Response {
        let PlayerIconButtonInput { ui, name, tip, lit } = input;
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
            appearance::icon_color()
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
}
