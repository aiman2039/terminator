use super::super::tap;
use super::analysis::{Analysis, PlaybackInput, playback};
use super::handle::{Controls, Desired, Event, Playable, Status};
use anyhow::Result;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use terminator_core::async_service::{CancellationToken, NativePool};
use tokio::sync::watch;

mod audio;
pub(super) use audio::{DecodeAudioInput, decode_audio};

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
pub(super) struct RunInput {
    pub(super) service: crate::gui_services::Services,
    pub(super) commands: watch::Receiver<Desired>,
    pub(super) controls: Arc<Controls>,
    pub(super) status: watch::Sender<Event>,
    pub(super) spectrum: watch::Sender<Option<[f32; tap::BARS]>>,
    pub(super) lifetime: CancellationToken,
}

pub(super) async fn run(input: RunInput) -> Result<()> {
    let RunInput {
        service,
        mut commands,
        controls,
        status,
        spectrum,
        lifetime,
    } = input;
    let client = http_client(&service, &lifetime).await?;
    let decoder = NativePool::new("audio-decoder", 1)?;
    let runner = Runner {
        service,
        controls,
        status,
        spectrum,
        lifetime,
        client,
        decoder,
    };
    loop {
        let desired = commands.borrow_and_update().clone();
        let Some(source) = desired.source.clone() else {
            runner.report_idle(desired.generation, Status::Stopped);
            if !runner.wait_for_command(&mut commands).await {
                break;
            }
            continue;
        };
        if source.radio() && runner.controls.paused.load(Ordering::Acquire) {
            runner.report_idle(desired.generation, paused_radio(&source));
            if !runner.wait_for_command(&mut commands).await {
                break;
            }
            continue;
        }
        if !runner.play(&desired, &mut commands).await {
            return Ok(());
        }
    }
    Ok(())
}

/// Stream client with dual-stack DNS, built on the catalog pool.
async fn http_client(
    service: &crate::gui_services::Services,
    lifetime: &CancellationToken,
) -> Result<reqwest::Client> {
    let runtime = tokio::runtime::Handle::current();
    service
        .client()
        .catalog
        .run(lifetime, move || {
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
        .await
}

/// A paused radio stream does not hold the connection open.
fn paused_radio(source: &Playable) -> Status {
    Status::Paused {
        title: source.title().into(),
        position: Duration::ZERO,
        duration: None,
        seekable: false,
    }
}

struct Runner {
    service: crate::gui_services::Services,
    controls: Arc<Controls>,
    status: watch::Sender<Event>,
    spectrum: watch::Sender<Option<[f32; tap::BARS]>>,
    lifetime: CancellationToken,
    client: reqwest::Client,
    decoder: NativePool,
}

impl Runner {
    fn report_idle(&self, generation: u64, status: Status) {
        self.status.send_replace(Event {
            generation,
            status,
            finished: false,
        });
        self.spectrum.send_replace(None);
    }

    /// Wait for the next command; false once the engine shuts down.
    async fn wait_for_command(&self, commands: &mut watch::Receiver<Desired>) -> bool {
        tokio::select! {
            () = self.lifetime.cancelled() => false,
            result = commands.changed() => result.is_ok(),
        }
    }

    /// Play `desired` until it ends or a newer command replaces it; false
    /// once the engine shuts down.
    async fn play(&self, desired: &Desired, commands: &mut watch::Receiver<Desired>) -> bool {
        let session = CancellationToken::new();
        let cleanup = session.clone().drop_guard();
        let work = playback(PlaybackInput {
            client: &self.client,
            decoder: &self.decoder,
            desired: desired.clone(),
            controls: Arc::clone(&self.controls),
            status: self.status.clone(),
            spectrum: Analysis {
                service: self.service.clone(),
                sender: self.spectrum.clone(),
                controls: Arc::clone(&self.controls),
            },
            cancellation: session.clone(),
        });
        tokio::pin!(work);
        loop {
            tokio::select! {
                () = self.lifetime.cancelled() => { session.cancel(); return false; }
                result = commands.changed() => {
                    if result.is_err() { return false; }
                    if commands.borrow().generation != desired.generation { session.cancel(); break; }
                }
                result = &mut work => {
                    self.report_finished(desired.generation, result);
                    if !self.wait_for_command(commands).await { return false; }
                    break;
                }
            }
        }
        drop(cleanup);
        true
    }

    fn report_finished(&self, generation: u64, result: Result<()>) {
        if self.controls.generation.load(Ordering::Acquire) != generation {
            return;
        }
        let (next, finished) = match result {
            Ok(()) => (Status::Stopped, true),
            Err(error) => (Status::Error(format!("{error:#}")), false),
        };
        self.status.send_replace(Event {
            generation,
            status: next,
            finished,
        });
    }
}
