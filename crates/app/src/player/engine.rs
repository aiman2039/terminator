//! Async HTTP -> bounded compressed bytes -> native decoder -> prepared PCM.
//! Audio callbacks only consume rings and atomics; Stop invalidates them immediately.
use super::tap;
use anyhow::{Context, Result, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::Source;
use std::{
    cell::RefCell,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use terminator_core::async_service::{CancellationToken, NativePool, OperationContext, Policy};
use tokio::sync::{mpsc, watch};

const CHUNK: usize = 16 * 1024;
const COMPRESSED_SLOTS: usize = 14; // + one producer and one reader chunk <= 256 KiB
const MAX_PCM_SAMPLES: usize = 4 * 1024 * 1024 / std::mem::size_of::<f32>();
#[derive(Clone, Debug)]
pub enum Playable {
    File { path: PathBuf, title: String },
    Stream { url: String, title: String },
}
impl Playable {
    fn title(&self) -> &str {
        match self {
            Self::File { title, .. } | Self::Stream { title, .. } => title,
        }
    }
    fn radio(&self) -> bool {
        matches!(self, Self::Stream { .. })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Stopped,
    Loading {
        title: String,
    },
    Buffering {
        title: String,
    },
    Reconnecting {
        title: String,
        attempt: u8,
    },
    Playing {
        title: String,
        position: Duration,
        duration: Option<Duration>,
        seekable: bool,
    },
    Paused {
        title: String,
        position: Duration,
        duration: Option<Duration>,
        seekable: bool,
    },
    Error(String),
}
#[derive(Clone)]
struct Desired {
    generation: u64,
    source: Option<Playable>,
    offset: Duration,
}
#[derive(Clone, PartialEq)]
struct Event {
    generation: u64,
    status: Status,
    finished: bool,
}
struct Controls {
    generation: AtomicU64,
    paused: AtomicBool,
    volume: AtomicU32,
    decoders: AtomicUsize,
    connections: AtomicUsize,
}
pub struct Handle {
    desired: watch::Sender<Desired>,
    controls: Arc<Controls>,
    events: RefCell<watch::Receiver<Event>>,
    spectrum: watch::Receiver<Option<[f32; tap::BARS]>>,
    lifetime: CancellationToken,
    seen: RefCell<Option<Event>>,
}
pub enum Outcome {
    Status(Status),
    Finished,
}
type Channels = (
    Handle,
    watch::Receiver<Desired>,
    watch::Sender<Event>,
    watch::Sender<Option<[f32; tap::BARS]>>,
);
impl Handle {
    pub fn spawn(service: crate::gui_services::Services) -> Self {
        let (handle, commands, status, spectrum) = Self::channels();
        let controls = handle.controls.clone();
        let lifetime = handle.lifetime.clone();
        let mut context = OperationContext::new("audio", "player".into(), Policy::ServiceLifetime);
        context.deadline = None;
        let services = service.clone();
        let errors = status.clone();
        let error_controls = controls.clone();
        if let Err(error) = service
            .handle()
            .submit(context, lifetime.clone(), async move {
                if let Err(error) = run(
                    services,
                    commands,
                    controls,
                    status.clone(),
                    spectrum,
                    lifetime,
                )
                .await
                {
                    let generation = error_controls.generation.load(Ordering::Acquire);
                    status.send_replace(Event {
                        generation,
                        status: Status::Error(format!("{error:#}")),
                        finished: false,
                    });
                }
                Ok(Vec::new())
            })
        {
            errors.send_replace(Event {
                generation: 0,
                status: Status::Error(error.to_string()),
                finished: false,
            });
        }
        handle
    }
    fn channels() -> Channels {
        let (desired, commands) = watch::channel(Desired {
            generation: 0,
            source: None,
            offset: Duration::ZERO,
        });
        let (status, events) = watch::channel(Event {
            generation: 0,
            status: Status::Stopped,
            finished: false,
        });
        let (spectrum_tx, spectrum) = watch::channel(None);
        (
            Self {
                desired,
                controls: Arc::new(Controls {
                    generation: AtomicU64::new(0),
                    paused: AtomicBool::new(false),
                    volume: AtomicU32::new(1.0_f32.to_bits()),
                    decoders: AtomicUsize::new(0),
                    connections: AtomicUsize::new(0),
                }),
                events: RefCell::new(events),
                spectrum,
                lifetime: CancellationToken::new(),
                seen: RefCell::new(None),
            },
            commands,
            status,
            spectrum_tx,
        )
    }
    #[cfg(test)]
    pub(super) fn finished_fixture() -> Self {
        let (handle, _, status, _) = Self::channels();
        status.send_replace(Event {
            generation: 0,
            status: Status::Stopped,
            finished: true,
        });
        handle
    }
    #[cfg(feature = "test-support")]
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"generation":self.controls.generation.load(Ordering::Acquire), "decoders":self.controls.decoders.load(Ordering::Acquire), "connections":self.controls.connections.load(Ordering::Acquire)})
    }
    pub fn play(&self, source: Playable) {
        self.controls.paused.store(false, Ordering::Release);
        let generation = self.controls.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.desired.send_replace(Desired {
            generation,
            source: Some(source),
            offset: Duration::ZERO,
        });
    }
    pub fn stop(&self) {
        let generation = self.controls.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.desired.send_replace(Desired {
            generation,
            source: None,
            offset: Duration::ZERO,
        });
    }
    pub fn pause(&self) {
        self.controls.paused.store(true, Ordering::Release);
        self.desired.send_modify(|desired| {
            if desired.source.as_ref().is_some_and(Playable::radio) {
                desired.generation = self.controls.generation.fetch_add(1, Ordering::AcqRel) + 1;
            }
        });
    }
    pub fn resume(&self) {
        self.controls.paused.store(false, Ordering::Release);
        self.desired.send_modify(|desired| {
            if desired.source.as_ref().is_some_and(Playable::radio) {
                desired.generation = self.controls.generation.fetch_add(1, Ordering::AcqRel) + 1;
            }
        });
    }
    pub fn seek(&self, position: Duration) {
        self.desired.send_modify(|desired| {
            if matches!(desired.source, Some(Playable::File { .. })) {
                desired.offset = position;
                desired.generation = self.controls.generation.fetch_add(1, Ordering::AcqRel) + 1;
            }
        });
    }
    pub fn volume(&self, volume: f32) {
        self.controls
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }
    pub fn spectrum_bars(&self) -> Option<[f32; tap::BARS]> {
        if self.events.borrow().borrow().generation
            != self.controls.generation.load(Ordering::Acquire)
        {
            return None;
        }
        *self.spectrum.borrow()
    }
    pub fn poll(&self) -> Vec<Outcome> {
        let mut receiver = self.events.borrow_mut();
        // A closed watch still retains its last unseen value (including errors).
        if !receiver.has_changed().unwrap_or(true) {
            return Vec::new();
        }
        let mut event = receiver.borrow_and_update().clone();
        let paused = self.controls.paused.load(Ordering::Acquire);
        if paused {
            match event.status {
                Status::Playing {
                    title,
                    position,
                    duration,
                    seekable,
                } => {
                    event.status = Status::Paused {
                        title,
                        position,
                        duration,
                        seekable,
                    }
                }
                Status::Loading { .. } | Status::Buffering { .. } | Status::Reconnecting { .. } => {
                    return Vec::new();
                }
                _ => {}
            }
        } else if matches!(event.status, Status::Paused { .. }) {
            return Vec::new();
        }
        if event.generation != self.controls.generation.load(Ordering::Acquire)
            || self.seen.borrow().as_ref() == Some(&event)
        {
            return Vec::new();
        }
        self.seen.replace(Some(event.clone()));
        let mut result = vec![Outcome::Status(event.status)];
        if event.finished {
            result.push(Outcome::Finished);
        }
        result
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
        self.lifetime.cancel();
    }
}

