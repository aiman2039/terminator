//! Ten-band peaking equalizer applied to PCM in the output callback.
use biquad::{Biquad, Coefficients, DirectForm2Transposed, Type};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Band centres, matching the sliders after the preamp.
pub(crate) const BAND_HZ: [f32; 10] = [
    60.0, 170.0, 310.0, 600.0, 1_000.0, 3_000.0, 6_000.0, 12_000.0, 14_000.0, 16_000.0,
];
const RANGE_DB: f32 = 12.0;
const Q: f32 = 1.0;
const IDENTITY: Coefficients<f32> = Coefficients {
    a1: 0.0,
    a2: 0.0,
    b0: 1.0,
    b1: 0.0,
    b2: 0.0,
};

/// Slider value in 0..=1 (0.5 is flat) to decibels.
pub(crate) fn slider_db(value: f32) -> f32 {
    (value.clamp(0.0, 1.0) - 0.5) * 2.0 * RANGE_DB
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EqSettings {
    pub(crate) on: bool,
    pub(crate) preamp: f32,
    pub(crate) bands: [f32; 10],
}

impl EqSettings {
    fn bypass(&self) -> bool {
        let flat = |value: f32| (value - 0.5).abs() < 1e-3;
        !self.on || (flat(self.preamp) && self.bands.iter().all(|band| flat(*band)))
    }
}

/// GUI-to-audio settings. Version 0 means "never set" and bypasses.
#[derive(Default)]
pub(crate) struct EqShared {
    version: AtomicU64,
    on: AtomicBool,
    preamp: AtomicU32,
    bands: [AtomicU32; 10],
}

impl EqShared {
    pub(crate) fn store(&self, settings: EqSettings) {
        self.on.store(settings.on, Ordering::Release);
        self.preamp
            .store(settings.preamp.to_bits(), Ordering::Release);
        for (slot, value) in self.bands.iter().zip(settings.bands) {
            slot.store(value.to_bits(), Ordering::Release);
        }
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    /// A consistent snapshot, or `None` while a store is in progress.
    fn load(&self) -> Option<(u64, EqSettings)> {
        let version = self.version.load(Ordering::Acquire);
        let mut bands = [0.5; 10];
        for (value, slot) in bands.iter_mut().zip(&self.bands) {
            *value = f32::from_bits(slot.load(Ordering::Acquire));
        }
        let settings = EqSettings {
            on: self.on.load(Ordering::Acquire),
            preamp: f32::from_bits(self.preamp.load(Ordering::Acquire)),
            bands,
        };
        (self.version.load(Ordering::Acquire) == version).then_some((version, settings))
    }
}

/// Per-stream filter state. Coefficients are rebuilt only when settings change.
pub(crate) struct Equalizer {
    rate: f32,
    seen: u64,
    active: bool,
    gain: f32,
    filters: Vec<[DirectForm2Transposed<f32>; 10]>,
}

impl Equalizer {
    pub(crate) fn new(channels: u16, rate: u32) -> Self {
        let chain = [DirectForm2Transposed::<f32>::new(IDENTITY); 10];
        Self {
            rate: rate_f32(rate),
            seen: 0,
            active: false,
            gain: 1.0,
            filters: vec![chain; usize::from(channels.max(1))],
        }
    }

    pub(crate) fn process(&mut self, shared: &EqShared, channel: u16, sample: f32) -> f32 {
        if channel == 0 {
            self.refresh(shared);
        }
        if !self.active {
            return sample;
        }
        let Some(chain) = self.filters.get_mut(usize::from(channel)) else {
            return sample;
        };
        chain
            .iter_mut()
            .fold(sample * self.gain, |value, filter| filter.run(value))
    }

    fn refresh(&mut self, shared: &EqShared) {
        if shared.version.load(Ordering::Acquire) == self.seen {
            return;
        }
        if let Some((version, settings)) = shared.load() {
            self.seen = version;
            self.apply(settings);
        }
    }

    fn apply(&mut self, settings: EqSettings) {
        self.active = !settings.bypass();
        self.gain = 10_f32.powf(slider_db(settings.preamp) / 20.0);
        for (index, (hz, value)) in BAND_HZ.iter().zip(settings.bands).enumerate() {
            let coefficients = peaking(self.rate, *hz, slider_db(value));
            for chain in &mut self.filters {
                if let Some(filter) = chain.get_mut(index) {
                    filter.update_coefficients(coefficients);
                }
            }
        }
    }
}

fn rate_f32(rate: u32) -> f32 {
    // Exact for every real sample rate (< 2^24).
    f32::from(u16::try_from(rate >> 8).unwrap_or(u16::MAX)) * 256.0
        + f32::from(u8::try_from(rate & 0xff).unwrap_or(0))
}

/// Bands at or above Nyquist pass through unchanged.
fn peaking(rate: f32, hz: f32, db: f32) -> Coefficients<f32> {
    // biquad: normalized frequency where 1.0 is Nyquist (omega = PI * f).
    let normalized = 2.0 * hz / rate;
    if !(0.0..0.95).contains(&normalized) {
        return IDENTITY;
    }
    Coefficients::from_normalized_params(Type::PeakingEQ(db), normalized, Q).unwrap_or(IDENTITY)
}
