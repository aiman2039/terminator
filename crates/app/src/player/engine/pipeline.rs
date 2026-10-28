use super::analysis::{Analysis, download};
use super::decode::{DecodeAudioInput, Permanent, decode_audio};
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
    let network = feed_network(FeedInput {
        client,
        source,
        controls: Arc::clone(&controls),
        sender,
    });
    let ready = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(PlaybackCounters::default());
    let startup = StartupMonitor {
        started: Instant::now(),
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
        result = startup.watch() => result,
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

/// Fails playback that never starts, and reports buffering once it has.
struct StartupMonitor {
    started: Instant,
    ready: Arc<AtomicBool>,
    counters: Arc<PlaybackCounters>,
    status: watch::Sender<Event>,
    controls: Arc<Controls>,
    title: String,
    generation: u64,
}

impl StartupMonitor {
    async fn watch(self) -> Result<()> {
        loop {
            if self.ready.load(Ordering::Acquire) {
                self.report_buffering();
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
        if self.counters.buffering.load(Ordering::Relaxed)
            && !self.controls.paused.load(Ordering::Acquire)
        {
            self.status.send_replace(Event {
                generation: self.generation,
                status: Status::Buffering {
                    title: self.title.clone(),
                },
                finished: false,
            });
        }
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
}
impl Iterator for PreparedSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.controls.generation.load(Ordering::Acquire) != self.generation {
            return None;
        }
        if self.controls.paused.load(Ordering::Relaxed) {
            return Some(0.0);
        }
        match self.pcm.pop() {
            Ok(sample) => {
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
            Err(_) if self.counters.finished.load(Ordering::Acquire) => {
                self.counters.drained.store(true, Ordering::Release);
                None
            }
            Err(_) => {
                self.counters.underruns.fetch_add(1, Ordering::Relaxed);
                self.counters.buffering.store(true, Ordering::Relaxed);
                Some(0.0)
            }
        }
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
        move |_| {
            failed.store(true, Ordering::Release);
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
