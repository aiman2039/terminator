use super::super::tap;
use super::decode::Permanent;
use super::handle::{CHUNK, Controls, Desired, Event, Status};
use super::pipeline::{PipelineInput, pipeline};
use anyhow::{Context, Result, ensure};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use terminator_core::async_service::{CancellationToken, NativePool, OperationContext, Policy};
use tokio::sync::{mpsc, watch};
#[derive(Clone)]
pub(super) struct Analysis {
    pub(super) service: crate::gui_services::Services,
    pub(super) sender: watch::Sender<Option<[f32; tap::BARS]>>,
    pub(super) controls: Arc<Controls>,
}
impl Analysis {
    pub(super) fn submit(&self, samples: [f32; tap::FFT_N], rate: u32, generation: u64) {
        let context =
            OperationContext::new("audio-spectrum", "player".into(), Policy::ReplaceableRead);
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let cpu = self.service.cpu().clone();
        let sender = self.sender.clone();
        let controls = Arc::clone(&self.controls);
        let _ = self.service.handle().submit(context, cancel, async move {
            let bars = cpu
                .run(&token, move || {
                    anyhow::Ok(tap::bars_from_samples(&samples, rate))
                })
                .await?;
            if controls.generation.load(Ordering::Acquire) == generation {
                sender.send_replace(Some(bars));
            }
            Ok(Vec::new())
        });
    }
}
pub(super) struct PlaybackInput<'a> {
    pub(super) client: &'a reqwest::Client,
    pub(super) decoder: &'a NativePool,
    pub(super) desired: Desired,
    pub(super) controls: Arc<Controls>,
    pub(super) status: watch::Sender<Event>,
    pub(super) spectrum: Analysis,
    pub(super) cancellation: CancellationToken,
}

pub(super) async fn playback(input: PlaybackInput<'_>) -> Result<()> {
    let PlaybackInput {
        client,
        decoder,
        desired,
        controls,
        status,
        spectrum,
        cancellation,
    } = input;
    let source = desired.source.as_ref().context("audio source missing")?;
    let mut retries: u8 = 0;
    loop {
        let admission_started = Instant::now();
        while decoder.occupancy() != 0 || decoder.queued() != 0 {
            ensure!(
                admission_started.elapsed() < Duration::from_secs(10),
                "Audio decoder is still completing an earlier operation"
            );
            tokio::select! { () = cancellation.cancelled() => anyhow::bail!("Playback cancelled"), () = tokio::time::sleep(Duration::from_millis(5)) => {} }
        }
        status.send_replace(Event {
            generation: desired.generation,
            status: Status::Loading {
                title: source.title().into(),
            },
            finished: false,
        });
        let healthy = Arc::new(AtomicBool::new(false));
        let result = pipeline(PipelineInput {
            client,
            pool: decoder,
            desired: desired.clone(),
            controls: Arc::clone(&controls),
            status: status.clone(),
            spectrum: spectrum.clone(),
            cancel: cancellation.child_token(),
            healthy: Arc::clone(&healthy),
        })
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
        let delay = 1_u64.checked_shl(u32::from(retries)).unwrap_or(u64::MAX);
        retries = retries.saturating_add(1);
        status.send_replace(Event {
            generation: desired.generation,
            status: Status::Reconnecting {
                title: source.title().into(),
                attempt: retries,
            },
            finished: false,
        });
        tokio::select! { () = cancellation.cancelled() => anyhow::bail!("Playback cancelled"), () = tokio::time::sleep(Duration::from_secs(delay)) => {} }
    }
}
pub(crate) struct DownloadInput<'a> {
    pub(crate) client: &'a reqwest::Client,
    pub(crate) url: String,
    pub(crate) sender: mpsc::Sender<Vec<u8>>,
    pub(crate) headers: Duration,
    pub(crate) read_timeout: Duration,
}

pub(crate) async fn download(input: DownloadInput<'_>) -> Result<()> {
    let DownloadInput {
        client,
        url,
        sender,
        headers,
        read_timeout,
    } = input;
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
