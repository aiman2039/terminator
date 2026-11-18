use super::analysis::{Analysis, download};
use super::decode::{DecodeAudioInput, Permanent, decode_audio};
use super::equalizer::Equalizer;
use super::handle::{COMPRESSED_SLOTS, Controls, Desired, Event, Playable, Status};
use crate::player::engine::DownloadInput;
use anyhow::{Context, Result, ensure};
use cpal::traits::{DeviceTrait, StreamTrait};
use rodio::Source;
use std::{
    io::{self, Read, Seek, SeekFrom},
    num::NonZero,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use terminator_core::async_service::{CancellationToken, NativePool};
use tokio::sync::{mpsc, watch};
pub(super) struct PipelineGuard {
    controls: Arc<Controls>,
    network: bool,
}
impl PipelineGuard {
    pub(super) fn new(controls: Arc<Controls>, network: bool) -> Self {
        if network {
            controls.connections.fetch_add(1, Ordering::AcqRel);
        } else {
            controls.decoders.fetch_add(1, Ordering::AcqRel);
        }
        Self { controls, network }
    }
}
impl Drop for PipelineGuard {
    fn drop(&mut self) {
        if self.network {
            self.controls.connections.fetch_sub(1, Ordering::AcqRel);
        } else {
            self.controls.decoders.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
pub(super) struct PipelineInput<'a> {
    pub(super) client: &'a reqwest::Client,
    pub(super) pool: &'a NativePool,
    pub(super) desired: Desired,
    pub(super) controls: Arc<Controls>,
    pub(super) status: watch::Sender<Event>,
    pub(super) spectrum: Analysis,
    pub(super) cancel: CancellationToken,
    pub(super) healthy: Arc<AtomicBool>,
}

pub(super) async fn pipeline(input: PipelineInput<'_>) -> Result<()> {
    let PipelineInput {
        client,
        pool,
        desired,
        controls,
        status,
        spectrum,
        cancel,
        healthy,
    } = input;
    let _cancel_on_drop = cancel.clone().drop_guard();
    let (sender, receiver) = mpsc::channel(COMPRESSED_SLOTS);
    let source = desired.source.clone().context("audio source missing")?;
    let stall = source.radio().then_some(RADIO_STALL);
    let network = feed_network(FeedInput {
        client,
        source,
        controls: Arc::clone(&controls),
        sender,
    });
    let ready = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(PlaybackCounters::default());
    let monitor = PlaybackMonitor {
        started: Instant::now(),
        stall,
        ready: Arc::clone(&ready),
        counters: Arc::clone(&counters),
        status: status.clone(),
        controls: Arc::clone(&controls),
        title: desired
            .source
            .as_ref()
            .context("audio source missing")?
            .title()
            .to_owned(),
        generation: desired.generation,
    };
    let decode_cancel = cancel.clone();
    let decode = pool.run(&cancel, move || {
        decode_audio(DecodeAudioInput {
            desired,
            controls,
            status,
            spectrum,
            receiver,
            cancel: decode_cancel,
            ready,
            counters,
            healthy,
        })
    });
    tokio::select! {
        result = async { tokio::try_join!(network, decode)?; Ok(()) } => result,
        result = monitor.watch() => result,
    }
}

struct FeedInput<'a> {
    client: &'a reqwest::Client,
    source: Playable,
    controls: Arc<Controls>,
    sender: mpsc::Sender<Vec<u8>>,
}

/// Download a stream source into the decoder's channel; file sources close
/// the channel at once.
async fn feed_network(input: FeedInput<'_>) -> Result<()> {
    let FeedInput {
        client,
        source,
        controls,
        sender,
    } = input;
    let Playable::Stream { url, .. } = source else {
        drop(sender);
        return Ok(());
    };
    let _network = PipelineGuard::new(controls, true);
    download(DownloadInput {
        client,
        url,
        sender,
        headers: Duration::from_secs(8),
        read_timeout: Duration::from_secs(10),
    })
    .await
}

/// How long a playing radio stream can give no audio before a reconnect.
/// A network stall has its own read deadline; this catches a decoder that
/// reads data but makes no samples.
const RADIO_STALL: Duration = Duration::from_secs(15);

/// Fails playback that never starts, reports buffering once it has, and
/// fails a radio stream that stays without audio.
struct PlaybackMonitor {
    started: Instant,
    stall: Option<Duration>,
    ready: Arc<AtomicBool>,
    counters: Arc<PlaybackCounters>,
    status: watch::Sender<Event>,
    controls: Arc<Controls>,
    title: String,
    generation: u64,
}

impl PlaybackMonitor {
    async fn watch(self) -> Result<()> {
        let mut silent_since: Option<Instant> = None;
        loop {
            if self.ready.load(Ordering::Acquire) {
                let silent = self.counters.buffering.load(Ordering::Relaxed)
                    && !self.controls.paused.load(Ordering::Acquire);
                if silent {
                    self.report_buffering();
                }
                silent_since = silent.then(|| silent_since.unwrap_or_else(Instant::now));
                if let (Some(since), Some(stall)) = (silent_since, self.stall) {
                    ensure!(since.elapsed() < stall, "Radio audio stalled");
                }
            } else {
                ensure!(
                    self.started.elapsed() < Duration::from_secs(10),
                    "Radio/audio startup deadline exceeded"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn report_buffering(&self) {
        self.status.send_replace(Event {
            generation: self.generation,
            status: Status::Buffering {
                title: self.title.clone(),
            },
            finished: false,
        });
    }
}

pub(super) struct StreamReader {
    pub(super) inner: Mutex<StreamInput>,
}
pub(crate) struct StreamInput {
    pub(crate) receiver: mpsc::Receiver<Vec<u8>>,
    pub(crate) current: std::io::Cursor<Vec<u8>>,
    pub(crate) position: u64,
    pub(crate) cancel: CancellationToken,
}
impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let input = self
            .inner
            .get_mut()
            .map_err(|_| io::Error::other("stream reader poisoned"))?;
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if input.cancel.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "playback cancelled",
                ));
            }
            let n = input.current.read(buf)?;
            if n != 0 {
                input.position = input
                    .position
                    .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
                return Ok(n);
            }
            let Some(bytes) = input.receiver.blocking_recv() else {
                return Ok(0);
            };
            input.current = std::io::Cursor::new(bytes);
        }
    }
}
impl Seek for StreamReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let input = self
            .inner
            .get_mut()
            .map_err(|_| io::Error::other("stream reader poisoned"))?;
        match from {
            SeekFrom::Current(0) => Ok(input.position),
            SeekFrom::Start(n) if n == input.position => Ok(n),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Live stream is not seekable",
            )),
        }
    }
}
#[derive(Default)]
pub(super) struct PlaybackCounters {
    pub(super) consumed: AtomicU64,
    pub(super) underruns: AtomicU64,
    pub(super) finished: AtomicBool,
    pub(super) drained: AtomicBool,
    pub(super) buffering: AtomicBool,
}
pub(super) struct PreparedSource {
    pub(super) pcm: rtrb::Consumer<f32>,
    pub(super) tap: rtrb::Producer<f32>,
    pub(super) controls: Arc<Controls>,
    pub(super) counters: Arc<PlaybackCounters>,
    pub(super) generation: u64,
    pub(super) channels: NonZero<u16>,
    pub(super) rate: NonZero<u32>,
    pub(super) channel: u16,
    pub(super) mix: f32,
    pub(super) eq: Equalizer,
    /// Silent samples left in the current silent frame.
    pub(super) silence: u16,
}
impl PreparedSource {
    /// At a frame start: true plays the next frame, false gives one silent
    /// frame (pause or underrun). At the end of the input the last frame
    /// can be incomplete, and the pop that fails ends playback.
    fn frame_ready(&mut self) -> bool {
        if self.controls.paused.load(Ordering::Relaxed) {
            return false;
        }
        if self.pcm.slots() >= usize::from(self.channels.get())
            || self.counters.finished.load(Ordering::Acquire)
        {
            return true;
        }
        self.counters.underruns.fetch_add(1, Ordering::Relaxed);
        self.counters.buffering.store(true, Ordering::Relaxed);
        false
    }
}
impl Iterator for PreparedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.controls.generation.load(Ordering::Acquire) != self.generation {
            return None;
        }
        if self.silence != 0 {
            self.silence = self.silence.saturating_sub(1);
            return Some(0.0);
        }
        // Silence starts only at a frame start and fills the full frame, so
        // the left and right channels keep their order.
        if self.channel == 0 && !self.frame_ready() {
            self.silence = self.channels.get().saturating_sub(1);
            return Some(0.0);
        }
        // `frame_ready` passed, so the ring is empty only at the end.
        let Ok(raw) = self.pcm.pop() else {
            self.counters.drained.store(true, Ordering::Release);
            return None;
        };
        let sample = self.eq.process(&self.controls.eq, self.channel, raw);
        self.counters.buffering.store(false, Ordering::Relaxed);
        self.counters.consumed.fetch_add(1, Ordering::Relaxed);
        self.mix += sample;
        self.channel = self.channel.saturating_add(1);
        if self.channel == self.channels.get() {
            let _ = self.tap.push(self.mix / f32::from(self.channels.get()));
            self.channel = 0;
            self.mix = 0.0;
        }
        Some(sample * f32::from_bits(self.controls.volume.load(Ordering::Relaxed)))
    }
}
impl Source for PreparedSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        self.channels
    }
    fn sample_rate(&self) -> NonZero<u32> {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
struct OutputStreamInput<'a> {
    device: &'a cpal::Device,
    config: &'a cpal::StreamConfig,
    source: PreparedSource,
    failed: Arc<AtomicBool>,
}

