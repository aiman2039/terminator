//! Playback tap + FFT bars via `spectrum-analyzer`.
use rodio::Source;
use spectrum_analyzer::scaling::divide_by_N;
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const BARS: usize = 32;
const FFT_N: usize = 2048;
const MIN_HZ: f32 = 40.0;
const MAX_HZ: f32 = 16_000.0;

#[derive(Clone)]
pub struct Handle {
    inner: Arc<Mutex<Tap>>,
}

struct Tap {
    samples: Vec<f32>,
    write: usize,
    filled: bool,
    channels: u16,
    rate: u32,
    channel_i: u16,
    mix: f32,
}

pub struct Tapped<S> {
    inner: S,
    tap: Handle,
}

impl Handle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Tap {
                samples: vec![0.0; FFT_N],
                write: 0,
                filled: false,
                channels: 0,
                rate: 0,
                channel_i: 0,
                mix: 0.0,
            })),
        }
    }

    pub fn clear(&self) {
        let Ok(mut tap) = self.inner.lock() else {
            return;
        };
        tap.write = 0;
        tap.filled = false;
        tap.channel_i = 0;
        tap.mix = 0.0;
        tap.samples.fill(0.0);
    }

    pub fn wrap<S: Source>(&self, inner: S) -> Tapped<S> {
        Tapped {
            inner,
            tap: self.clone(),
        }
    }

    pub fn bars(&self) -> Option<[f32; BARS]> {
        let window = {
            let Ok(tap) = self.inner.lock() else {
                return None;
            };
            tap.window()?
        };
        Some(bars_from_window(&window))
    }
}

impl Tap {
    fn push(&mut self, sample: f32, channels: u16, rate: u32) {
        if channels == 0 {
            return;
        }
        if self.channels != channels || self.rate != rate {
            self.channels = channels;
            self.rate = rate;
            self.write = 0;
            self.filled = false;
            self.channel_i = 0;
            self.mix = 0.0;
            self.samples.fill(0.0);
        }
        self.mix += sample;
        self.channel_i = self.channel_i.saturating_add(1);
        if self.channel_i < channels {
            return;
        }
        let mono = self.mix / f32::from(channels);
        self.mix = 0.0;
        self.channel_i = 0;
        let Some(slot) = self.samples.get_mut(self.write) else {
            return;
        };
        *slot = mono;
        self.write += 1;
        if self.write >= FFT_N {
            self.write = 0;
            self.filled = true;
        }
    }

    fn window(&self) -> Option<Window> {
        if !self.filled || self.rate == 0 {
            return None;
        }
        let mut samples = [0.0; FFT_N];
        for (i, slot) in samples.iter_mut().enumerate() {
            let index = (self.write + i) % FFT_N;
            *slot = self.samples.get(index).copied().unwrap_or(0.0);
        }
        Some(Window {
            samples,
            rate: self.rate,
        })
    }
}

struct Window {
    samples: [f32; FFT_N],
    rate: u32,
}

fn bars_from_window(window: &Window) -> [f32; BARS] {
    let mut bars = [0.0; BARS];
    let hann = hann_window(&window.samples);
    let nyquist = window.rate as f32 / 2.0;
    let max_hz = MAX_HZ.min(nyquist);
    let limit = if max_hz > MIN_HZ {
        FrequencyLimit::range(MIN_HZ, max_hz)
    } else {
        FrequencyLimit::All
    };
    let Ok(spectrum) = samples_fft_to_spectrum(&hann, window.rate, limit, Some(&divide_by_N))
    else {
        return bars;
    };
    let data = spectrum.to_vec();
    let span = (max_hz / MIN_HZ).log2();
    for (index, bar) in bars.iter_mut().enumerate() {
        let lo = MIN_HZ * 2.0_f32.powf(index as f32 / BARS as f32 * span);
        let hi = MIN_HZ * 2.0_f32.powf((index + 1) as f32 / BARS as f32 * span);
        let mut mag = 0.0_f32;
        let mut hit = false;
        for &(hz, value) in &data {
            if hz >= lo && hz < hi {
                mag = mag.max(value);
                hit = true;
            }
        }
        if !hit {
            mag = interpolate_hz(&data, (lo * hi).sqrt());
        }
        let db = 20.0 * (mag + 1.0e-9).log10();
        *bar = ((db + 80.0) / 80.0).clamp(0.0, 1.0);
    }
    bars
}

fn interpolate_hz(data: &[(f32, f32)], hz: f32) -> f32 {
    let Some(first) = data.first() else {
        return 0.0;
    };
    let Some(last) = data.last() else {
        return 0.0;
    };
    if hz <= first.0 {
        return first.1;
    }
    if hz >= last.0 {
        return last.1;
    }
    for pair in data.windows(2) {
        let (f0, v0) = pair[0];
        let (f1, v1) = pair[1];
        if hz >= f0 && hz <= f1 {
            let span = (f1 - f0).max(1.0e-9);
            return v0 + (hz - f0) / span * (v1 - v0);
        }
    }
    0.0
}

impl<S: Source> Iterator for Tapped<S> {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        let channels = self.inner.channels();
        let rate = self.inner.sample_rate();
        if let Ok(mut tap) = self.tap.inner.try_lock() {
            tap.push(sample, channels, rate);
        }
        Some(sample)
    }
}

impl<S: Source> Source for Tapped<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peaks_in_the_expected_log_band() {
        let tap = Handle::new();
        let rate = 44_100;
        let hz = 440.0_f32;
        {
            let Ok(mut inner) = tap.inner.lock() else {
                panic!("tap lock");
            };
            for i in 0..(FFT_N * 2) {
                let sample = (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin();
                inner.push(sample, 1, rate);
            }
        }
        let bars = tap.bars().expect("full window");
        let span = (MAX_HZ / MIN_HZ).log2();
        let expected = ((hz / MIN_HZ).log2() / span * BARS as f32) as usize;
        let peak = bars
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
            .expect("peak");
        assert!(
            peak.abs_diff(expected) <= 2,
            "peak={peak} expected={expected} bars={bars:?}"
        );
    }

    #[test]
    fn impulse_fills_every_log_bar() {
        let tap = Handle::new();
        {
            let Ok(mut inner) = tap.inner.lock() else {
                panic!("tap lock");
            };
            for i in 0..FFT_N {
                let sample = if i == FFT_N / 2 { 1.0 } else { 0.0 };
                inner.push(sample, 1, 44_100);
            }
        }
        let bars = tap.bars().expect("full window");
        for (index, bar) in bars.iter().enumerate() {
            assert!(
                *bar > 0.02,
                "bar {index} should not be empty after interpolate, bars={bars:?}"
            );
        }
    }
}