#[derive(Debug)]
struct Permanent(&'static str);
impl std::fmt::Display for Permanent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Permanent {}
struct Resolver(hickory_resolver::TokioResolver);
impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = self.0.clone();
        Box::pin(async move {
            let lookup = resolver.lookup_ip(name.as_str()).await?;
            let addresses: reqwest::dns::Addrs = Box::new(
                lookup
                    .into_iter()
                    .map(|ip| std::net::SocketAddr::new(ip, 0)),
            );
            Ok(addresses)
        })
    }
}
async fn run(
    service: crate::gui_services::Services,
    mut commands: watch::Receiver<Desired>,
    controls: Arc<Controls>,
    status: watch::Sender<Event>,
    spectrum: watch::Sender<Option<[f32; tap::BARS]>>,
    lifetime: CancellationToken,
) -> Result<()> {
    let runtime = tokio::runtime::Handle::current();
    let client = service
        .client()
        .catalog
        .run(&lifetime, move || {
            let _entered = runtime.enter();
            let mut resolver = hickory_resolver::TokioResolver::builder_tokio()?;
            resolver.options_mut().ip_strategy =
                hickory_resolver::config::LookupIpStrategy::Ipv4AndIpv6;
            Ok(reqwest::Client::builder()
                .pool_max_idle_per_host(0)
                .dns_resolver(Arc::new(Resolver(resolver.build()?)))
                .connect_timeout(Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::limited(8))
                .build()?)
        })
        .await?;
    let decoder = NativePool::new("audio-decoder", 1)?;
    loop {
        let desired = commands.borrow_and_update().clone();
        let Some(source) = desired.source.clone() else {
            status.send_replace(Event {
                generation: desired.generation,
                status: Status::Stopped,
                finished: false,
            });
            spectrum.send_replace(None);
            tokio::select! { _ = lifetime.cancelled() => break, result = commands.changed() => if result.is_err() { break } }
            continue;
        };
        if source.radio() && controls.paused.load(Ordering::Acquire) {
            status.send_replace(Event {
                generation: desired.generation,
                status: Status::Paused {
                    title: source.title().into(),
                    position: Duration::ZERO,
                    duration: None,
                    seekable: false,
                },
                finished: false,
            });
            spectrum.send_replace(None);
            tokio::select! { _ = lifetime.cancelled() => break, result = commands.changed() => if result.is_err() { break } }
            continue;
        }
        let session = CancellationToken::new();
        let cleanup = session.clone().drop_guard();
        let work = playback(
            &client,
            &decoder,
            desired.clone(),
            controls.clone(),
            status.clone(),
            Analysis {
                service: service.clone(),
                sender: spectrum.clone(),
                controls: controls.clone(),
            },
            session.clone(),
        );
        tokio::pin!(work);
        loop {
            tokio::select! {
                _ = lifetime.cancelled() => { session.cancel(); return Ok(()); }
                result = commands.changed() => {
                    if result.is_err() { return Ok(()); }
                    if commands.borrow().generation != desired.generation { session.cancel(); break; }
                }
                result = &mut work => {
                    if controls.generation.load(Ordering::Acquire) == desired.generation {
                        let (next, finished) = match result {
                            Ok(()) => (Status::Stopped, true),
                            Err(error) => (Status::Error(format!("{error:#}")), false),
                        };
                        status.send_replace(Event { generation: desired.generation, status: next, finished });
                    }
                    tokio::select! { _ = lifetime.cancelled() => return Ok(()), result = commands.changed() => if result.is_err() { return Ok(()); } }
                    break;
                }
            }
        }
        drop(cleanup);
    }
    Ok(())
}
#[derive(Clone)]
struct Analysis {
    service: crate::gui_services::Services,
    sender: watch::Sender<Option<[f32; tap::BARS]>>,
    controls: Arc<Controls>,
}
impl Analysis {
    fn submit(&self, samples: [f32; tap::FFT_N], rate: u32, generation: u64) {
        let context =
            OperationContext::new("audio-spectrum", "player".into(), Policy::ReplaceableRead);
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let cpu = self.service.cpu().clone();
        let sender = self.sender.clone();
        let controls = self.controls.clone();
        let _ = self.service.handle().submit(context, cancel, async move {
            let bars = cpu
                .run(&token, move || Ok(tap::bars_from_samples(&samples, rate)))
                .await?;
            if controls.generation.load(Ordering::Acquire) == generation {
                sender.send_replace(Some(bars));
            }
            Ok(Vec::new())
        });
    }
}
async fn playback(
    client: &reqwest::Client,
    decoder: &NativePool,
    desired: Desired,
    controls: Arc<Controls>,
    status: watch::Sender<Event>,
    spectrum: Analysis,
    cancellation: CancellationToken,
) -> Result<()> {
    let source = desired.source.as_ref().unwrap();
    let mut retries = 0;
    loop {
        let admission_started = Instant::now();
        while decoder.occupancy() != 0 || decoder.queued() != 0 {
            ensure!(
                admission_started.elapsed() < Duration::from_secs(10),
                "Audio decoder is still completing an earlier operation"
            );
            tokio::select! { _ = cancellation.cancelled() => anyhow::bail!("Playback cancelled"), _ = tokio::time::sleep(Duration::from_millis(5)) => {} }
        }
        status.send_replace(Event {
            generation: desired.generation,
            status: Status::Loading {
                title: source.title().into(),
            },
            finished: false,
        });
        let healthy = Arc::new(AtomicBool::new(false));
        let result = pipeline(
            client,
            decoder,
            desired.clone(),
            controls.clone(),
            status.clone(),
            spectrum.clone(),
            (cancellation.child_token(), healthy.clone()),
        )
        .await;
        if !source.radio() {
            return result;
        }
        let error = result
            .err()
            .unwrap_or_else(|| anyhow::anyhow!("Radio stream ended unexpectedly"));
        if error.downcast_ref::<Permanent>().is_some() {
            return Err(error);
        }
        if healthy.load(Ordering::Acquire) {
            retries = 0;
        }
        if retries == 3 {
            return Err(error.context("Radio retry limit reached"));
        }
        let delay = 1 << retries;
        retries += 1;
        status.send_replace(Event {
            generation: desired.generation,
            status: Status::Reconnecting {
                title: source.title().into(),
                attempt: retries,
            },
            finished: false,
        });
        tokio::select! { _ = cancellation.cancelled() => anyhow::bail!("Playback cancelled"), _ = tokio::time::sleep(Duration::from_secs(delay)) => {} }
    }
}
pub(super) async fn download(
    client: &reqwest::Client,
    url: String,
    sender: mpsc::Sender<Vec<u8>>,
    headers: Duration,
    read_timeout: Duration,
) -> Result<()> {
    let mut response = tokio::time::timeout(headers, client.get(url).send())
        .await
        .context("Radio connection/header deadline exceeded")?
        .map_err(reqwest::Error::without_url)?;
    let code = response.status();
    if !code.is_success() {
        if code.is_client_error() && code.as_u16() != 408 && code.as_u16() != 429 {
            return Err(Permanent("Radio server rejected this station").into());
        }
        anyhow::bail!("Radio server temporarily unavailable ({code})");
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.contains("html") || content_type.contains("json") {
        return Err(Permanent("Station response is not audio").into());
    }
    loop {
        let chunk = tokio::time::timeout(read_timeout, response.chunk())
            .await
            .context("Radio network read stalled")?
            .map_err(reqwest::Error::without_url)?;
        let Some(chunk) = chunk else { break };
        for bytes in chunk.chunks(CHUNK) {
            if sender.send(bytes.to_vec()).await.is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}
struct PipelineGuard {
    controls: Arc<Controls>,
    network: bool,
}
impl PipelineGuard {
    fn new(controls: Arc<Controls>, network: bool) -> Self {
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
async fn pipeline(
    client: &reqwest::Client,
    pool: &NativePool,
    desired: Desired,
    controls: Arc<Controls>,
    status: watch::Sender<Event>,
    spectrum: Analysis,
    (cancel, healthy): (CancellationToken, Arc<AtomicBool>),
) -> Result<()> {
    let _cancel_on_drop = cancel.clone().drop_guard();
    let (sender, receiver) = mpsc::channel(COMPRESSED_SLOTS);
    let source = desired.source.as_ref().unwrap().clone();
    let network_controls = controls.clone();
    let network = async {
        if let Playable::Stream { url, .. } = source {
            let _network = PipelineGuard::new(network_controls, true);
            download(
                client,
                url,
                sender,
                Duration::from_secs(8),
                Duration::from_secs(10),
            )
            .await
        } else {
            drop(sender);
            Ok(())
        }
    };
    let ready = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let monitor_ready = ready.clone();
    let counters = Arc::new(PlaybackCounters::default());
    let monitored = counters.clone();
    let monitor_status = status.clone();
    let monitor_controls = controls.clone();
    let monitor_title = desired.source.as_ref().unwrap().title().to_owned();
    let monitor_generation = desired.generation;
    let decode_cancel = cancel.clone();
    let decode = pool.run(&cancel, move || {
        decode_audio(
            desired,
            controls,
            status,
            spectrum,
            receiver,
            decode_cancel,
            (ready, counters, healthy),
        )
    });
    let startup = async move {
        loop {
            if monitor_ready.load(Ordering::Acquire) {
                if monitored.buffering.load(Ordering::Relaxed)
                    && !monitor_controls.paused.load(Ordering::Acquire)
                {
                    monitor_status.send_replace(Event {
                        generation: monitor_generation,
                        status: Status::Buffering {
                            title: monitor_title.clone(),
                        },
                        finished: false,
                    });
                }
            } else {
                ensure!(
                    started.elapsed() < Duration::from_secs(10),
                    "Radio/audio startup deadline exceeded"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        result = async { tokio::try_join!(network, decode)?; Ok(()) } => result,
        result = startup => result,
    }
}

struct StreamReader {
    inner: Mutex<StreamInput>,
}
struct StreamInput {
    receiver: mpsc::Receiver<Vec<u8>>,
    current: std::io::Cursor<Vec<u8>>,
    position: u64,
    cancel: CancellationToken,
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
                input.position += n as u64;
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
struct PlaybackCounters {
    consumed: AtomicU64,
    underruns: AtomicU64,
    finished: AtomicBool,
    drained: AtomicBool,
    buffering: AtomicBool,
}
struct PreparedSource {
    pcm: rtrb::Consumer<f32>,
    tap: rtrb::Producer<f32>,
    controls: Arc<Controls>,
    counters: Arc<PlaybackCounters>,
    generation: u64,
    channels: u16,
    rate: u32,
    channel: u16,
    mix: f32,
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
                self.channel += 1;
                if self.channel == self.channels {
                    let _ = self.tap.push(self.mix / f32::from(self.channels));
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
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
fn output_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut source: PreparedSource,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
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
fn start_output(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    source: PreparedSource,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream> {
    let settings = config.config();
    macro_rules! build {
        ($sample:ty) => {
            output_stream::<$sample>(device, &settings, source, failed)?
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
fn decode_audio(
    desired: Desired,
    controls: Arc<Controls>,
    status: watch::Sender<Event>,
    spectrum: Analysis,
    receiver: mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
    (ready, counters, healthy): (Arc<AtomicBool>, Arc<PlaybackCounters>, Arc<AtomicBool>),
) -> Result<()> {
    let _decoder = PipelineGuard::new(controls.clone(), false);
    let source = desired.source.as_ref().unwrap();
    let mut decoder: Box<dyn Source<Item = f32> + Send> = match source {
        Playable::File { path, .. } => {
            use std::os::unix::fs::OpenOptionsExt;
            let file = File::options()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(path)
                .context("Open audio file")?;
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
    };
    if !desired.offset.is_zero() {
        decoder
            .try_seek(desired.offset)
            .map_err(|e| anyhow::anyhow!("Seek failed: {e}"))?;
    }
    let duration = decoder.total_duration();
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(Permanent("No audio output device"))?;
    let configuration = device
        .default_output_config()
        .map_err(|_| Permanent("Audio output device configuration is unavailable"))?;
    let channels = configuration.channels();
    let rate = configuration.sample_rate().0;
    // Resampling/channel conversion stays on this native decoder worker.
    let mut decoder = rodio::source::UniformSourceIterator::new(decoder, channels, rate);
    let output_failed = Arc::new(AtomicBool::new(false));
    ensure!(channels != 0 && rate != 0, "Invalid audio sample format");
    let capacity = (rate as usize * channels as usize * 2).min(MAX_PCM_SAMPLES);
    let threshold = (rate as usize * channels as usize / 4).min(capacity);
    let (mut pcm, consumer) = rtrb::RingBuffer::new(capacity);
    let (tap_producer, mut tap_consumer) = rtrb::RingBuffer::new(tap::FFT_N * 2);
    let mut prepared = Some(PreparedSource {
        pcm: consumer,
        tap: tap_producer,
        controls: controls.clone(),
        counters: counters.clone(),
        generation: desired.generation,
        channels,
        rate,
        channel: 0,
        mix: 0.0,
    });
    let mut output = None;
    let mut produced = 0;
    let mut next = None;
    let mut eof = false;
    let mut tick = Instant::now() - Duration::from_millis(100);
    let mut samples = [0.0; tap::FFT_N];
    let mut sample_index = 0;
    let mut sampled = 0;
    loop {
        if output_failed.load(Ordering::Acquire) {
            return Err(Permanent("Audio output device disconnected").into());
        }
        if cancel.is_cancelled()
            || controls.generation.load(Ordering::Acquire) != desired.generation
        {
            break;
        }
        if !eof && next.is_none() {
            next = decoder.next();
            if next.is_none() {
                eof = true;
                counters.finished.store(true, Ordering::Release);
            }
        }
        if let Some(sample) = next
            && pcm.push(sample).is_ok()
        {
            next = None;
            produced += 1;
        }
        if output.is_none() && (produced >= threshold || eof) {
            let stream = start_output(
                &device,
                &configuration,
                prepared.take().unwrap(),
                output_failed.clone(),
            )?;
            output = Some(stream);
            ready.store(true, Ordering::Release);
        }
        if tick.elapsed() >= Duration::from_millis(50) {
            tick = Instant::now();
            while let Ok(sample) = tap_consumer.pop() {
                samples[sample_index] = sample;
                sample_index = (sample_index + 1) % tap::FFT_N;
                sampled += 1;
            }
            if sampled >= tap::FFT_N {
                let mut window = [0.0; tap::FFT_N];
                for (i, sample) in window.iter_mut().enumerate() {
                    *sample = samples[(sample_index + i) % tap::FFT_N];
                }
                spectrum.submit(window, rate, desired.generation);
            }
            if ready.load(Ordering::Acquire) {
                if counters.consumed.load(Ordering::Relaxed)
                    >= u64::from(rate) * u64::from(channels) * 60
                {
                    healthy.store(true, Ordering::Release);
                }
                let position = desired.offset
                    + Duration::from_secs_f64(
                        counters.consumed.load(Ordering::Relaxed) as f64
                            / f64::from(rate)
                            / f64::from(channels),
                    );
                let title = source.title().to_owned();
                let next = if controls.paused.load(Ordering::Acquire) {
                    Status::Paused {
                        title,
                        position,
                        duration,
                        seekable: !source.radio(),
                    }
                } else if counters.buffering.load(Ordering::Relaxed) {
                    Status::Buffering { title }
                } else {
                    Status::Playing {
                        title,
                        position,
                        duration,
                        seekable: !source.radio(),
                    }
                };
                status.send_replace(Event {
                    generation: desired.generation,
                    status: next,
                    finished: false,
                });
            }
        }
        if eof && counters.drained.load(Ordering::Acquire) {
            break;
        }
        if next.is_some() || eof {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    drop(output);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared(capacity: usize) -> (rtrb::Producer<f32>, PreparedSource) {
        let (pcm, consumer) = rtrb::RingBuffer::new(capacity);
        let (tap, _consumer) = rtrb::RingBuffer::new(tap::FFT_N);
        (
            pcm,
            PreparedSource {
                pcm: consumer,
                tap,
                controls: Arc::new(Controls {
                    generation: AtomicU64::new(1),
                    paused: AtomicBool::new(false),
                    volume: AtomicU32::new(1.0_f32.to_bits()),
                    decoders: AtomicUsize::new(0),
                    connections: AtomicUsize::new(0),
                }),
                counters: Arc::default(),
                generation: 1,
                channels: 1,
                rate: 8000,
                channel: 0,
                mix: 0.0,
            },
        )
    }
    #[test]
    fn pending_progress_cannot_undo_local_pause_or_resume() {
        let (handle, _commands, status, _) = Handle::channels();
        handle.play(Playable::File {
            path: "fixture.wav".into(),
            title: "Fixture".into(),
        });
        handle.pause();
        status.send_replace(Event {
            generation: 1,
            status: Status::Playing {
                title: "Fixture".into(),
                position: Duration::from_secs(1),
                duration: None,
                seekable: true,
            },
            finished: false,
        });
        assert!(matches!(
            handle.poll().as_slice(),
            [Outcome::Status(Status::Paused { .. })]
        ));
        handle.resume();
        status.send_replace(Event {
            generation: 1,
            status: Status::Paused {
                title: "Fixture".into(),
                position: Duration::from_secs(1),
                duration: None,
                seekable: true,
            },
            finished: false,
        });
        assert!(handle.poll().is_empty());
    }

    #[test]
    #[ignore = "real output-device local audio lifecycle check; muted"]
    fn local_audio_pause_seek_resume_and_natural_completion() {
        let directory = tempfile::Builder::new()
            .prefix("audio-local-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = directory.path().join("silence.wav");
        let data_len = 8000_u32 * 3 * 2;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data_len).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(8000_u32.to_le_bytes());
        wav.extend(16000_u32.to_le_bytes());
        wav.extend(2_u16.to_le_bytes());
        wav.extend(16_u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        std::fs::write(&path, wav).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let (service, mut owner) = crate::gui_services::Services::new(
            terminator_core::Paths::at(directory.path().into()),
            eframe::egui::Context::default(),
            tx,
        )
        .unwrap();
        let handle = Handle::spawn(service);
        handle.volume(0.0);
        handle.play(Playable::File {
            path,
            title: "Local fixture".into(),
        });
        let mut wait = |predicate: &dyn Fn(&Outcome) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(6);
            loop {
                while owner.supervisor.try_recv().is_some() {}
                for outcome in handle.poll() {
                    if let Outcome::Status(Status::Error(error)) = &outcome {
                        panic!("Local audio fixture: {error}");
                    }
                    if predicate(&outcome) {
                        return;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "Local audio lifecycle did not progress"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait(
            &|event| matches!(event, Outcome::Status(Status::Playing { position, .. }) if *position >= Duration::from_millis(50)),
        );
        handle.pause();
        wait(&|event| matches!(event, Outcome::Status(Status::Paused { .. })));
        handle.seek(Duration::from_secs(1));
        wait(
            &|event| matches!(event, Outcome::Status(Status::Paused { position, .. }) if *position == Duration::from_secs(1)),
        );
        handle.resume();
        wait(
            &|event| matches!(event, Outcome::Status(Status::Playing { position, .. }) if *position > Duration::from_secs(1)),
        );
        wait(&|event| matches!(event, Outcome::Finished));
        assert_eq!(handle.controls.decoders.load(Ordering::Acquire), 0);
    }

    #[test]
    #[ignore = "bounded live 103FM network and real output-device check; muted"]
    fn live_103fm_reaches_pcm_and_stop_releases_pipeline() {
        let directory = tempfile::Builder::new()
            .prefix("radio-live-")
            .tempdir_in("/tmp")
            .unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let (service, mut owner) = crate::gui_services::Services::new(
            terminator_core::Paths::at(directory.path().into()),
            eframe::egui::Context::default(),
            tx,
        )
        .unwrap();
        let handle = Handle::spawn(service);
        handle.volume(0.0);
        let start = Instant::now();
        handle.play(Playable::Stream {
            title: "103FM live fixture".into(),
            url: "https://cdn.cybercdn.live/103FM/Live/icecast.audio".into(),
        });
        let mut first_pcm = None;
        while start.elapsed() < Duration::from_secs(20) {
            while owner.supervisor.try_recv().is_some() {}
            for outcome in handle.poll() {
                match outcome {
                    Outcome::Status(Status::Playing { position, .. })
                        if position >= Duration::from_millis(250) =>
                    {
                        first_pcm = Some(start.elapsed())
                    }
                    Outcome::Status(Status::Error(error)) => {
                        handle.stop();
                        panic!("103FM live fixture: {error}");
                    }
                    _ => {}
                }
            }
            if first_pcm.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let stop = Instant::now();
        handle.stop();
        while handle.controls.decoders.load(Ordering::Acquire) != 0
            || handle.controls.connections.load(Ordering::Acquire) != 0
        {
            assert!(
                stop.elapsed() < Duration::from_secs(2),
                "Stopped live pipeline remained active"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            first_pcm.is_some(),
            "103FM did not produce PCM within 20 seconds"
        );
        println!(
            "{}",
            serde_json::json!({"station":"103FM","pcm_ms":first_pcm.unwrap().as_secs_f64()*1000.0,"cleanup_ms":stop.elapsed().as_secs_f64()*1000.0,"output_device":true,"muted":true})
        );
    }

    #[test]
    fn callback_never_waits_and_stop_invalidates_already_buffered_pcm() {
        let (mut producer, mut callback) = prepared(2);
        let started = Instant::now();
        for _ in 0..10000 {
            assert_eq!(callback.next(), Some(0.0));
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        producer.push(0.75).unwrap();
        producer.push(0.5).unwrap();
        assert!(producer.push(1.0).is_err());
        callback.controls.generation.store(2, Ordering::Release);
        assert_eq!(callback.next(), None);
    }
    #[test]
    fn local_pause_keeps_samples_and_saved_mute_applies_before_first_sample() {
        let (mut producer, mut callback) = prepared(4);
        producer.push(0.5).unwrap();
        callback.controls.paused.store(true, Ordering::Release);
        assert_eq!(callback.next(), Some(0.0));
        callback.controls.paused.store(false, Ordering::Release);
        callback
            .controls
            .volume
            .store(0.0_f32.to_bits(), Ordering::Release);
        assert_eq!(callback.next(), Some(0.0));
        assert_eq!(callback.counters.consumed.load(Ordering::Acquire), 1);
        callback.counters.finished.store(true, Ordering::Release);
        assert_eq!(callback.next(), None);
        assert!(callback.counters.drained.load(Ordering::Acquire));
    }
    #[test]
    fn rapid_switching_keeps_only_latest_request_and_old_finish_cannot_advance_it() {
        let handle = Handle::finished_fixture();
        for n in 0..100 {
            handle.play(Playable::Stream {
                url: format!("http://127.0.0.1/{n}"),
                title: n.to_string(),
            });
        }
        assert_eq!(handle.desired.borrow().generation, 100);
        assert_eq!(handle.controls.generation.load(Ordering::Acquire), 100);
        assert!(handle.poll().is_empty());
        handle.pause();
        assert_eq!(handle.controls.generation.load(Ordering::Acquire), 101);
        handle.resume();
        assert_eq!(handle.controls.generation.load(Ordering::Acquire), 102);
        handle.stop();
        assert!(handle.desired.borrow().source.is_none());
    }
    async fn server(
        header_delay: Duration,
        gaps: Vec<Duration>,
        header: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(header_delay).await;
            if socket.write_all(header).await.is_err() {
                return;
            }
            for gap in gaps {
                tokio::time::sleep(gap).await;
                if socket.write_all(b"x").await.is_err() {
                    break;
                }
            }
        });
        (format!("http://{address}"), task)
    }
    #[tokio::test]
    async fn slow_headers_and_stalled_reads_have_separate_deadlines() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let headers = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n";
        for (header_delay, gaps, expected) in [
            (Duration::from_millis(100), vec![], "header"),
            (Duration::ZERO, vec![Duration::from_millis(100)], "stalled"),
        ] {
            let (url, server) = server(header_delay, gaps, headers).await;
            let (tx, _rx) = mpsc::channel(COMPRESSED_SLOTS);
            let error = download(
                &client,
                url,
                tx,
                Duration::from_millis(30),
                Duration::from_millis(30),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn fragmented_healthy_radio_has_no_total_lifetime_deadline() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (url, server) = server(
            Duration::ZERO,
            vec![Duration::from_millis(35); 4],
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n",
        )
        .await;
        let (tx, mut rx) = mpsc::channel(COMPRESSED_SLOTS);
        download(
            &client,
            url,
            tx,
            Duration::from_millis(80),
            Duration::from_millis(80),
        )
        .await
        .unwrap();
        let mut bytes = Vec::new();
        while let Some(chunk) = rx.recv().await {
            bytes.extend(chunk);
        }
        assert_eq!(bytes, b"xxxx");
        server.await.unwrap();
    }
    #[tokio::test]
    async fn permanent_http_error_is_not_a_retryable_station_failure() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (url, server) = server(
            Duration::ZERO,
            vec![],
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        let (tx, _rx) = mpsc::channel(COMPRESSED_SLOTS);
        assert!(
            download(
                &client,
                url,
                tx,
                Duration::from_secs(1),
                Duration::from_secs(1)
            )
            .await
            .unwrap_err()
            .downcast_ref::<Permanent>()
            .is_some()
        );
        server.await.unwrap();
    }
}
