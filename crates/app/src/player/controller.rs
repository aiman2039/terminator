use super::*;
use crate::preferences::RadioStation;
use engine::{Handle, Outcome, Playable, Status};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;
pub(crate) struct Skip<'a> {
    pub project: &'a str,
    pub playlist: &'a [PathBuf],
    pub delta: isize,
    pub shuffle: bool,
    pub repeat: bool,
}

pub(crate) struct PollInput<'a> {
    pub project: &'a str,
    pub playlist: &'a [PathBuf],
    pub shuffle: bool,
    pub repeat: bool,
}

#[derive(Default)]
pub(crate) struct RadioCache {
    pub(crate) custom: Vec<RadioStation>,
    pub(crate) base: usize,
    pub(crate) all: std::sync::Arc<Vec<radio::Station>>,
    pub(crate) query: String,
    pub(crate) category: String,
    pub(crate) visible_base: usize,
    pub(crate) visible: std::sync::Arc<Vec<radio::Station>>,
}
pub(crate) struct Controller {
    pub(crate) engine: Handle,
    pub(crate) status: Status,
    pub(crate) file_index: Option<usize>,
    pub(crate) station_index: Option<usize>,
    pub(crate) project: Option<String>,
    pub(crate) station_draft: String,
    pub(crate) playlist_draft: String,
    pub(crate) volume: f32,
    pub(crate) selected: HashSet<usize>,
    pub(crate) last_clicked: Option<usize>,
    pub(crate) stack: Option<Stack>,
    pub(crate) eq_open: bool,
    pub(crate) pl_open: bool,
    pub(crate) eq_on: bool,
    pub(crate) eq_preamp: f32,
    pub(crate) eq_bands: [f32; 10],
    pub(crate) remaining_time: bool,
    pub(crate) url_prompt: bool,
    pub(crate) shuffle_bag: Vec<usize>,
    pub(crate) rng: u64,
    pub(crate) current_path: Option<PathBuf>,
    pub(crate) durations: HashMap<PathBuf, Duration>,
    pub(crate) spectrum: [f32; tap::BARS],
    pub(crate) radio_base: std::sync::Arc<Vec<radio::Station>>,
    pub(crate) radio_cache: std::cell::RefCell<RadioCache>,
    pub(crate) radio_mode: bool,
    pub(crate) radio_query: String,
    pub(crate) radio_category: String,
    pub(crate) radio_draft_name: String,
}

impl Controller {
    pub(super) fn with_engine(
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
                std::sync::Arc::default()
            },
            radio_cache: std::cell::RefCell::default(),
            radio_mode: false,
            radio_query: String::new(),
            radio_category: String::new(),
            radio_draft_name: String::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn finished_fixture(project: &str, file_index: Option<usize>) -> Self {
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
            .map(|elapsed| u64_from_u128(elapsed.as_nanos()))
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

    pub(super) fn refresh_spectrum(&mut self) {
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

    pub(super) fn cache_duration(&mut self) {
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

    pub(super) fn play_sequential(
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
        let last = playlist.len().saturating_sub(1);
        let Some(next) = current.cast_signed().checked_add(delta) else {
            self.stop();
            return None;
        };
        let index = if next < 0 {
            if repeat { last } else { 0 }
        } else if next.cast_unsigned() > last {
            if repeat {
                0
            } else {
                self.stop();
                return None;
            }
        } else {
            next.cast_unsigned()
        };
        let path = playlist.get(index).cloned()?;
        self.play_file(project, path, index);
        Some(index)
    }

    pub(super) fn play_shuffled(
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
                let path = playlist.get(current).cloned()?;
                self.play_file(project, path, current);
                return Some(current);
            }
            self.stop();
            return None;
        };
        let index = index.min(playlist.len().saturating_sub(1));
        let path = playlist.get(index).cloned()?;
        self.play_file(project, path, index);
        Some(index)
    }

    pub(super) fn refill_bag(&mut self, len: usize, current: usize) {
        self.shuffle_bag = (0..len).filter(|index| *index != current).collect();
        playlist::fisher_yates(&mut self.shuffle_bag, &mut self.rng);
    }

    pub fn seek_fraction(&mut self, fraction: f32, duration: Duration) {
        // Seek position. Durations above 2^24 ms do not fit in an f32 mantissa.
        #[allow(clippy::cast_precision_loss)]
        let millis = duration.as_millis() as f32 * fraction.clamp(0.0, 1.0);
        self.engine
            .seek(Duration::from_millis(u64_from_f32(millis)));
    }

    pub fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        if (self.volume - volume).abs() <= 0.001 {
            return;
        }
        self.volume = volume;
        self.engine.volume(self.volume);
    }

    pub(crate) fn playing(&self) -> bool {
        matches!(self.status, Status::Playing { .. })
    }

    pub(super) fn click_track(&mut self, index: usize, shift: bool, ctrl: bool) {
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
