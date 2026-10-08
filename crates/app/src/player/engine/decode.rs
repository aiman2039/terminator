use super::super::tap;
use super::analysis::{Analysis, playback};
use super::handle::{Controls, Desired, Event, MAX_PCM_SAMPLES, Playable, Status};
use super::pipeline::{
    PipelineGuard, PlaybackCounters, PreparedSource, StreamInput, StreamReader, start_output,
};
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
use terminator_core::async_service::{CancellationToken, NativePool};
use tokio::sync::{mpsc, watch};
#[derive(Debug)]
pub(super) struct Permanent(pub(super) &'static str);
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
pub(super) async fn run(
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
            tokio::select! { () = lifetime.cancelled() => break, result = commands.changed() => if result.is_err() { break } }
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
            tokio::select! { () = lifetime.cancelled() => break, result = commands.changed() => if result.is_err() { break } }
            continue;
        }
        let session = CancellationToken::new();
        let cleanup = session.clone().drop_guard();
        let work = playback(
            &client,
            &decoder,
            desired.clone(),
            Arc::clone(&controls),
            status.clone(),
            Analysis {
                service: service.clone(),
                sender: spectrum.clone(),
                controls: Arc::clone(&controls),
            },
            session.clone(),
        );
        tokio::pin!(work);
        loop {
            tokio::select! {
                () = lifetime.cancelled() => { session.cancel(); return Ok(()); }
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
                    tokio::select! { () = lifetime.cancelled() => return Ok(()), result = commands.changed() => if result.is_err() { return Ok(()); } }
                    break;
                }
            }
        }
        drop(cleanup);
    }
    Ok(())
}
pub(super) fn decode_audio(
    desired: Desired,
    controls: Arc<Controls>,
    status: watch::Sender<Event>,
    spectrum: Analysis,
    receiver: mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
    (ready, counters, healthy): (Arc<AtomicBool>, Arc<PlaybackCounters>, Arc<AtomicBool>),
) -> Result<()> {
    let _decoder = PipelineGuard::new(Arc::clone(&controls), false);
    let source = desired.source.as_ref().context("audio source missing")?;
    let mut decoder: Box<dyn Source<Item = f32> + Send> = match source {
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
    let channels =
        NonZero::new(configuration.channels()).ok_or(Permanent("Invalid audio sample format"))?;
    let rate = NonZero::new(configuration.sample_rate())
        .ok_or(Permanent("Invalid audio sample format"))?;
    // Resampling/channel conversion stays on this native decoder worker.
    let mut decoder = rodio::source::UniformSourceIterator::new(decoder, channels, rate);
    let output_failed = Arc::new(AtomicBool::new(false));
    let rate_samples = usize::try_from(rate.get()).unwrap_or(usize::MAX);
    let channel_count = usize::from(channels.get());
    let capacity = rate_samples
        .saturating_mul(channel_count)
        .saturating_mul(2)
        .min(MAX_PCM_SAMPLES);
    let threshold = rate_samples
        .saturating_mul(channel_count)
        .checked_div(4)
        .unwrap_or(0)
        .min(capacity);
    let (mut pcm, consumer) = rtrb::RingBuffer::new(capacity);
    let (tap_producer, mut tap_consumer) = rtrb::RingBuffer::new(tap::FFT_N * 2);
    let mut prepared = Some(PreparedSource {
        pcm: consumer,
        tap: tap_producer,
        controls: Arc::clone(&controls),
        counters: Arc::clone(&counters),
        generation: desired.generation,
        channels,
        rate,
        channel: 0,
        mix: 0.0,
    });
    let mut output = None;
    let mut produced: usize = 0;
    let mut next = None;
    let mut eof = false;
    let mut tick = Instant::now()
        .checked_sub(Duration::from_millis(100))
        .unwrap_or_else(Instant::now);
    let mut samples = [0.0; tap::FFT_N];
    let mut sample_index = 0;
    let mut sampled: usize = 0;
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
            produced = produced.saturating_add(1);
        }
        if output.is_none() && (produced >= threshold || eof) {
            let stream = start_output(
                &device,
                &configuration,
                prepared.take().context("audio output was not prepared")?,
                Arc::clone(&output_failed),
            )?;
            output = Some(stream);
            ready.store(true, Ordering::Release);
        }
        if tick.elapsed() >= Duration::from_millis(50) {
            tick = Instant::now();
            while let Ok(sample) = tap_consumer.pop() {
                if let Some(slot) = samples.get_mut(sample_index) {
                    *slot = sample;
                }
                sample_index = sample_index
                    .checked_add(1)
                    .and_then(|index| index.checked_rem(tap::FFT_N))
                    .unwrap_or(0);
                sampled = sampled.saturating_add(1);
            }
            if sampled >= tap::FFT_N {
                let mut window = [0.0; tap::FFT_N];
                for (i, sample) in window.iter_mut().enumerate() {
                    if let Some(value) = sample_index
                        .checked_add(i)
                        .and_then(|index| index.checked_rem(tap::FFT_N))
                        .and_then(|index| samples.get(index))
                    {
                        *sample = *value;
                    }
                }
                spectrum.submit(window, rate.get(), desired.generation);
            }
            if ready.load(Ordering::Acquire) {
                let played = counters.consumed.load(Ordering::Relaxed);
                let frame = u64::from(rate.get()).saturating_mul(u64::from(channels.get()));
                if played >= frame.saturating_mul(60) {
                    healthy.store(true, Ordering::Release);
                }
                // Playback position. Sample counts above 2^53 do not fit in an f64 mantissa.
                #[allow(clippy::cast_precision_loss)]
                let seconds = played as f64 / f64::from(rate.get()) / f64::from(channels.get());
                let Ok(elapsed) = Duration::try_from_secs_f64(seconds) else {
                    continue;
                };
                let Some(position) = desired.offset.checked_add(elapsed) else {
                    continue;
                };
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
