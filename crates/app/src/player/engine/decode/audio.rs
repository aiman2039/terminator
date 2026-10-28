use super::super::super::tap;
use super::super::analysis::Analysis;
use super::super::handle::{Controls, Desired, Event, MAX_PCM_SAMPLES, Playable, Status};
use super::super::pipeline::{
    PipelineGuard, PlaybackCounters, PreparedSource, StartOutputInput, StreamInput, StreamReader,
    start_output,
};
use super::Permanent;
use anyhow::{Context, Result, ensure};
use cpal::traits::{DeviceTrait, HostTrait};
use rodio::Source;
use std::{
    fs::File,
    num::NonZero,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use terminator_core::async_service::CancellationToken;
use tokio::sync::{mpsc, watch};

pub(in super::super) struct DecodeAudioInput {
    pub(in super::super) desired: Desired,
    pub(in super::super) controls: Arc<Controls>,
    pub(in super::super) status: watch::Sender<Event>,
    pub(in super::super) spectrum: Analysis,
    pub(in super::super) receiver: mpsc::Receiver<Vec<u8>>,
    pub(in super::super) cancel: CancellationToken,
    pub(in super::super) ready: Arc<AtomicBool>,
    pub(in super::super) counters: Arc<PlaybackCounters>,
    pub(in super::super) healthy: Arc<AtomicBool>,
}

type BoxedSource = Box<dyn Source<Item = f32> + Send>;

pub(in super::super) fn decode_audio(input: DecodeAudioInput) -> Result<()> {
    let DecodeAudioInput {
        desired,
        controls,
        status,
        spectrum,
        receiver,
        cancel,
        ready,
        counters,
        healthy,
    } = input;
    let _decoder = PipelineGuard::new(Arc::clone(&controls), false);
    let source = desired.source.as_ref().context("audio source missing")?;
    let mut decoder = open_decoder(source, receiver, &cancel)?;
    if !desired.offset.is_zero() {
        decoder
            .try_seek(desired.offset)
            .map_err(|e| anyhow::anyhow!("Seek failed: {e}"))?;
    }
    let duration = decoder.total_duration();
    let device = OutputDevice::open()?;
    // Resampling/channel conversion stays on this native decoder worker.
    let decoder = rodio::source::UniformSourceIterator::new(decoder, device.channels, device.rate);
    let (capacity, threshold) = device.buffer_sizes();
    let (pcm, consumer) = rtrb::RingBuffer::new(capacity);
    let (tap_producer, tap_consumer) = rtrb::RingBuffer::new(tap::FFT_N * 2);
    let prepared = PreparedSource {
        pcm: consumer,
        tap: tap_producer,
        controls: Arc::clone(&controls),
        counters: Arc::clone(&counters),
        generation: desired.generation,
        channels: device.channels,
        rate: device.rate,
        channel: 0,
        mix: 0.0,
    };
    let title = source.title().to_owned();
    let radio = source.radio();
    DecodeLoop {
        desired: &desired,
        title,
        radio,
        duration,
        controls,
        status,
        spectrum,
        cancel,
        ready,
        counters,
        healthy,
        decoder,
        device,
        threshold,
        pcm,
        tap: tap_consumer,
        prepared: Some(prepared),
        output: None,
        output_failed: Arc::new(AtomicBool::new(false)),
        produced: 0,
        next: None,
        eof: false,
        tick: Instant::now()
            .checked_sub(Duration::from_millis(100))
            .unwrap_or_else(Instant::now),
        window: SpectrumWindow::default(),
    }
    .run()
}

fn open_decoder(
    source: &Playable,
    receiver: mpsc::Receiver<Vec<u8>>,
    cancel: &CancellationToken,
) -> Result<BoxedSource> {
    Ok(match source {
        Playable::File { path, .. } => {
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;
            let mut options = File::options();
            options.read(true);
            // Avoid blocking on FIFOs; Windows has no equivalent flag.
            #[cfg(unix)]
            options.custom_flags(libc::O_NONBLOCK);
            let file = options.open(path).context("Open audio file")?;
            ensure!(
                file.metadata()?.is_file(),
                "Audio playback requires a regular file"
            );
            Box::new(
                rodio::Decoder::try_from(file)
                    .map_err(|_| Permanent("Unsupported or invalid audio file"))?,
            )
        }
        Playable::Stream { .. } => Box::new(
            rodio::Decoder::builder()
                .with_data(StreamReader {
                    inner: Mutex::new(StreamInput {
                        receiver,
                        current: std::io::Cursor::new(Vec::new()),
                        position: 0,
                        cancel: cancel.clone(),
                    }),
                })
                .with_seekable(false)
                .build()
                .map_err(|_| Permanent("Unsupported or invalid radio audio"))?,
        ),
    })
}

/// The default output device and its native format.
struct OutputDevice {
    device: cpal::Device,
    configuration: cpal::SupportedStreamConfig,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
}

impl OutputDevice {
    fn open() -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(Permanent("No audio output device"))?;
        let configuration = device
            .default_output_config()
            .map_err(|_| Permanent("Audio output device configuration is unavailable"))?;
        let channels = NonZero::new(configuration.channels())
            .ok_or(Permanent("Invalid audio sample format"))?;
        let rate = NonZero::new(configuration.sample_rate())
            .ok_or(Permanent("Invalid audio sample format"))?;
        Ok(Self {
            device,
            configuration,
            channels,
            rate,
        })
    }

    /// Ring buffer capacity (two seconds, capped) and the fill level that
    /// starts output (a quarter second).
    fn buffer_sizes(&self) -> (usize, usize) {
        let rate_samples = usize::try_from(self.rate.get()).unwrap_or(usize::MAX);
        let channel_count = usize::from(self.channels.get());
        let capacity = rate_samples
            .saturating_mul(channel_count)
            .saturating_mul(2)
            .min(MAX_PCM_SAMPLES);
        let threshold = rate_samples
            .saturating_mul(channel_count)
            .checked_div(4)
            .unwrap_or(0)
            .min(capacity);
        (capacity, threshold)
    }
}

