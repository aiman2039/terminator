//! Local echo prediction for high-latency sessions (Terminator #83).
//!
//! Behavior follows the `JediTerm` `TerminalTypeAheadManager` tests as a
//! specification. No `JediTerm` code is copied.
//!
//! - Every single printable ASCII key is recorded with its send time. The
//!   echo time (key to matching grid cell) feeds a moving average.
//! - `Auto` shows predictions only while that average is above
//!   [`SHOW_ABOVE`]; it hides them again below [`HIDE_BELOW`].
//! - The real echo confirms a prediction. A different character, a cursor
//!   that leaves the line, or any other key clears all predictions.
//! - Alternate screen and scrollback never predict.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Echo time above which `Auto` starts to show predictions.
pub const SHOW_ABOVE: Duration = Duration::from_millis(100);
/// Echo time below which `Auto` stops showing predictions again.
pub const HIDE_BELOW: Duration = Duration::from_millis(70);
/// A prediction with no echo after this time is dropped.
const STALE_AFTER: Duration = Duration::from_secs(2);
/// Weight of the newest sample in the moving average.
const ALPHA: f64 = 0.3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TypeAheadMode {
    Off,
    #[default]
    Auto,
    On,
}

/// Grid position as (line, column); line 0 is the top of the live screen.
pub type CellPoint = (i32, usize);

/// Terminal state the predictor reads at input and output time.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub cursor: CellPoint,
    pub columns: usize,
    /// Alternate screen or a scrolled-back view: never predict.
    pub blocked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Prediction {
    ch: char,
    sent: Instant,
}

#[derive(Debug, Default)]
pub struct TypeAhead {
    mode: TypeAheadMode,
    pending: VecDeque<Prediction>,
    /// Cell of the first pending prediction.
    origin: Option<CellPoint>,
    average: Option<Duration>,
    showing: bool,
}

impl TypeAhead {
    pub fn set_mode(&mut self, mode: TypeAheadMode) {
        if self.mode != mode {
            self.mode = mode;
            if mode == TypeAheadMode::Off {
                self.clear();
            }
        }
    }

    pub fn mode(&self) -> TypeAheadMode {
        self.mode
    }

    /// Measured key-to-echo time, if any sample exists.
    pub fn latency(&self) -> Option<Duration> {
        self.average
    }

    /// Record bytes the user sent to the PTY.
    pub fn on_input(&mut self, bytes: &[u8], screen: Screen, now: Instant) {
        if self.mode == TypeAheadMode::Off || screen.blocked {
            self.clear();
            return;
        }
        match bytes {
            [byte @ 0x20..=0x7e] => {
                let origin = *self.origin.get_or_insert(screen.cursor);
                let column = origin.1 + self.pending.len();
                // Stop at the right edge: wrapping is the shell's decision.
                if column + 1 >= screen.columns {
                    self.clear();
                    return;
                }
                self.pending.push_back(Prediction {
                    ch: char::from(*byte),
                    sent: now,
                });
            }
            // Backspace removes the newest unconfirmed prediction; the echo
            // of the erase and of the key cancel out.
            [0x7f | 0x08] if !self.pending.is_empty() => {
                self.pending.pop_back();
                if self.pending.is_empty() {
                    self.origin = None;
                }
            }
            _ => self.clear(),
        }
    }

    /// Compare pending predictions with the grid after new output.
    pub fn on_output(&mut self, screen: Screen, cell: impl Fn(CellPoint) -> char, now: Instant) {
        if screen.blocked {
            self.clear();
            return;
        }
        while let (Some(origin), Some(front)) = (self.origin, self.pending.front().copied()) {
            if screen.cursor.0 != origin.0 {
                self.clear();
                return;
            }
            if screen.cursor.1 <= origin.1 {
                if now.duration_since(front.sent) > STALE_AFTER {
                    self.clear();
                }
                return;
            }
            if cell(origin) != front.ch {
                self.clear();
                return;
            }
            self.sample(now.duration_since(front.sent));
            self.pending.pop_front();
            self.origin = Some((origin.0, origin.1 + 1));
        }
        if self.pending.is_empty() {
            self.origin = None;
        }
    }

    /// Unconfirmed characters to draw, with their cells. Empty unless shown.
    pub fn visible(&self) -> Vec<(CellPoint, char)> {
        let show = match self.mode {
            TypeAheadMode::Off => false,
            TypeAheadMode::On => true,
            TypeAheadMode::Auto => self.showing,
        };
        let Some(origin) = self.origin.filter(|_| show) else {
            return Vec::new();
        };
        self.pending
            .iter()
            .enumerate()
            .map(|(index, prediction)| ((origin.0, origin.1 + index), prediction.ch))
            .collect()
    }

