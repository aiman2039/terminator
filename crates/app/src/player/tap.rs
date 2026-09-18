//! FFT preparation runs on the decoder worker, never in the audio callback or UI.
use spectrum_analyzer::scaling::divide_by_N;
use spectrum_analyzer::windows::hann_window;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};
pub const BARS: usize = 32;
pub const FFT_N: usize = 2048;
const MIN_HZ: f32 = 40.0;
const MAX_HZ: f32 = 16_000.0;
pub fn bars_from_samples(samples: &[f32; FFT_N], rate: u32) -> [f32; BARS] {
    let mut bars = [0.0; BARS];
    let hann = hann_window(samples);
    let nyquist = rate as f32 / 2.0;
    let max_hz = MAX_HZ.min(nyquist);
    let limit = if max_hz > MIN_HZ {
        FrequencyLimit::range(MIN_HZ, max_hz)
    } else {
        FrequencyLimit::All
    };
    let Ok(spectrum) = samples_fft_to_spectrum(&hann, rate, limit, Some(&divide_by_N)) else {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sine_peaks_in_the_expected_log_band() {
        let mut samples = [0.0; FFT_N];
        for (i, sample) in samples.iter_mut().enumerate() {
            *sample = (std::f32::consts::TAU * 440.0 * i as f32 / 44100.0).sin();
        }
        let bars = bars_from_samples(&samples, 44100);
        let peak = bars
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .unwrap()
            .0;
        let expected =
            ((440.0_f32 / MIN_HZ).log2() / (MAX_HZ / MIN_HZ).log2() * BARS as f32) as usize;
        assert!(peak.abs_diff(expected) <= 2);
    }
}