/// Circular buffer of the latest tapped samples for the spectrum.
struct SpectrumWindow {
    samples: [f32; tap::FFT_N],
    index: usize,
    sampled: usize,
}

impl Default for SpectrumWindow {
    fn default() -> Self {
        Self {
            samples: [0.0; tap::FFT_N],
            index: 0,
            sampled: 0,
        }
    }
}

impl SpectrumWindow {
    fn drain(&mut self, tap: &mut rtrb::Consumer<f32>) {
        while let Ok(sample) = tap.pop() {
            if let Some(slot) = self.samples.get_mut(self.index) {
                *slot = sample;
            }
            self.index = self
                .index
                .checked_add(1)
                .and_then(|index| index.checked_rem(tap::FFT_N))
                .unwrap_or(0);
            self.sampled = self.sampled.saturating_add(1);
        }
    }

    /// The buffer in time order, once it has filled at least once.
    fn ordered(&self) -> Option<[f32; tap::FFT_N]> {
        if self.sampled < tap::FFT_N {
            return None;
        }
        let mut window = [0.0; tap::FFT_N];
        for (i, sample) in window.iter_mut().enumerate() {
            if let Some(value) = self
                .index
                .checked_add(i)
                .and_then(|index| index.checked_rem(tap::FFT_N))
                .and_then(|index| self.samples.get(index))
            {
                *sample = *value;
            }
        }
        Some(window)
    }
}