    fn sample(&mut self, echo: Duration) {
        let average = self.average.map_or(echo, |previous| {
            previous.mul_f64(1.0 - ALPHA) + echo.mul_f64(ALPHA)
        });
        self.average = Some(average);
        if average > SHOW_ABOVE {
            self.showing = true;
        } else if average < HIDE_BELOW {
            self.showing = false;
        }
    }

    fn clear(&mut self) {
        self.pending.clear();
        self.origin = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(column: usize) -> Screen {
        Screen {
            cursor: (3, column),
            columns: 80,
            blocked: false,
        }
    }

    /// Type `text` at column 2, then echo it after `delay`.
    fn typed(ahead: &mut TypeAhead, text: &str, start: Instant) {
        for (index, byte) in text.bytes().enumerate() {
            ahead.on_input(&[byte], screen(2 + index), start);
        }
    }

    fn echo(ahead: &mut TypeAhead, text: &str, at: Instant) {
        let line: Vec<char> = format!("$ {text}").chars().collect();
        ahead.on_output(
            screen(2 + text.len()),
            |(_, column)| line.get(column).copied().unwrap_or(' '),
            at,
        );
    }

    fn slow_link(ahead: &mut TypeAhead, start: Instant) -> Instant {
        typed(ahead, "ls", start);
        let later = start + Duration::from_millis(250);
        echo(ahead, "ls", later);
        later
    }

    #[test]
    fn prediction_confirmed_by_echo() {
        let mut ahead = TypeAhead::default();
        let start = Instant::now();
        let later = slow_link(&mut ahead, start);
        assert_eq!(ahead.latency(), Some(Duration::from_millis(250)));
        typed(&mut ahead, "x", later);
        assert_eq!(ahead.visible(), vec![((3, 2), 'x')]);
        echo(&mut ahead, "x", later + Duration::from_millis(200));
        assert!(ahead.visible().is_empty());
    }

    #[test]
    fn low_latency_keeps_prediction_off() {
        let mut ahead = TypeAhead::default();
        let start = Instant::now();
        typed(&mut ahead, "ls", start);
        echo(&mut ahead, "ls", start + Duration::from_millis(5));
        typed(&mut ahead, "x", start);
        assert!(ahead.visible().is_empty());
        ahead.set_mode(TypeAheadMode::On);
        assert_eq!(ahead.visible(), vec![((3, 2), 'x')]);
    }

    #[test]
    fn mismatch_clears_predictions() {
        let mut ahead = TypeAhead::default();
        let later = slow_link(&mut ahead, Instant::now());
        typed(&mut ahead, "ab", later);
        assert_eq!(ahead.visible().len(), 2);
        // The shell echoed something else (for example a completion).
        echo(&mut ahead, "zz", later);
        assert!(ahead.visible().is_empty());
    }

    #[test]
    fn backspace_removes_prediction() {
        let mut ahead = TypeAhead::default();
        let later = slow_link(&mut ahead, Instant::now());
        typed(&mut ahead, "ab", later);
        ahead.on_input(&[0x7f], screen(4), later);
        assert_eq!(ahead.visible(), vec![((3, 2), 'a')]);
    }

    #[test]
    fn alt_screen_disables_prediction() {
        let mut ahead = TypeAhead::default();
        let later = slow_link(&mut ahead, Instant::now());
        let mut blocked = screen(2);
        blocked.blocked = true;
        ahead.on_input(b"j", blocked, later);
        assert!(ahead.visible().is_empty());
        typed(&mut ahead, "a", later);
        ahead.on_output(blocked, |_| 'a', later);
        assert!(ahead.visible().is_empty());
    }

    #[test]
    fn control_keys_and_pastes_clear_predictions() {
        let mut ahead = TypeAhead::default();
        let later = slow_link(&mut ahead, Instant::now());
        typed(&mut ahead, "a", later);
        ahead.on_input(b"\r", screen(3), later);
        assert!(ahead.visible().is_empty());
        typed(&mut ahead, "a", later);
        ahead.on_input(b"pasted", screen(3), later);
        assert!(ahead.visible().is_empty());
    }

    #[test]
    fn cursor_leaving_the_line_clears_predictions() {
        let mut ahead = TypeAhead::default();
        let later = slow_link(&mut ahead, Instant::now());
        typed(&mut ahead, "a", later);
        let moved = Screen {
            cursor: (4, 0),
            columns: 80,
            blocked: false,
        };
        ahead.on_output(moved, |_| ' ', later);
        assert!(ahead.visible().is_empty());
    }

    #[test]
    fn off_mode_never_predicts() {
        let mut ahead = TypeAhead::default();
        let later = slow_link(&mut ahead, Instant::now());
        ahead.set_mode(TypeAheadMode::Off);
        typed(&mut ahead, "a", later);
        assert!(ahead.visible().is_empty());
    }
}
