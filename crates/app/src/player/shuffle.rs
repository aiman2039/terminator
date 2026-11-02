//! Shuffle rounds: each track plays once per round; Previous walks back.

use super::controller::{Controller, Skip};
use super::playlist;

/// Previous steps kept for shuffle.
pub(super) const HISTORY_LIMIT: usize = 1000;

impl Controller {
    /// Next plays each track once per round (stopping at the end unless
    /// `repeat`); Previous walks back through the tracks played this round.
    pub(super) fn play_shuffled(&mut self, skip: Skip<'_>) -> Option<usize> {
        let Skip {
            project,
            playlist,
            delta,
            repeat,
            ..
        } = skip;
        let current = self.file_index.unwrap_or(0);
        if delta < 0 {
            let index = if self.shuffle_history.len() >= 2 {
                if let Some(left) = self.shuffle_history.pop() {
                    self.shuffle_bag.push(left);
                }
                self.shuffle_history.last().copied().unwrap_or(current)
            } else {
                current
            };
            let path = playlist.get(index).cloned()?;
            self.play_file(project, path, index);
            return Some(index);
        }
        if self.shuffle_bag.is_empty() {
            if self.shuffle_round && !repeat {
                self.reset_shuffle();
                self.stop();
                return None;
            }
            self.refill_bag(playlist.len(), current);
            self.shuffle_round = true;
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

    /// Starts a new shuffle round (playlist edits, shuffle toggled).
    pub(crate) fn reset_shuffle(&mut self) {
        self.shuffle_bag.clear();
        self.shuffle_history.clear();
        self.shuffle_round = false;
    }
}