/// Decoder-side state for one playback: fills the PCM ring, starts output
/// once enough is buffered, and reports status and spectrum.
struct DecodeLoop<'a> {
    desired: &'a Desired,
    title: String,
    radio: bool,
    duration: Option<Duration>,
    controls: Arc<Controls>,
    status: watch::Sender<Event>,
    spectrum: Analysis,
    cancel: CancellationToken,
    ready: Arc<AtomicBool>,
    counters: Arc<PlaybackCounters>,
    healthy: Arc<AtomicBool>,
    decoder: rodio::source::UniformSourceIterator<BoxedSource>,
    device: OutputDevice,
    threshold: usize,
    pcm: rtrb::Producer<f32>,
    tap: rtrb::Consumer<f32>,
    prepared: Option<PreparedSource>,
    output: Option<cpal::Stream>,
    output_failed: Arc<AtomicBool>,
    produced: usize,
    next: Option<f32>,
    eof: bool,
    tick: Instant,
    window: SpectrumWindow,
}

impl DecodeLoop<'_> {
    fn run(mut self) -> Result<()> {
        loop {
            if self.output_failed.load(Ordering::Acquire) {
                return Err(Permanent("Audio output device disconnected").into());
            }
            if self.cancel.is_cancelled()
                || self.controls.generation.load(Ordering::Acquire) != self.desired.generation
            {
                break;
            }
            self.decode_next();
            self.start_output_when_buffered()?;
            if self.tick.elapsed() >= Duration::from_millis(50) && !self.report() {
                continue;
            }
            if self.eof && self.counters.drained.load(Ordering::Acquire) {
                break;
            }
            if self.next.is_some() || self.eof {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        drop(self.output);
        Ok(())
    }

    /// Pull the next sample (unless one is pending) and push it to the ring.
    fn decode_next(&mut self) {
        if !self.eof && self.next.is_none() {
            self.next = self.decoder.next();
            if self.next.is_none() {
                self.eof = true;
                self.counters.finished.store(true, Ordering::Release);
            }
        }
        if let Some(sample) = self.next
            && self.pcm.push(sample).is_ok()
        {
            self.next = None;
            self.produced = self.produced.saturating_add(1);
        }
    }

    fn start_output_when_buffered(&mut self) -> Result<()> {
        if self.output.is_some() || (self.produced < self.threshold && !self.eof) {
            return Ok(());
        }
        let stream = start_output(StartOutputInput {
            device: &self.device.device,
            config: &self.device.configuration,
            source: self
                .prepared
                .take()
                .context("audio output was not prepared")?,
            failed: Arc::clone(&self.output_failed),
        })?;
        self.output = Some(stream);
        self.ready.store(true, Ordering::Release);
        Ok(())
    }

    /// Periodic spectrum and status report. False skips the rest of this
    /// loop pass, as when the position cannot be represented.
    fn report(&mut self) -> bool {
        self.tick = Instant::now();
        self.window.drain(&mut self.tap);
        let rate = self.device.rate.get();
        if let Some(window) = self.window.ordered() {
            self.spectrum.submit(window, rate, self.desired.generation);
        }
        if !self.ready.load(Ordering::Acquire) {
            return true;
        }
        let channels = self.device.channels.get();
        let played = self.counters.consumed.load(Ordering::Relaxed);
        let frame = u64::from(rate).saturating_mul(u64::from(channels));
        if played >= frame.saturating_mul(60) {
            self.healthy.store(true, Ordering::Release);
        }
        // Playback position. Sample counts above 2^53 do not fit in an f64 mantissa.
        let seconds = played as f64 / f64::from(rate) / f64::from(channels);
        let Ok(elapsed) = Duration::try_from_secs_f64(seconds) else {
            return false;
        };
        let Some(position) = self.desired.offset.checked_add(elapsed) else {
            return false;
        };
        self.status.send_replace(Event {
            generation: self.desired.generation,
            status: self.playback_status(position),
            finished: false,
        });
        true
    }

    fn playback_status(&self, position: Duration) -> Status {
        let title = self.title.clone();
        let (duration, seekable) = (self.duration, !self.radio);
        if self.controls.paused.load(Ordering::Acquire) {
            Status::Paused {
                title,
                position,
                duration,
                seekable,
            }
        } else if self.counters.buffering.load(Ordering::Relaxed) {
            Status::Buffering { title }
        } else {
            Status::Playing {
                title,
                position,
                duration,
                seekable,
            }
        }
    }
}
