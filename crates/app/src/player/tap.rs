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
    // Spectrum frequency coordinate; rates above 2^24 do not fit in an f32 mantissa.
    #[allow(clippy::cast_precision_loss)]
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
        let index_f = f32::from(u16::try_from(index).unwrap_or(u16::MAX));
        let bars_f = f32::from(u16::try_from(BARS).unwrap_or(u16::MAX));
        let next_f = f32::from(u16::try_from(index.saturating_add(1)).unwrap_or(u16::MAX));
        let lo = MIN_HZ * 2.0_f32.powf(index_f / bars_f * span);
        let hi = MIN_HZ * 2.0_f32.powf(next_f / bars_f * span);
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
        let Some(&(f0, v0)) = pair.first() else {
            continue;
        };
        let Some(&(f1, v1)) = pair.get(1) else {
            continue;
        };
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

    fn trunc_usize(value: f32) -> usize {
        if !value.is_finite() || value <= 0.0 {
            return 0;
        }
        let bits = value.to_bits();
        let Ok(biased) = u8::try_from((bits >> 23) & 0xff) else {
            return 0;
        };
        let Some(exp) = i32::from(biased).checked_sub(127) else {
            return 0;
        };
        if exp < 0 {
            return 0;
        }
        let mant = u64::from((bits & 0x007f_ffff) | 0x0080_0000);
        let Some(shift) = exp.checked_sub(23) else {
            return 0;
        };
        let mag = if shift >= 0 {
            mant.checked_shl(shift.cast_unsigned()).unwrap_or(u64::MAX)
        } else {
            mant.checked_shr(shift.unsigned_abs()).unwrap_or(0)
        };
        usize::try_from(mag).unwrap_or(usize::MAX)
    }

    #[test]
    fn sine_peaks_in_the_expected_log_band() {
        let mut samples = [0.0; FFT_N];
        for (i, sample) in samples.iter_mut().enumerate() {
            let i_f = f32::from(u16::try_from(i).unwrap_or(u16::MAX));
            *sample = (std::f32::consts::TAU * 440.0 * i_f / 44100.0).sin();
        }
        let bars = bars_from_samples(&samples, 44100);
        let peak = bars
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .unwrap()
            .0;
        let expected_f = (440.0_f32 / MIN_HZ).log2() / (MAX_HZ / MIN_HZ).log2()
            * f32::from(u16::try_from(BARS).unwrap_or(u16::MAX));
        let expected = trunc_usize(expected_f);
        assert!(peak.abs_diff(expected) <= 2);
    }
}