fn output_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    input: OutputStreamInput<'_>,
) -> Result<cpal::Stream> {
    let OutputStreamInput {
        device,
        config,
        mut source,
        failed,
    } = input;
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            for sample in output {
                *sample = T::from_sample(source.next().unwrap_or(0.0));
            }
        },
        move |error| {
            // An underrun is a glitch; the stream continues.
            if !matches!(error, cpal::StreamError::BufferUnderrun) {
                failed.store(true, Ordering::Release);
            }
        },
        None,
    )?)
}
pub(super) struct StartOutputInput<'a> {
    pub(super) device: &'a cpal::Device,
    pub(super) config: &'a cpal::SupportedStreamConfig,
    pub(super) source: PreparedSource,
    pub(super) failed: Arc<AtomicBool>,
}

pub(super) fn start_output(input: StartOutputInput<'_>) -> Result<cpal::Stream> {
    let StartOutputInput {
        device,
        config,
        source,
        failed,
    } = input;
    let settings = config.config();
    macro_rules! build {
        ($sample:ty) => {
            output_stream::<$sample>(OutputStreamInput {
                device,
                config: &settings,
                source,
                failed,
            })?
        };
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::F64 => build!(f64),
        cpal::SampleFormat::I8 => build!(i8),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::I24 => build!(cpal::I24),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::I64 => build!(i64),
        cpal::SampleFormat::U8 => build!(u8),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::U32 => build!(u32),
        cpal::SampleFormat::U64 => build!(u64),
        _ => return Err(Permanent("Unsupported audio output sample format").into()),
    };
    stream.play()?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::super::handle::Handle;
    use super::*;

    fn silent_monitor(stall: Option<Duration>) -> PlaybackMonitor {
        let (handle, _, status, _) = Handle::channels();
        let counters = Arc::new(PlaybackCounters::default());
        counters.buffering.store(true, Ordering::Relaxed);
        PlaybackMonitor {
            started: Instant::now(),
            stall,
            ready: Arc::new(AtomicBool::new(true)),
            counters,
            status,
            controls: Arc::clone(&handle.controls),
            title: String::new(),
            generation: 0,
        }
    }

    #[tokio::test]
    async fn radio_without_audio_fails_for_a_reconnect() {
        let error = silent_monitor(Some(Duration::from_millis(50)))
            .watch()
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Radio audio stalled");
    }

    #[tokio::test]
    async fn paused_radio_and_local_files_have_no_stall_deadline() {
        let paused = silent_monitor(Some(Duration::from_millis(50)));
        paused.controls.paused.store(true, Ordering::Release);
        let file = silent_monitor(None);
        let both = async { tokio::join!(paused.watch(), file.watch()) };
        assert!(
            tokio::time::timeout(Duration::from_millis(300), both)
                .await
                .is_err()
        );
    }
}
